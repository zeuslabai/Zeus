//! Tool execution hooks — pre/post lifecycle interceptors.
//!
//! Supports shell-based hooks from ~/.zeus/hooks/tools/ and config-driven hooks.
//! Exit codes: 0 = allow/continue, 2 = deny/abort, 3 = skip, other = warn.

use serde_json::Value;
use std::process::Stdio;
use tracing::{debug, info, warn};

/// Hook event types for different lifecycle points
#[derive(Debug, Clone, PartialEq)]
pub enum HookEventType {
    OnMessageReceived,
    OnSessionStart,
    OnSessionEnd,
    OnAgentLoopStart,
    OnAgentLoopEnd,
    OnToolExecuted,
    OnError,
    PreToolUse,
    PostToolUse,
}

impl std::fmt::Display for HookEventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OnMessageReceived => write!(f, "on_message_received"),
            Self::OnSessionStart => write!(f, "on_session_start"),
            Self::OnSessionEnd => write!(f, "on_session_end"),
            Self::OnAgentLoopStart => write!(f, "on_agent_loop_start"),
            Self::OnAgentLoopEnd => write!(f, "on_agent_loop_end"),
            Self::OnToolExecuted => write!(f, "on_tool_executed"),
            Self::OnError => write!(f, "on_error"),
            Self::PreToolUse => write!(f, "pre_tool_use"),
            Self::PostToolUse => write!(f, "post_tool_use"),
        }
    }
}

/// Context passed to hooks
#[derive(Debug, Clone)]
pub struct HookContext {
    pub event_type: HookEventType,
    pub session_id: String,
    pub content: Option<String>,
    pub tool_name: Option<String>,
    pub tool_args: Option<Value>,
    pub error: Option<String>,
}

impl HookContext {
    pub fn new(event_type: HookEventType, session_id: &str) -> Self {
        Self {
            event_type,
            session_id: session_id.to_string(),
            content: None,
            tool_name: None,
            tool_args: None,
            error: None,
        }
    }

    pub fn with_content(mut self, content: &str) -> Self {
        self.content = Some(content.to_string());
        self
    }

    pub fn with_tool(mut self, name: &str, args: &Value) -> Self {
        self.tool_name = Some(name.to_string());
        self.tool_args = Some(args.clone());
        self
    }

    /// Overload for post-tool context with success + output instead of args
    pub fn with_tool_result(mut self, name: &str, success: bool, output: &str) -> Self {
        self.tool_name = Some(name.to_string());
        self.tool_args = Some(serde_json::json!({"success": success, "output": output}));
        self
    }

    pub fn with_error(mut self, error: &str) -> Self {
        self.error = Some(error.to_string());
        self
    }

    pub fn with_iteration(mut self, iteration: usize) -> Self {
        // Store iteration in content for hook scripts to read
        if self.content.is_none() {
            self.content = Some(format!("iteration:{}", iteration));
        }
        self
    }

    fn to_payload(&self) -> Value {
        serde_json::json!({
            "event": self.event_type.to_string(),
            "session_id": self.session_id,
            "content": self.content,
            "tool": self.tool_name,
            "args": self.tool_args,
            "error": self.error,
        })
    }
}

/// Result of running a hook
#[derive(Debug, Clone, PartialEq)]
pub enum HookAction {
    Continue,
    Abort(String),
    Skip,
    Allow,
    Deny(String),
    Warn(String),
    ModifyMessage(String),
}

/// Hook definition
#[derive(Debug, Clone)]
pub struct Hook {
    pub event_pattern: String,
    pub command: String,
}

/// Hook registry — manages and executes lifecycle hooks
#[derive(Debug, Clone)]
pub struct HookRegistry {
    hooks: Vec<Hook>,
}

impl HookRegistry {
    pub fn new() -> Self {
        let mut registry = Self { hooks: Vec::new() };
        // Auto-load from ~/.zeus/hooks/tools/
        registry.load_from_hooks_dir();
        registry
    }

    pub fn from_config(config: &zeus_core::HooksConfig) -> Self {
        let mut hooks: Vec<Hook> = Vec::new();
        for (pattern, cmd) in &config.shell_hooks {
            hooks.push(Hook { event_pattern: pattern.clone(), command: cmd.clone() });
        }
        for (pattern, cmd) in &config.before_tool {
            hooks.push(Hook { event_pattern: format!("pre-{}", pattern), command: cmd.clone() });
        }
        for (pattern, cmd) in &config.after_tool {
            hooks.push(Hook { event_pattern: format!("post-{}", pattern), command: cmd.clone() });
        }
        let mut registry = Self { hooks };
        registry.load_from_hooks_dir();
        registry
    }

    fn load_from_hooks_dir(&mut self) {
        if let Some(home) = dirs::home_dir() {
            let hooks_dir = home.join(".zeus").join("hooks").join("tools");
            if hooks_dir.is_dir() {
                if let Ok(entries) = std::fs::read_dir(&hooks_dir) {
                    for entry in entries.filter_map(|e| e.ok()) {
                        let path = entry.path();
                        if !path.is_file() { continue; }
                        let name = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
                        self.hooks.push(Hook {
                            event_pattern: name,
                            command: path.to_string_lossy().to_string(),
                        });
                    }
                }
            }
        }
    }

    /// Fire hooks and resolve to an action. Returns Continue if no hooks match or all allow.
    pub async fn fire_resolve(&self, ctx: &HookContext) -> HookAction {
        if self.hooks.is_empty() {
            return HookAction::Continue;
        }

        let event_str = ctx.event_type.to_string();
        let payload = ctx.to_payload();

        for hook in &self.hooks {
            if !matches_event(&hook.event_pattern, &event_str, ctx.tool_name.as_deref()) {
                continue;
            }

            match run_hook_command(&hook.command, &payload) {
                Ok(0) => {
                    debug!("Hook '{}' allowed event '{}'", hook.command, event_str);
                }
                Ok(2) => {
                    let msg = format!("Hook '{}' aborted event '{}'", hook.command, event_str);
                    info!("{}", msg);
                    return HookAction::Abort(msg);
                }
                Ok(3) => {
                    info!("Hook '{}' skipped event '{}'", hook.command, event_str);
                    return HookAction::Skip;
                }
                Ok(code) => {
                    warn!("Hook '{}' returned exit code {} for '{}'", hook.command, code, event_str);
                    return HookAction::Warn(format!("Hook exit code {}", code));
                }
                Err(e) => {
                    warn!("Hook '{}' failed: {} — continuing", hook.command, e);
                }
            }
        }
        HookAction::Continue
    }

    pub fn register(&mut self, event_pattern: &str, command: &str) {
        self.hooks.push(Hook {
            event_pattern: event_pattern.to_string(),
            command: command.to_string(),
        });
    }

    pub fn has_hooks(&self) -> bool {
        !self.hooks.is_empty()
    }

    pub fn len(&self) -> usize {
        self.hooks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.hooks.is_empty()
    }

    /// Fire hooks without waiting for resolution (fire-and-forget).
    pub async fn fire(&self, ctx: &HookContext) {
        let _ = self.fire_resolve(ctx).await;
    }
}

// Also export HookRunner as alias for backward compat
pub type HookRunner = HookRegistry;

fn matches_event(pattern: &str, event: &str, tool_name: Option<&str>) -> bool {
    if pattern == "*" { return true; }
    if pattern == event { return true; }
    // "pre-shell" matches pre_tool_use when tool is "shell"
    if let Some(tool) = tool_name {
        if pattern == format!("pre-{}", tool) && event == "pre_tool_use" { return true; }
        if pattern == format!("post-{}", tool) && event == "post_tool_use" { return true; }
    }
    if pattern.ends_with('*') && event.starts_with(pattern.trim_end_matches('*')) { return true; }
    false
}

fn run_hook_command(command: &str, payload: &serde_json::Value) -> Result<i32, String> {
    let payload_str = serde_json::to_string(payload).unwrap_or_default();

    let output = std::process::Command::new("sh")
        .args(["-c", command])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("HOOK_EVENT", payload.get("event").and_then(|v| v.as_str()).unwrap_or(""))
        .env("HOOK_TOOL_NAME", payload.get("tool").and_then(|v| v.as_str()).unwrap_or(""))
        .env("HOOK_PAYLOAD", &payload_str)
        .spawn()
        .and_then(|mut child| {
            if let Some(ref mut stdin) = child.stdin {
                use std::io::Write;
                let _ = stdin.write_all(payload_str.as_bytes());
            }
            child.wait_with_output()
        })
        .map_err(|e| format!("Failed to execute hook: {}", e))?;

    Ok(output.status.code().unwrap_or(1))
}

impl Default for HookRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_matches_event_exact() {
        assert!(matches_event("on_message_received", "on_message_received", None));
        assert!(!matches_event("on_session_start", "on_message_received", None));
    }

    #[test]
    fn test_matches_event_wildcard() {
        assert!(matches_event("*", "on_message_received", None));
    }

    #[test]
    fn test_matches_event_tool_pattern() {
        assert!(matches_event("pre-shell", "pre_tool_use", Some("shell")));
        assert!(!matches_event("pre-shell", "pre_tool_use", Some("read_file")));
    }

    #[test]
    fn test_registry_empty() {
        let reg = HookRegistry { hooks: vec![] };
        let ctx = HookContext::new(HookEventType::OnMessageReceived, "test");
        let rt = tokio::runtime::Runtime::new().unwrap();
        let action = rt.block_on(reg.fire_resolve(&ctx));
        assert_eq!(action, HookAction::Continue);
    }
}
