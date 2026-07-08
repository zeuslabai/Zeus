//! `zeus logs` — tail the gateway log without knowing where it lives.
//!
//! Gateway observability P3. Resolves the platform/instance-correct log
//! directory and picks the freshest gateway log file:
//!
//! - Base dir: `{zeus_home}/logs/` where `zeus_home` honors `$ZEUS_HOME`
//!   (see `zeus_paths::zeus_home`). With `--instance NAME`, the base becomes
//!   `~/.zeus/instances/NAME` — same layout the multi-instance gateway uses.
//! - File: newest-mtime match among `gateway*.log*` — covers the P1
//!   `tracing-appender` daily files (`gateway.YYYY-MM-DD.log` /
//!   `gateway.log.YYYY-MM-DD`), a plain `gateway.log`, and the service
//!   redirection files (`gateway.out.log`, `gateway.err.log`).
//! - `-n N` prints the last N lines (default 50); `-f` follows, surviving
//!   daily rotation by re-resolving the newest file on each poll.

use anyhow::{Context, Result, bail};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::zeus_paths;

/// Resolve the logs directory for the (optionally named) instance.
fn logs_dir(instance: Option<&str>) -> Result<PathBuf> {
    let home = match instance {
        Some(name) => {
            // Mirror the multi-instance layout: ~/.zeus/instances/<name>.
            // Validate the name the same defensive way (path-safe token).
            if name.is_empty()
                || !name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            {
                bail!(
                    "invalid instance name '{name}': use only [A-Za-z0-9_-]"
                );
            }
            dirs::home_dir()
                .context("could not resolve home directory")?
                .join(".zeus")
                .join("instances")
                .join(name)
        }
        None => zeus_paths::zeus_home(),
    };
    Ok(home.join("logs"))
}

/// Newest-mtime `gateway*` log file in `dir`, if any.
fn newest_gateway_log(dir: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        // gateway.log, gateway.2026-07-02.log, gateway.log.2026-07-02,
        // gateway.out.log, gateway.err.log — all start with "gateway" and
        // mention "log".
        if !name.starts_with("gateway") || !name.contains("log") {
            continue;
        }
        let mtime = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        if best.as_ref().map(|(t, _)| mtime > *t).unwrap_or(true) {
            best = Some((mtime, path));
        }
    }
    best.map(|(_, p)| p)
}

/// Print the last `n` lines of `path`.
///
/// Reads at most the trailing 1 MiB — plenty for any sane `-n`, and keeps
/// giant unrotated logs from being slurped whole.
fn print_tail(path: &Path, n: usize) -> Result<u64> {
    const MAX_TAIL_BYTES: u64 = 1024 * 1024;
    let mut f = std::fs::File::open(path)
        .with_context(|| format!("cannot open {}", path.display()))?;
    let len = f.metadata()?.len();
    let start = len.saturating_sub(MAX_TAIL_BYTES);
    f.seek(SeekFrom::Start(start))?;
    let mut buf = String::new();
    f.read_to_string(&mut buf).unwrap_or(0);
    let lines: Vec<&str> = buf.lines().collect();
    let skip = lines.len().saturating_sub(n);
    for line in &lines[skip..] {
        println!("{line}");
    }
    Ok(len)
}

/// Entry point for `zeus logs`.
pub async fn run_logs(instance: Option<String>, lines: usize, follow: bool) -> Result<()> {
    let dir = logs_dir(instance.as_deref())?;
    let Some(mut current) = newest_gateway_log(&dir) else {
        bail!(
            "no gateway log found in {} — is the gateway running? (zeus daemon status)",
            dir.display()
        );
    };

    eprintln!("==> {}", current.display());
    let mut offset = print_tail(&current, lines)?;

    if !follow {
        return Ok(());
    }

    // Follow mode: poll for growth; survive daily rotation by re-resolving
    // the newest file each tick and switching when it changes.
    loop {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        if let Some(newest) = newest_gateway_log(&dir)
            && newest != current
        {
            eprintln!("==> {} (rotated)", newest.display());
            current = newest;
            offset = 0;
        }

        let Ok(meta) = std::fs::metadata(&current) else {
            continue;
        };
        let len = meta.len();
        if len < offset {
            // Truncated in place — restart from the top.
            offset = 0;
        }
        if len > offset {
            let mut f = match std::fs::File::open(&current) {
                Ok(f) => f,
                Err(_) => continue,
            };
            if f.seek(SeekFrom::Start(offset)).is_err() {
                continue;
            }
            let mut buf = String::new();
            if f.read_to_string(&mut buf).is_ok() {
                print!("{buf}");
                use std::io::Write;
                std::io::stdout().flush().ok();
            }
            offset = len;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_gateway_log_picks_latest_mtime() {
        let dir = std::env::temp_dir().join(format!("zeus-logs-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let older = dir.join("gateway.2026-07-01.log");
        let newer = dir.join("gateway.2026-07-02.log");
        std::fs::write(&older, "old\n").unwrap();
        std::fs::write(&newer, "new\n").unwrap();
        // Ensure distinct mtimes regardless of filesystem granularity.
        let past = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        let f = std::fs::File::open(&older).unwrap();
        f.set_modified(past).unwrap();
        assert_eq!(newest_gateway_log(&dir), Some(newer));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn newest_gateway_log_ignores_non_gateway_files() {
        let dir =
            std::env::temp_dir().join(format!("zeus-logs-test2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("build.log"), "x\n").unwrap();
        std::fs::write(dir.join("other.txt"), "x\n").unwrap();
        assert_eq!(newest_gateway_log(&dir), None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn invalid_instance_name_rejected() {
        assert!(logs_dir(Some("../evil")).is_err());
        assert!(logs_dir(Some("")).is_err());
        assert!(logs_dir(Some("ok-name_1")).is_ok());
    }
}
