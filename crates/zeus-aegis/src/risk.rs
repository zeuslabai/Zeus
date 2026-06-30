//! Aegis policy → Risk level mapping
//!
//! Maps tool names to a `RiskLevel` based on their destructive/reversibility
//! profile. Used by the approvals API to surface risk in the WebUI.

use serde::{Deserialize, Serialize};

/// Risk level for a pending approval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

impl RiskLevel {
    /// Return a human-readable label.
    pub fn label(&self) -> &'static str {
        match self {
            RiskLevel::Low => "Low",
            RiskLevel::Medium => "Medium",
            RiskLevel::High => "High",
            RiskLevel::Critical => "Critical",
        }
    }
}

impl std::fmt::Display for RiskLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// Compute the `RiskLevel` for a tool call given its name and args.
///
/// Logic mirrors Aegis approval-trigger heuristics:
/// - `Critical`: irreversible system-wide operations (rm -rf, DROP TABLE, etc.)
/// - `High`:     destructive but scoped (file delete, db mutations, sudo)
/// - `Medium`:   side-effects that are reversible (write/create, network POST)
/// - `Low`:      read-only / benign (read, list, echo, get)
pub fn tool_risk(tool_name: &str, args: &serde_json::Value) -> RiskLevel {
    let tool = tool_name.to_lowercase();

    // ── Critical: shell commands with destructive patterns ──────────────────
    if tool == "shell" || tool == "bash" || tool == "run_command" {
        if let Some(cmd) = args.get("command").and_then(|v| v.as_str()) {
            let cmd_lower = cmd.to_lowercase();
            // Irreversible / system-wide
            if cmd_lower.contains("rm -rf")
                || cmd_lower.contains("rm -fr")
                || cmd_lower.contains(":(){:|:&};:")  // fork bomb
                || cmd_lower.contains("mkfs")
                || cmd_lower.contains("dd if=")
                || cmd_lower.contains("wipefs")
                || cmd_lower.contains("> /dev/")
            {
                return RiskLevel::Critical;
            }
            // High: destructive but scoped
            if cmd_lower.contains("rm ")
                || cmd_lower.contains("sudo ")
                || cmd_lower.contains("chmod ")
                || cmd_lower.contains("chown ")
                || cmd_lower.contains("kill ")
                || cmd_lower.contains("pkill")
                || cmd_lower.contains("systemctl")
                || cmd_lower.contains("apt ")
                || cmd_lower.contains("apt-get")
                || cmd_lower.contains("yum ")
                || cmd_lower.contains("dnf ")
            {
                return RiskLevel::High;
            }
            // Medium: writes / network
            if cmd_lower.contains("curl ")
                || cmd_lower.contains("wget ")
                || cmd_lower.contains("git push")
                || cmd_lower.contains("git commit")
                || cmd_lower.contains("> ")   // redirect write
                || cmd_lower.contains(">>")
                || cmd_lower.contains("tee ")
                || cmd_lower.contains("cp ")
                || cmd_lower.contains("mv ")
                || cmd_lower.contains("mkdir ")
            {
                return RiskLevel::Medium;
            }
            return RiskLevel::Low;
        }
        // shell with no command arg — treat as medium
        return RiskLevel::Medium;
    }

    // ── Database tools ────────────────────────────────────────────────────────
    if tool.contains("db") || tool.contains("sql") || tool.contains("database") {
        if let Some(q) = args.get("query").or_else(|| args.get("sql")).and_then(|v| v.as_str()) {
            let q_upper = q.to_uppercase();
            if q_upper.contains("DROP ")
                || q_upper.contains("TRUNCATE ")
                || q_upper.contains("DELETE ")
                    && q_upper.contains("WHERE") == false
            {
                return RiskLevel::Critical;
            }
            if q_upper.contains("DELETE ")
                || q_upper.contains("UPDATE ")
                || q_upper.contains("ALTER ")
            {
                return RiskLevel::High;
            }
            if q_upper.contains("INSERT ") || q_upper.contains("CREATE ") {
                return RiskLevel::Medium;
            }
        }
        return RiskLevel::Low;
    }

    // ── File system tools ─────────────────────────────────────────────────────
    match tool.as_str() {
        "write_file" | "create_file" | "edit_file" | "apply_patch" => RiskLevel::Medium,
        "delete_file" | "remove_file" | "trash" => RiskLevel::High,
        "read_file" | "list_dir" | "glob" | "search_files" => RiskLevel::Low,

        // Network / messaging
        "web_fetch" | "web_search" | "deep_research" => RiskLevel::Low,
        "send_message" | "discord_send_message" | "telegram_send_message" => RiskLevel::Medium,
        "discord_delete_message" | "telegram_delete_message" => RiskLevel::High,

        // Code execution
        "execute_code" | "run_python" | "run_script" => RiskLevel::High,

        // Spawn / agent control
        "spawn" | "spawn_agent" => RiskLevel::Medium,

        // Credential / secret access
        "read_secret" | "get_secret" | "vault_read" => RiskLevel::High,
        "write_secret" | "set_secret" | "vault_write" => RiskLevel::Critical,

        // Everything else — unknown tool, treat conservatively
        _ => RiskLevel::Medium,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_risk_low_read_tool() {
        assert_eq!(tool_risk("read_file", &json!({"path": "/etc/hosts"})), RiskLevel::Low);
        assert_eq!(tool_risk("web_search", &json!({"query": "hello"})), RiskLevel::Low);
        assert_eq!(tool_risk("list_dir", &json!({"path": "/tmp"})), RiskLevel::Low);
    }

    #[test]
    fn test_risk_medium_write_tool() {
        assert_eq!(tool_risk("write_file", &json!({"path": "/tmp/x", "content": "y"})), RiskLevel::Medium);
        assert_eq!(tool_risk("send_message", &json!({"text": "hi"})), RiskLevel::Medium);
        assert_eq!(tool_risk("shell", &json!({"command": "cp foo bar"})), RiskLevel::Medium);
    }

    #[test]
    fn test_risk_high_destructive_tool() {
        assert_eq!(tool_risk("delete_file", &json!({"path": "/tmp/x"})), RiskLevel::High);
        assert_eq!(tool_risk("shell", &json!({"command": "sudo apt install vim"})), RiskLevel::High);
        assert_eq!(tool_risk("execute_code", &json!({"code": "print('hi')"})), RiskLevel::High);
    }

    #[test]
    fn test_risk_critical_shell_rm_rf() {
        assert_eq!(
            tool_risk("shell", &json!({"command": "rm -rf /tmp/mydir"})),
            RiskLevel::Critical
        );
        assert_eq!(
            tool_risk("bash", &json!({"command": "rm -rf /"})),
            RiskLevel::Critical
        );
    }

    #[test]
    fn test_risk_critical_vault_write() {
        assert_eq!(tool_risk("vault_write", &json!({"key": "secret", "value": "x"})), RiskLevel::Critical);
        assert_eq!(tool_risk("write_secret", &json!({})), RiskLevel::Critical);
    }

    #[test]
    fn test_risk_unknown_tool_defaults_medium() {
        assert_eq!(tool_risk("some_custom_tool", &json!({})), RiskLevel::Medium);
    }
}
