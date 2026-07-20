//! Best-effort fleet failure telemetry JSONL writer.
//!
//! This is intentionally small and local-only: failures append to
//! `$ZEUS_HOME/logs/fleet-failures.jsonl` and write errors never affect caller
//! control flow when using [`record_event_best_effort`].

use chrono::Utc;
use serde::Serialize;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::debug;

const LOG_FILE_NAME: &str = "fleet-failures.jsonl";
const SUMMARY_LIMIT: usize = 512;
const DETAILS_LIMIT: usize = 2048;

/// Normalized fleet telemetry event kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FleetEventKind {
    CookTimeout,
    AdapterFlap,
    GateBounce,
    DeployFailure,
    DeploySuccess,
}

impl FleetEventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CookTimeout => "cook_timeout",
            Self::AdapterFlap => "adapter_flap",
            Self::GateBounce => "gate_bounce",
            Self::DeployFailure => "deploy_failure",
            Self::DeploySuccess => "deploy_success",
        }
    }
}

/// Normalized fleet telemetry severity levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FleetSeverity {
    Info,
    Warn,
    Error,
}

impl FleetSeverity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Serialize)]
struct FleetTelemetryEvent<'a> {
    ts: String,
    seat: String,
    host: String,
    kind: &'a str,
    severity: &'a str,
    source: &'a str,
    summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    sha: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<String>,
}

/// Append a normalized event to the default fleet-failure JSONL sink.
///
/// Returns the path written on success. Callers in runtime paths should prefer
/// [`record_event_best_effort`] so telemetry never blocks or fails the gateway.
pub fn record_event(
    kind: FleetEventKind,
    severity: FleetSeverity,
    source: &str,
    summary: &str,
    sha: Option<&str>,
    details: Option<&str>,
) -> io::Result<PathBuf> {
    let path = telemetry_log_path();
    record_event_to_path(&path, kind, severity, source, summary, sha, details)?;
    Ok(path)
}

/// Best-effort event append. Errors are debug-logged and intentionally dropped.
pub fn record_event_best_effort(
    kind: FleetEventKind,
    severity: FleetSeverity,
    source: &str,
    summary: &str,
    sha: Option<&str>,
    details: Option<&str>,
) {
    if let Err(e) = record_event(kind, severity, source, summary, sha, details) {
        debug!(target: "fleet_telemetry", error = %e, "fleet telemetry append skipped");
    }
}

/// Append a normalized event to an explicit path. Public for focused tests and
/// operator tooling that wants the same schema without environment lookup.
pub fn record_event_to_path(
    path: &Path,
    kind: FleetEventKind,
    severity: FleetSeverity,
    source: &str,
    summary: &str,
    sha: Option<&str>,
    details: Option<&str>,
) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let zeus_home = zeus_home_dir();
    let event = FleetTelemetryEvent {
        ts: Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        seat: seat_name(&zeus_home),
        host: host_name(),
        kind: kind.as_str(),
        severity: severity.as_str(),
        source,
        summary: truncate_utf8(summary, SUMMARY_LIMIT).to_string(),
        sha,
        details: details.map(|d| truncate_utf8(d, DETAILS_LIMIT).to_string()),
    };

    let line = serde_json::to_string(&event).map_err(io::Error::other)?;
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{line}")
}

fn telemetry_log_path() -> PathBuf {
    if let Ok(path) = std::env::var("ZEUS_FLEET_FAILURE_LOG") {
        return PathBuf::from(path);
    }

    let log_dir = std::env::var("ZEUS_LOG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| zeus_home_dir().join("logs"));
    log_dir.join(LOG_FILE_NAME)
}

fn zeus_home_dir() -> PathBuf {
    if let Ok(path) = std::env::var("ZEUS_HOME") {
        return PathBuf::from(path);
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".zeus")
}

fn seat_name(zeus_home: &Path) -> String {
    if let Ok(seat) = std::env::var("ZEUS_SEAT") {
        let seat = seat.trim();
        if !seat.is_empty() {
            return seat.to_string();
        }
    }

    let identity = zeus_home.join("IDENTITY.md");
    if let Ok(contents) = fs::read_to_string(identity) {
        for line in contents.lines() {
            if line.starts_with("- **Name**") {
                if let Some((_, value)) = line.split_once(':') {
                    let value = value.trim();
                    if !value.is_empty() {
                        return value.to_string();
                    }
                }
            }
        }
    }

    host_name()
}

fn host_name() -> String {
    for key in ["HOSTNAME", "COMPUTERNAME"] {
        if let Ok(value) = std::env::var(key) {
            let value = value.trim();
            if !value.is_empty() {
                return value.to_string();
            }
        }
    }

    Command::new("hostname")
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

fn truncate_utf8(value: &str, limit: usize) -> &str {
    if value.len() <= limit {
        return value;
    }
    let mut end = limit;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn appends_jsonl_event_with_contract_fields() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("logs").join("fleet-failures.jsonl");

        record_event_to_path(
            &path,
            FleetEventKind::CookTimeout,
            FleetSeverity::Error,
            "cook-wrapper",
            "cook timed out",
            Some("abc123"),
            Some("timeout_secs=1800 source=tui"),
        )
        .expect("event append succeeds");

        let contents = fs::read_to_string(&path).expect("read event log");
        let lines: Vec<_> = contents.lines().collect();
        assert_eq!(lines.len(), 1);

        let event: Value = serde_json::from_str(lines[0]).expect("valid json line");
        assert!(event["ts"].as_str().unwrap().ends_with('Z'));
        assert!(!event["seat"].as_str().unwrap().is_empty());
        assert!(!event["host"].as_str().unwrap().is_empty());
        assert_eq!(event["kind"], "cook_timeout");
        assert_eq!(event["severity"], "error");
        assert_eq!(event["source"], "cook-wrapper");
        assert_eq!(event["summary"], "cook timed out");
        assert_eq!(event["sha"], "abc123");
        assert_eq!(event["details"], "timeout_secs=1800 source=tui");
    }

    #[test]
    fn omits_optional_fields_when_absent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("fleet-failures.jsonl");

        record_event_to_path(
            &path,
            FleetEventKind::AdapterFlap,
            FleetSeverity::Warn,
            "channel-manager",
            "adapter failed to start",
            None,
            None,
        )
        .expect("event append succeeds");

        let contents = fs::read_to_string(&path).expect("read event log");
        let event: Value = serde_json::from_str(contents.lines().next().unwrap()).unwrap();
        assert_eq!(event["kind"], "adapter_flap");
        assert!(event.get("sha").is_none());
        assert!(event.get("details").is_none());
    }

    #[test]
    fn bounds_summary_and_details_without_breaking_utf8() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("fleet-failures.jsonl");
        let long = "⚡".repeat(2000);

        record_event_to_path(
            &path,
            FleetEventKind::GateBounce,
            FleetSeverity::Warn,
            "gate",
            &long,
            None,
            Some(&long),
        )
        .expect("event append succeeds");

        let contents = fs::read_to_string(&path).expect("read event log");
        let event: Value = serde_json::from_str(contents.lines().next().unwrap()).unwrap();
        assert!(event["summary"].as_str().unwrap().len() <= SUMMARY_LIMIT);
        assert!(event["details"].as_str().unwrap().len() <= DETAILS_LIMIT);
    }
}
