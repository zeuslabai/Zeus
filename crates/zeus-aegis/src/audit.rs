//! Tamper-evident audit logging with rotation, severity, querying, and alerts
//!
//! Features:
//! - SHA-256 hash chain for tamper detection
//! - Optional HMAC-SHA256 signing per entry
//! - Log rotation: keep last 10K entries, gzip archive older
//! - Severity levels (Info/Warning/Error/Critical)
//! - User tracking per entry
//! - Query support: filter by tool, user, severity, time range
//! - Suspicious pattern detection for Hermes alerts

use chrono::{DateTime, Utc};
use flate2::Compression;
use flate2::write::GzEncoder;
use ring::digest::{SHA256, digest};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::PathBuf;
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tracing::{info, warn};
use zeus_core::{Error, Result};

/// Maximum entries before rotation triggers.
const MAX_ENTRIES_BEFORE_ROTATION: u64 = 10_000;

/// Severity levels for audit events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Error,
    Critical,
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Severity::Info => write!(f, "info"),
            Severity::Warning => write!(f, "warning"),
            Severity::Error => write!(f, "error"),
            Severity::Critical => write!(f, "critical"),
        }
    }
}

/// Audit event types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuditEvent {
    /// Secret accessed
    SecretAccess { key: String, operation: String },
    /// Tool executed
    ToolExecution {
        tool: String,
        args: serde_json::Value,
        success: bool,
    },
    /// Network request
    NetworkRequest {
        host: String,
        method: String,
        path: String,
    },
    /// File access
    FileAccess { path: String, operation: String },
    /// Authentication event
    Authentication {
        channel: String,
        user: String,
        success: bool,
    },
    /// Permission check
    PermissionCheck { operation: String, allowed: bool },
    /// System event
    System {
        event: String,
        details: Option<String>,
    },
}

impl AuditEvent {
    /// Extract tool name if this is a ToolExecution event.
    pub fn tool_name(&self) -> Option<&str> {
        match self {
            AuditEvent::ToolExecution { tool, .. } => Some(tool),
            _ => None,
        }
    }

    /// Infer a default severity for this event type.
    pub fn default_severity(&self) -> Severity {
        match self {
            AuditEvent::SecretAccess { .. } => Severity::Warning,
            AuditEvent::ToolExecution { success, .. } => {
                if *success {
                    Severity::Info
                } else {
                    Severity::Warning
                }
            }
            AuditEvent::NetworkRequest { .. } => Severity::Info,
            AuditEvent::FileAccess { .. } => Severity::Info,
            AuditEvent::Authentication { success, .. } => {
                if *success {
                    Severity::Info
                } else {
                    Severity::Warning
                }
            }
            AuditEvent::PermissionCheck { allowed, .. } => {
                if *allowed {
                    Severity::Info
                } else {
                    Severity::Error
                }
            }
            AuditEvent::System { .. } => Severity::Info,
        }
    }
}

/// Audit log entry with hash chain
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    /// Entry sequence number
    pub sequence: u64,
    /// Timestamp
    pub timestamp: DateTime<Utc>,
    /// Severity level
    #[serde(default = "default_severity")]
    pub severity: Severity,
    /// User or agent identifier
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    /// Event data
    pub event: AuditEvent,
    /// Hash of previous entry (for tamper detection)
    pub prev_hash: String,
    /// Hash of this entry
    pub hash: String,
    /// HMAC-SHA256 signature of the entry hash (when server key is configured).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub hmac_sig: Option<String>,
}

fn default_severity() -> Severity {
    Severity::Info
}

impl AuditEntry {
    /// Create a new audit entry with default severity inferred from event type.
    pub fn new(sequence: u64, event: AuditEvent, prev_hash: String) -> Self {
        let severity = event.default_severity();
        Self::with_severity(sequence, event, prev_hash, severity, None)
    }

    /// Create a new audit entry with explicit severity and user.
    pub fn with_severity(
        sequence: u64,
        event: AuditEvent,
        prev_hash: String,
        severity: Severity,
        user: Option<String>,
    ) -> Self {
        let timestamp = Utc::now();

        let hash_input = format!(
            "{}:{}:{}:{}",
            sequence,
            timestamp.timestamp_nanos_opt().unwrap_or(0),
            serde_json::to_string(&event).unwrap_or_default(),
            prev_hash
        );
        let hash = hex::encode(digest(&SHA256, hash_input.as_bytes()));

        Self {
            sequence,
            timestamp,
            severity,
            user,
            event,
            prev_hash,
            hash,
            hmac_sig: None,
        }
    }

    /// Verify the hash chain
    pub fn verify(&self, prev_hash: &str) -> bool {
        if self.prev_hash != prev_hash {
            return false;
        }

        let hash_input = format!(
            "{}:{}:{}:{}",
            self.sequence,
            self.timestamp.timestamp_nanos_opt().unwrap_or(0),
            serde_json::to_string(&self.event).unwrap_or_default(),
            self.prev_hash
        );
        let expected_hash = hex::encode(digest(&SHA256, hash_input.as_bytes()));

        self.hash == expected_hash
    }
}

/// Query parameters for filtering audit entries.
#[derive(Debug, Clone, Default)]
pub struct AuditQuery {
    /// Filter by tool name (for ToolExecution events)
    pub tool: Option<String>,
    /// Filter by user
    pub user: Option<String>,
    /// Filter by minimum severity
    pub min_severity: Option<Severity>,
    /// Filter entries after this timestamp
    pub since: Option<DateTime<Utc>>,
    /// Maximum number of entries to return
    pub limit: Option<usize>,
}

/// A suspicious activity alert.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuspiciousAlert {
    /// Alert type identifier
    pub alert_type: String,
    /// Human-readable description
    pub description: String,
    /// Severity of the alert
    pub severity: Severity,
    /// Timestamp of detection
    pub detected_at: DateTime<Utc>,
    /// Related entry sequences
    pub related_sequences: Vec<u64>,
}

/// Configuration for suspicious pattern detection.
#[derive(Debug, Clone)]
pub struct PatternConfig {
    /// Number of auth failures in the window that triggers an alert
    pub auth_failure_threshold: usize,
    /// Time window for auth failure detection (seconds)
    pub auth_failure_window_secs: i64,
    /// Number of permission denials in the window that triggers an alert
    pub permission_denial_threshold: usize,
    /// Time window for permission denial detection (seconds)
    pub permission_denial_window_secs: i64,
}

impl Default for PatternConfig {
    fn default() -> Self {
        Self {
            auth_failure_threshold: 5,
            auth_failure_window_secs: 300, // 5 minutes
            permission_denial_threshold: 10,
            permission_denial_window_secs: 600, // 10 minutes
        }
    }
}

/// Tamper-evident audit log with rotation, querying, and pattern detection.
pub struct AuditLog {
    path: PathBuf,
    sequence: u64,
    last_hash: String,
    hmac_key: Option<ring::hmac::Key>,
    pattern_config: PatternConfig,
}

impl AuditLog {
    /// Create or open an audit log
    pub async fn new(path: &std::path::Path) -> Result<Self> {
        Self::with_hmac_key(path, None).await
    }

    /// Create or open an audit log with an optional HMAC key for signing.
    pub async fn with_hmac_key(
        path: &std::path::Path,
        hmac_key_bytes: Option<&[u8]>,
    ) -> Result<Self> {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| Error::Security(format!("Failed to create audit directory: {}", e)))?;
        }

        let (sequence, last_hash) = if path.exists() {
            Self::read_last_entry(path).await?
        } else {
            (0, "genesis".to_string())
        };

        let hmac_key =
            hmac_key_bytes.map(|bytes| ring::hmac::Key::new(ring::hmac::HMAC_SHA256, bytes));

        Ok(Self {
            path: path.to_path_buf(),
            sequence,
            last_hash,
            hmac_key,
            pattern_config: PatternConfig::default(),
        })
    }

    /// Set the pattern detection configuration.
    pub fn set_pattern_config(&mut self, config: PatternConfig) {
        self.pattern_config = config;
    }

    /// Current entry count.
    pub fn entry_count(&self) -> u64 {
        self.sequence
    }

    /// Read the last entry from the log
    pub(crate) async fn read_last_entry(path: &std::path::Path) -> Result<(u64, String)> {
        let file = File::open(path)
            .await
            .map_err(|e| Error::Security(format!("Failed to open audit log: {}", e)))?;

        let reader = BufReader::new(file);
        let mut lines = reader.lines();
        let mut last_entry: Option<AuditEntry> = None;

        while let Ok(Some(line)) = lines.next_line().await {
            if !line.is_empty()
                && let Ok(entry) = serde_json::from_str::<AuditEntry>(&line)
            {
                last_entry = Some(entry);
            }
        }

        match last_entry {
            Some(entry) => Ok((entry.sequence, entry.hash)),
            None => Ok((0, "genesis".to_string())),
        }
    }

    /// Compute HMAC signature for an audit entry hash.
    fn sign_entry(&self, entry_hash: &str) -> Option<String> {
        self.hmac_key.as_ref().map(|key| {
            let tag = ring::hmac::sign(key, entry_hash.as_bytes());
            hex::encode(tag.as_ref())
        })
    }

    /// Log an event (with auto-rotation check).
    pub async fn log(&mut self, event: AuditEvent) -> Result<()> {
        self.log_with_user(event, None).await
    }

    /// Log an event with an explicit user identifier.
    pub async fn log_with_user(&mut self, event: AuditEvent, user: Option<String>) -> Result<()> {
        // Check if rotation is needed before writing
        if self.sequence >= MAX_ENTRIES_BEFORE_ROTATION
            && self.sequence.is_multiple_of(MAX_ENTRIES_BEFORE_ROTATION)
            && let Err(e) = self.rotate().await
        {
            warn!("Audit log rotation failed: {}", e);
        }

        self.sequence += 1;
        let severity = event.default_severity();
        let mut entry =
            AuditEntry::with_severity(self.sequence, event, self.last_hash.clone(), severity, user);

        entry.hmac_sig = self.sign_entry(&entry.hash);
        self.last_hash = entry.hash.clone();

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await
            .map_err(|e| Error::Security(format!("Failed to open audit log: {}", e)))?;

        let line = serde_json::to_string(&entry)
            .map_err(|e| Error::Security(format!("Failed to serialize audit entry: {}", e)))?;

        file.write_all(format!("{}\n", line).as_bytes())
            .await
            .map_err(|e| Error::Security(format!("Failed to write audit entry: {}", e)))?;

        Ok(())
    }

    /// Log an event with explicit severity and user.
    pub async fn log_with_severity(
        &mut self,
        event: AuditEvent,
        severity: Severity,
        user: Option<String>,
    ) -> Result<()> {
        if self.sequence >= MAX_ENTRIES_BEFORE_ROTATION
            && self.sequence.is_multiple_of(MAX_ENTRIES_BEFORE_ROTATION)
            && let Err(e) = self.rotate().await
        {
            warn!("Audit log rotation failed: {}", e);
        }

        self.sequence += 1;
        let mut entry =
            AuditEntry::with_severity(self.sequence, event, self.last_hash.clone(), severity, user);

        entry.hmac_sig = self.sign_entry(&entry.hash);
        self.last_hash = entry.hash.clone();

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await
            .map_err(|e| Error::Security(format!("Failed to open audit log: {}", e)))?;

        let line = serde_json::to_string(&entry)
            .map_err(|e| Error::Security(format!("Failed to serialize audit entry: {}", e)))?;

        file.write_all(format!("{}\n", line).as_bytes())
            .await
            .map_err(|e| Error::Security(format!("Failed to write audit entry: {}", e)))?;

        Ok(())
    }

    // ========================================================================
    // Log rotation
    // ========================================================================

    /// Rotate the audit log: archive all current entries to a gzip file,
    /// then truncate the active log. The hash chain continues from the
    /// last entry's hash in the new file.
    pub async fn rotate(&self) -> Result<()> {
        if !self.path.exists() {
            return Ok(());
        }

        let content = tokio::fs::read_to_string(&self.path).await.map_err(|e| {
            Error::Security(format!("Failed to read audit log for rotation: {}", e))
        })?;

        let lines: Vec<&str> = content.lines().filter(|l| !l.is_empty()).collect();
        if lines.is_empty() {
            return Ok(());
        }

        // Build archive filename with timestamp
        let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
        let archive_name = format!(
            "{}.{}.gz",
            self.path.file_stem().unwrap_or_default().to_string_lossy(),
            timestamp
        );
        let archive_path = self.path.with_file_name(archive_name);

        // Gzip the content
        let archive_data = {
            let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
            encoder
                .write_all(content.as_bytes())
                .map_err(|e| Error::Security(format!("Failed to compress audit log: {}", e)))?;
            encoder
                .finish()
                .map_err(|e| Error::Security(format!("Failed to finalize gzip: {}", e)))?
        };

        tokio::fs::write(&archive_path, &archive_data)
            .await
            .map_err(|e| Error::Security(format!("Failed to write archive: {}", e)))?;

        // Truncate the active log (keep empty file for new entries)
        tokio::fs::write(&self.path, b"")
            .await
            .map_err(|e| Error::Security(format!("Failed to truncate audit log: {}", e)))?;

        info!(
            "Audit log rotated: {} entries archived to {}",
            lines.len(),
            archive_path.display()
        );

        Ok(())
    }

    // ========================================================================
    // Querying
    // ========================================================================

    /// Read all entries from the active audit log.
    pub async fn read_entries(&self) -> Result<Vec<AuditEntry>> {
        self.query_entries(&AuditQuery::default()).await
    }

    /// Query entries with filtering.
    pub async fn query_entries(&self, query: &AuditQuery) -> Result<Vec<AuditEntry>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }

        let file = File::open(&self.path)
            .await
            .map_err(|e| Error::Security(format!("Failed to open audit log: {}", e)))?;

        let reader = BufReader::new(file);
        let mut lines = reader.lines();
        let mut entries = Vec::new();

        while let Ok(Some(line)) = lines.next_line().await {
            if line.is_empty() {
                continue;
            }
            let entry: AuditEntry = match serde_json::from_str(&line) {
                Ok(e) => e,
                Err(_) => continue,
            };

            // Apply filters
            if let Some(ref tool) = query.tool
                && entry.event.tool_name() != Some(tool.as_str())
            {
                continue;
            }
            if let Some(ref user) = query.user
                && entry.user.as_deref() != Some(user.as_str())
            {
                continue;
            }
            if let Some(min_sev) = query.min_severity
                && entry.severity < min_sev
            {
                continue;
            }
            if let Some(since) = query.since
                && entry.timestamp < since
            {
                continue;
            }

            entries.push(entry);
        }

        // Sort newest first, then apply limit
        entries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

        if let Some(limit) = query.limit {
            entries.truncate(limit);
        }

        Ok(entries)
    }

    /// Read the most recent N entries.
    pub async fn read_recent(&self, limit: usize) -> Result<Vec<AuditEntry>> {
        self.query_entries(&AuditQuery {
            limit: Some(limit),
            ..Default::default()
        })
        .await
    }

    // ========================================================================
    // Suspicious pattern detection
    // ========================================================================

    /// Scan recent entries for suspicious patterns and return alerts.
    pub async fn detect_suspicious_patterns(&self) -> Result<Vec<SuspiciousAlert>> {
        let entries = self.read_entries().await?;
        Ok(self.analyze_patterns(&entries))
    }

    /// Analyze a set of entries for suspicious patterns.
    pub fn analyze_patterns(&self, entries: &[AuditEntry]) -> Vec<SuspiciousAlert> {
        let mut alerts = Vec::new();
        let now = Utc::now();

        // 1. Rapid authentication failures
        let auth_failures: Vec<&AuditEntry> = entries
            .iter()
            .filter(|e| {
                matches!(&e.event, AuditEvent::Authentication { success, .. } if !success)
                    && (now - e.timestamp).num_seconds()
                        < self.pattern_config.auth_failure_window_secs
            })
            .collect();

        if auth_failures.len() >= self.pattern_config.auth_failure_threshold {
            alerts.push(SuspiciousAlert {
                alert_type: "rapid_auth_failures".to_string(),
                description: format!(
                    "{} authentication failures in the last {} seconds",
                    auth_failures.len(),
                    self.pattern_config.auth_failure_window_secs
                ),
                severity: Severity::Critical,
                detected_at: now,
                related_sequences: auth_failures.iter().map(|e| e.sequence).collect(),
            });
        }

        // 2. Rapid permission denials
        let perm_denials: Vec<&AuditEntry> = entries
            .iter()
            .filter(|e| {
                matches!(&e.event, AuditEvent::PermissionCheck { allowed, .. } if !allowed)
                    && (now - e.timestamp).num_seconds()
                        < self.pattern_config.permission_denial_window_secs
            })
            .collect();

        if perm_denials.len() >= self.pattern_config.permission_denial_threshold {
            alerts.push(SuspiciousAlert {
                alert_type: "rapid_permission_denials".to_string(),
                description: format!(
                    "{} permission denials in the last {} seconds",
                    perm_denials.len(),
                    self.pattern_config.permission_denial_window_secs
                ),
                severity: Severity::Error,
                detected_at: now,
                related_sequences: perm_denials.iter().map(|e| e.sequence).collect(),
            });
        }

        // 3. Secret access spikes (>3 accesses in 60 seconds)
        let secret_accesses: Vec<&AuditEntry> = entries
            .iter()
            .filter(|e| {
                matches!(&e.event, AuditEvent::SecretAccess { .. })
                    && (now - e.timestamp).num_seconds() < 60
            })
            .collect();

        if secret_accesses.len() > 3 {
            alerts.push(SuspiciousAlert {
                alert_type: "secret_access_spike".to_string(),
                description: format!(
                    "{} secret accesses in the last 60 seconds",
                    secret_accesses.len()
                ),
                severity: Severity::Warning,
                detected_at: now,
                related_sequences: secret_accesses.iter().map(|e| e.sequence).collect(),
            });
        }

        // 4. Tool execution failures (>5 failures in 5 minutes)
        let tool_failures: Vec<&AuditEntry> = entries
            .iter()
            .filter(|e| {
                matches!(&e.event, AuditEvent::ToolExecution { success, .. } if !success)
                    && (now - e.timestamp).num_seconds() < 300
            })
            .collect();

        if tool_failures.len() > 5 {
            alerts.push(SuspiciousAlert {
                alert_type: "tool_failure_spike".to_string(),
                description: format!(
                    "{} tool execution failures in the last 5 minutes",
                    tool_failures.len()
                ),
                severity: Severity::Warning,
                detected_at: now,
                related_sequences: tool_failures.iter().map(|e| e.sequence).collect(),
            });
        }

        alerts
    }

    /// Build a Hermes-compatible notification message from an alert.
    pub fn alert_to_notification_message(alert: &SuspiciousAlert) -> String {
        format!(
            "🚨 Security Alert: {}\n\nSeverity: {}\nType: {}\nDetected: {}\nRelated entries: {:?}",
            alert.description,
            alert.severity,
            alert.alert_type,
            alert.detected_at.format("%Y-%m-%d %H:%M:%S UTC"),
            alert.related_sequences
        )
    }

    // ========================================================================
    // Verification
    // ========================================================================

    /// Verify the integrity of the audit log.
    pub async fn verify(&self) -> Result<bool> {
        if !self.path.exists() {
            return Ok(true);
        }

        let file = File::open(&self.path)
            .await
            .map_err(|e| Error::Security(format!("Failed to open audit log: {}", e)))?;

        let reader = BufReader::new(file);
        let mut lines = reader.lines();
        let mut prev_hash = "genesis".to_string();

        while let Ok(Some(line)) = lines.next_line().await {
            if line.is_empty() {
                continue;
            }

            let entry: AuditEntry = serde_json::from_str(&line)
                .map_err(|e| Error::Security(format!("Invalid audit entry: {}", e)))?;

            if !entry.verify(&prev_hash) {
                return Ok(false);
            }

            // Verify HMAC signature when a key is configured
            if let Some(ref key) = self.hmac_key {
                match &entry.hmac_sig {
                    Some(sig) => {
                        let sig_bytes = hex::decode(sig).map_err(|_| {
                            Error::Security("Invalid HMAC hex in audit entry".into())
                        })?;
                        if ring::hmac::verify(key, entry.hash.as_bytes(), &sig_bytes).is_err() {
                            return Ok(false);
                        }
                    }
                    None => {
                        return Ok(false);
                    }
                }
            }

            prev_hash = entry.hash;
        }

        Ok(true)
    }

    /// Get the log file path.
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audit_entry_hash_chain() {
        let event = AuditEvent::System {
            event: "test".into(),
            details: None,
        };

        let entry1 = AuditEntry::new(1, event.clone(), "genesis".into());
        let entry2 = AuditEntry::new(2, event, entry1.hash.clone());

        assert!(entry1.verify("genesis"));
        assert!(entry2.verify(&entry1.hash));
        assert!(!entry2.verify("wrong_hash"));
    }

    #[test]
    fn test_severity_ordering() {
        assert!(Severity::Info < Severity::Warning);
        assert!(Severity::Warning < Severity::Error);
        assert!(Severity::Error < Severity::Critical);
    }

    #[test]
    fn test_severity_display() {
        assert_eq!(Severity::Info.to_string(), "info");
        assert_eq!(Severity::Critical.to_string(), "critical");
    }

    #[test]
    fn test_severity_serialization() {
        let json = serde_json::to_string(&Severity::Warning).expect("should serialize");
        assert_eq!(json, "\"warning\"");
        let de: Severity = serde_json::from_str("\"error\"").expect("should parse");
        assert_eq!(de, Severity::Error);
    }

    #[test]
    fn test_event_default_severity() {
        let tool_ok = AuditEvent::ToolExecution {
            tool: "read_file".into(),
            args: serde_json::json!({}),
            success: true,
        };
        assert_eq!(tool_ok.default_severity(), Severity::Info);

        let tool_fail = AuditEvent::ToolExecution {
            tool: "shell".into(),
            args: serde_json::json!({}),
            success: false,
        };
        assert_eq!(tool_fail.default_severity(), Severity::Warning);

        let perm_denied = AuditEvent::PermissionCheck {
            operation: "fs.write".into(),
            allowed: false,
        };
        assert_eq!(perm_denied.default_severity(), Severity::Error);

        let auth_fail = AuditEvent::Authentication {
            channel: "api".into(),
            user: "unknown".into(),
            success: false,
        };
        assert_eq!(auth_fail.default_severity(), Severity::Warning);
    }

    #[test]
    fn test_event_tool_name() {
        let tool = AuditEvent::ToolExecution {
            tool: "shell".into(),
            args: serde_json::json!({}),
            success: true,
        };
        assert_eq!(tool.tool_name(), Some("shell"));

        let system = AuditEvent::System {
            event: "start".into(),
            details: None,
        };
        assert_eq!(system.tool_name(), None);
    }

    #[test]
    fn test_entry_with_severity() {
        let event = AuditEvent::System {
            event: "test".into(),
            details: None,
        };
        let entry = AuditEntry::with_severity(
            1,
            event,
            "genesis".into(),
            Severity::Critical,
            Some("admin".to_string()),
        );
        assert_eq!(entry.severity, Severity::Critical);
        assert_eq!(entry.user.as_deref(), Some("admin"));
        assert!(entry.verify("genesis"));
    }

    #[test]
    fn test_entry_serialization_with_severity() {
        let event = AuditEvent::ToolExecution {
            tool: "shell".into(),
            args: serde_json::json!({"cmd": "ls"}),
            success: true,
        };
        let entry = AuditEntry::with_severity(
            1,
            event,
            "genesis".into(),
            Severity::Info,
            Some("agent-1".into()),
        );

        let json = serde_json::to_string(&entry).expect("should serialize");
        assert!(json.contains("\"severity\":\"info\""));
        assert!(json.contains("\"user\":\"agent-1\""));

        let parsed: AuditEntry = serde_json::from_str(&json).expect("should parse");
        assert_eq!(parsed.severity, Severity::Info);
        assert_eq!(parsed.user.as_deref(), Some("agent-1"));
    }

    #[test]
    fn test_backward_compat_missing_severity() {
        // Old entries without severity field should default to Info
        let json = r#"{"sequence":1,"timestamp":"2026-02-18T00:00:00Z","event":{"type":"system","event":"test"},"prev_hash":"genesis","hash":"abc123"}"#;
        let entry: AuditEntry = serde_json::from_str(json).expect("should parse");
        assert_eq!(entry.severity, Severity::Info);
        assert!(entry.user.is_none());
    }

    #[tokio::test]
    async fn test_audit_log_create_and_log() {
        let tmp = tempfile::tempdir().expect("should create temp dir");
        let path = tmp.path().join("audit.log");

        let mut log = AuditLog::new(&path).await.expect("should create log");
        assert_eq!(log.entry_count(), 0);

        log.log(AuditEvent::System {
            event: "start".into(),
            details: None,
        })
        .await
        .expect("should log");

        assert_eq!(log.entry_count(), 1);

        log.log_with_user(
            AuditEvent::ToolExecution {
                tool: "shell".into(),
                args: serde_json::json!({}),
                success: true,
            },
            Some("user-1".into()),
        )
        .await
        .expect("should log");

        assert_eq!(log.entry_count(), 2);

        let entries = log.read_entries().await.expect("should read");
        assert_eq!(entries.len(), 2);
    }

    #[tokio::test]
    async fn test_query_entries_by_tool() {
        let tmp = tempfile::tempdir().expect("should create temp dir");
        let path = tmp.path().join("audit.log");
        let mut log = AuditLog::new(&path).await.expect("should create log");

        log.log(AuditEvent::ToolExecution {
            tool: "shell".into(),
            args: serde_json::json!({}),
            success: true,
        })
        .await
        .unwrap();

        log.log(AuditEvent::ToolExecution {
            tool: "read_file".into(),
            args: serde_json::json!({}),
            success: true,
        })
        .await
        .unwrap();

        log.log(AuditEvent::System {
            event: "test".into(),
            details: None,
        })
        .await
        .unwrap();

        let query = AuditQuery {
            tool: Some("shell".into()),
            ..Default::default()
        };
        let results = log.query_entries(&query).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].event.tool_name(), Some("shell"));
    }

    #[tokio::test]
    async fn test_query_entries_by_severity() {
        let tmp = tempfile::tempdir().expect("should create temp dir");
        let path = tmp.path().join("audit.log");
        let mut log = AuditLog::new(&path).await.expect("should create log");

        log.log(AuditEvent::ToolExecution {
            tool: "shell".into(),
            args: serde_json::json!({}),
            success: true, // Info
        })
        .await
        .unwrap();

        log.log(AuditEvent::PermissionCheck {
            operation: "fs.write".into(),
            allowed: false, // Error
        })
        .await
        .unwrap();

        let query = AuditQuery {
            min_severity: Some(Severity::Error),
            ..Default::default()
        };
        let results = log.query_entries(&query).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].severity, Severity::Error);
    }

    #[tokio::test]
    async fn test_query_entries_with_limit() {
        let tmp = tempfile::tempdir().expect("should create temp dir");
        let path = tmp.path().join("audit.log");
        let mut log = AuditLog::new(&path).await.expect("should create log");

        for i in 0..10 {
            log.log(AuditEvent::System {
                event: format!("event-{}", i),
                details: None,
            })
            .await
            .unwrap();
        }

        let recent = log.read_recent(3).await.unwrap();
        assert_eq!(recent.len(), 3);
        // Should be newest first
        assert!(recent[0].sequence > recent[1].sequence);
    }

    #[tokio::test]
    async fn test_rotation() {
        let tmp = tempfile::tempdir().expect("should create temp dir");
        let path = tmp.path().join("audit.log");
        let mut log = AuditLog::new(&path).await.expect("should create log");

        // Write some entries
        for _ in 0..5 {
            log.log(AuditEvent::System {
                event: "test".into(),
                details: None,
            })
            .await
            .unwrap();
        }

        assert_eq!(log.entry_count(), 5);

        // Rotate
        log.rotate().await.expect("should rotate");

        // Active log should be empty now
        let content = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(content.is_empty());

        // Archive .gz file should exist
        let mut archives = Vec::new();
        let mut entries = tokio::fs::read_dir(tmp.path()).await.unwrap();
        while let Some(entry) = entries.next_entry().await.unwrap() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(".gz") {
                archives.push(name);
            }
        }
        assert_eq!(archives.len(), 1);
        assert!(archives[0].starts_with("audit."));

        // Verify archive can be decompressed
        let archive_path = tmp.path().join(&archives[0]);
        let compressed = tokio::fs::read(&archive_path).await.unwrap();
        let mut decoder = flate2::read::GzDecoder::new(&compressed[..]);
        let mut decompressed = String::new();
        std::io::Read::read_to_string(&mut decoder, &mut decompressed).unwrap();
        assert_eq!(decompressed.lines().count(), 5);
    }

    #[tokio::test]
    async fn test_suspicious_auth_failures() {
        let tmp = tempfile::tempdir().expect("should create temp dir");
        let path = tmp.path().join("audit.log");
        let mut log = AuditLog::new(&path).await.expect("should create log");
        log.set_pattern_config(PatternConfig {
            auth_failure_threshold: 3,
            auth_failure_window_secs: 300,
            ..Default::default()
        });

        // Write 3 auth failures
        for _ in 0..3 {
            log.log(AuditEvent::Authentication {
                channel: "api".into(),
                user: "attacker".into(),
                success: false,
            })
            .await
            .unwrap();
        }

        let alerts = log.detect_suspicious_patterns().await.unwrap();
        assert!(!alerts.is_empty());
        assert_eq!(alerts[0].alert_type, "rapid_auth_failures");
        assert_eq!(alerts[0].severity, Severity::Critical);
    }

    #[tokio::test]
    async fn test_suspicious_permission_denials() {
        let tmp = tempfile::tempdir().expect("should create temp dir");
        let path = tmp.path().join("audit.log");
        let mut log = AuditLog::new(&path).await.expect("should create log");
        log.set_pattern_config(PatternConfig {
            permission_denial_threshold: 3,
            permission_denial_window_secs: 600,
            ..Default::default()
        });

        for i in 0..4 {
            log.log(AuditEvent::PermissionCheck {
                operation: format!("op-{}", i),
                allowed: false,
            })
            .await
            .unwrap();
        }

        let alerts = log.detect_suspicious_patterns().await.unwrap();
        let perm_alert = alerts
            .iter()
            .find(|a| a.alert_type == "rapid_permission_denials");
        assert!(perm_alert.is_some());
    }

    #[tokio::test]
    async fn test_no_alerts_when_clean() {
        let tmp = tempfile::tempdir().expect("should create temp dir");
        let path = tmp.path().join("audit.log");
        let mut log = AuditLog::new(&path).await.expect("should create log");

        log.log(AuditEvent::ToolExecution {
            tool: "read_file".into(),
            args: serde_json::json!({}),
            success: true,
        })
        .await
        .unwrap();

        let alerts = log.detect_suspicious_patterns().await.unwrap();
        assert!(alerts.is_empty());
    }

    #[test]
    fn test_alert_to_notification_message() {
        let alert = SuspiciousAlert {
            alert_type: "rapid_auth_failures".into(),
            description: "5 auth failures in 300s".into(),
            severity: Severity::Critical,
            detected_at: Utc::now(),
            related_sequences: vec![1, 2, 3, 4, 5],
        };

        let msg = AuditLog::alert_to_notification_message(&alert);
        assert!(msg.contains("Security Alert"));
        assert!(msg.contains("5 auth failures"));
        assert!(msg.contains("critical"));
    }

    #[tokio::test]
    async fn test_verify_after_log() {
        let tmp = tempfile::tempdir().expect("should create temp dir");
        let path = tmp.path().join("audit.log");
        let mut log = AuditLog::new(&path).await.expect("should create log");

        log.log(AuditEvent::System {
            event: "start".into(),
            details: None,
        })
        .await
        .unwrap();

        log.log(AuditEvent::PermissionCheck {
            operation: "fs.read".into(),
            allowed: true,
        })
        .await
        .unwrap();

        assert!(log.verify().await.unwrap());
    }

    #[tokio::test]
    async fn test_log_with_severity() {
        let tmp = tempfile::tempdir().expect("should create temp dir");
        let path = tmp.path().join("audit.log");
        let mut log = AuditLog::new(&path).await.expect("should create log");

        log.log_with_severity(
            AuditEvent::System {
                event: "critical_issue".into(),
                details: Some("disk full".into()),
            },
            Severity::Critical,
            Some("admin".into()),
        )
        .await
        .unwrap();

        let entries = log.read_entries().await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].severity, Severity::Critical);
        assert_eq!(entries[0].user.as_deref(), Some("admin"));
    }

    #[test]
    fn test_pattern_config_default() {
        let cfg = PatternConfig::default();
        assert_eq!(cfg.auth_failure_threshold, 5);
        assert_eq!(cfg.auth_failure_window_secs, 300);
        assert_eq!(cfg.permission_denial_threshold, 10);
        assert_eq!(cfg.permission_denial_window_secs, 600);
    }

    #[test]
    fn test_suspicious_alert_serialization() {
        let alert = SuspiciousAlert {
            alert_type: "test".into(),
            description: "test alert".into(),
            severity: Severity::Warning,
            detected_at: Utc::now(),
            related_sequences: vec![1, 2],
        };

        let json = serde_json::to_string(&alert).expect("should serialize");
        let parsed: SuspiciousAlert = serde_json::from_str(&json).expect("should parse");
        assert_eq!(parsed.alert_type, "test");
        assert_eq!(parsed.severity, Severity::Warning);
        assert_eq!(parsed.related_sequences, vec![1, 2]);
    }
}
