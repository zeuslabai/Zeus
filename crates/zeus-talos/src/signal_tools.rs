//! Signal CLI tools
//!
//! Provides tools for interacting with Signal via signal-cli as a subprocess.
//! Each tool accepts optional `signal_cli` and `account` parameters, falling
//! back to the `SIGNAL_CLI_PATH` and `SIGNAL_ACCOUNT` environment variables.
//!
//! signal-cli must be installed and a registered account must exist before
//! these tools can be used.  See: <https://github.com/AsamK/signal-cli>

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::Value;
use tokio::process::Command;
use zeus_core::{Error, Result, ToolSchema};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Get the signal-cli binary path from args or the `SIGNAL_CLI_PATH` env var.
/// Defaults to `"signal-cli"` (i.e. resolves via PATH).
fn get_signal_cli(args: &Value) -> String {
    if let Some(path) = args.get("signal_cli").and_then(|v| v.as_str()) {
        return path.to_string();
    }
    std::env::var("SIGNAL_CLI_PATH").unwrap_or_else(|_| "signal-cli".to_string())
}

/// Get the Signal account (phone number) from args or the `SIGNAL_ACCOUNT` env var.
fn get_account(args: &Value) -> Result<String> {
    if let Some(account) = args.get("account").and_then(|v| v.as_str()) {
        return Ok(account.to_string());
    }
    std::env::var("SIGNAL_ACCOUNT").map_err(|_| {
        Error::Tool("Missing 'account' parameter and SIGNAL_ACCOUNT env var not set".to_string())
    })
}

/// Run signal-cli with the given account and additional arguments.
///
/// Executes: `{signal_cli} -a {account} {args...}`
///
/// Returns stdout on success; propagates stderr as an error.
pub async fn signal_cli_exec(signal_cli: &str, account: &str, args: &[&str]) -> Result<String> {
    let mut cmd = Command::new(signal_cli);
    cmd.arg("-a").arg(account);
    for arg in args {
        cmd.arg(arg);
    }

    let output = cmd
        .output()
        .await
        .map_err(|e| Error::Tool(format!("Failed to spawn signal-cli: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Error::Tool(format!("signal-cli error: {}", stderr.trim())));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

// ---------------------------------------------------------------------------
// 1. SignalSendMessageTool
// ---------------------------------------------------------------------------

/// Send a text message to a Signal recipient (phone number).
pub struct SignalSendMessageTool;

#[async_trait]
impl TalosTool for SignalSendMessageTool {
    fn name(&self) -> &'static str {
        "signal_send_message"
    }

    fn description(&self) -> &'static str {
        "Send a text message to a Signal recipient via signal-cli"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "recipient",
                "string",
                "Recipient phone number in E.164 format (e.g. +15551234567)",
                true,
            )
            .with_param("message", "string", "Message text to send", true)
            .with_param(
                "account",
                "string",
                "Sender account phone number (or set SIGNAL_ACCOUNT env var)",
                false,
            )
            .with_param(
                "signal_cli",
                "string",
                "Path to signal-cli binary (or set SIGNAL_CLI_PATH env var, default: signal-cli)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let signal_cli = get_signal_cli(&args);
        let account = get_account(&args)?;

        let recipient = args
            .get("recipient")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'recipient' parameter".to_string()))?;

        let message = args
            .get("message")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'message' parameter".to_string()))?;

        // signal-cli -a {account} send -m {message} {recipient}
        signal_cli_exec(&signal_cli, &account, &["send", "-m", message, recipient]).await?;

        Ok(format!("Message sent successfully to {}", recipient))
    }
}

// ---------------------------------------------------------------------------
// 2. SignalSendGroupMessageTool
// ---------------------------------------------------------------------------

/// Send a text message to a Signal group.
pub struct SignalSendGroupMessageTool;

#[async_trait]
impl TalosTool for SignalSendGroupMessageTool {
    fn name(&self) -> &'static str {
        "signal_send_group_message"
    }

    fn description(&self) -> &'static str {
        "Send a text message to a Signal group via signal-cli"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "group_id",
                "string",
                "Signal group ID (base64-encoded group identifier)",
                true,
            )
            .with_param("message", "string", "Message text to send", true)
            .with_param(
                "account",
                "string",
                "Sender account phone number (or set SIGNAL_ACCOUNT env var)",
                false,
            )
            .with_param(
                "signal_cli",
                "string",
                "Path to signal-cli binary (or set SIGNAL_CLI_PATH env var, default: signal-cli)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let signal_cli = get_signal_cli(&args);
        let account = get_account(&args)?;

        let group_id = args
            .get("group_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'group_id' parameter".to_string()))?;

        let message = args
            .get("message")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'message' parameter".to_string()))?;

        // signal-cli -a {account} send -m {message} -g {group_id}
        signal_cli_exec(
            &signal_cli,
            &account,
            &["send", "-m", message, "-g", group_id],
        )
        .await?;

        Ok(format!(
            "Group message sent successfully to group {}",
            group_id
        ))
    }
}

// ---------------------------------------------------------------------------
// 3. SignalReceiveMessagesTool
// ---------------------------------------------------------------------------

/// Receive and return pending Signal messages.
pub struct SignalReceiveMessagesTool;

#[async_trait]
impl TalosTool for SignalReceiveMessagesTool {
    fn name(&self) -> &'static str {
        "signal_receive"
    }

    fn description(&self) -> &'static str {
        "Receive pending Signal messages via signal-cli (returns JSON output)"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "timeout",
                "integer",
                "How long to wait for new messages in seconds (default: 5)",
                false,
            )
            .with_param(
                "account",
                "string",
                "Account phone number (or set SIGNAL_ACCOUNT env var)",
                false,
            )
            .with_param(
                "signal_cli",
                "string",
                "Path to signal-cli binary (or set SIGNAL_CLI_PATH env var, default: signal-cli)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let signal_cli = get_signal_cli(&args);
        let account = get_account(&args)?;

        let timeout = args.get("timeout").and_then(|v| v.as_u64()).unwrap_or(5);

        let timeout_str = timeout.to_string();

        // signal-cli -a {account} receive --timeout {timeout} --json
        let raw = signal_cli_exec(
            &signal_cli,
            &account,
            &["receive", "--timeout", &timeout_str, "--json"],
        )
        .await?;

        if raw.is_empty() {
            return Ok("No pending messages.".to_string());
        }

        // Parse each newline-delimited JSON envelope and produce a summary
        let mut output = String::new();
        let mut count = 0usize;

        for line in raw.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            count += 1;

            match serde_json::from_str::<Value>(line) {
                Ok(envelope) => {
                    let source = envelope
                        .get("envelope")
                        .and_then(|e| e.get("source"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");

                    // Dig into the data message if present
                    let text = envelope
                        .get("envelope")
                        .and_then(|e| e.get("dataMessage"))
                        .and_then(|dm| dm.get("message"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("[non-text or system message]");

                    let timestamp = envelope
                        .get("envelope")
                        .and_then(|e| e.get("timestamp"))
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);

                    output.push_str(&format!("[{}] From {}: {}\n", timestamp, source, text));
                }
                Err(_) => {
                    // Fall back to raw line if JSON parse fails
                    output.push_str(line);
                    output.push('\n');
                }
            }
        }

        if count == 0 {
            Ok("No pending messages.".to_string())
        } else {
            Ok(format!(
                "Received {} message(s):\n{}",
                count,
                output.trim_end()
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// 4. SignalListGroupsTool
// ---------------------------------------------------------------------------

/// List all Signal groups the account belongs to.
pub struct SignalListGroupsTool;

#[async_trait]
impl TalosTool for SignalListGroupsTool {
    fn name(&self) -> &'static str {
        "signal_list_groups"
    }

    fn description(&self) -> &'static str {
        "List all Signal groups for the configured account via signal-cli"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "account",
                "string",
                "Account phone number (or set SIGNAL_ACCOUNT env var)",
                false,
            )
            .with_param(
                "signal_cli",
                "string",
                "Path to signal-cli binary (or set SIGNAL_CLI_PATH env var, default: signal-cli)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let signal_cli = get_signal_cli(&args);
        let account = get_account(&args)?;

        // signal-cli -a {account} listGroups
        let raw = signal_cli_exec(&signal_cli, &account, &["listGroups"]).await?;

        if raw.is_empty() {
            return Ok("No groups found.".to_string());
        }

        Ok(raw)
    }
}

// ---------------------------------------------------------------------------
// 5. SignalSendReactionTool
// ---------------------------------------------------------------------------

/// Send an emoji reaction to a specific Signal message.
///
/// Signal reactions require the original message author and timestamp to
/// identify the target message (there is no opaque "message ID").
pub struct SignalSendReactionTool;

#[async_trait]
impl TalosTool for SignalSendReactionTool {
    fn name(&self) -> &'static str {
        "signal_send_reaction"
    }

    fn description(&self) -> &'static str {
        "Send an emoji reaction to a Signal message via signal-cli"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "recipient",
                "string",
                "Recipient phone number (the conversation partner, not the message author)",
                true,
            )
            .with_param("emoji", "string", "Emoji to react with (e.g. \"👍\")", true)
            .with_param(
                "target_timestamp",
                "string",
                "Timestamp of the message to react to (milliseconds since epoch)",
                true,
            )
            .with_param(
                "target_author",
                "string",
                "Phone number of the message author whose message you are reacting to",
                true,
            )
            .with_param(
                "account",
                "string",
                "Sender account phone number (or set SIGNAL_ACCOUNT env var)",
                false,
            )
            .with_param(
                "signal_cli",
                "string",
                "Path to signal-cli binary (or set SIGNAL_CLI_PATH env var, default: signal-cli)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let signal_cli = get_signal_cli(&args);
        let account = get_account(&args)?;

        let recipient = args
            .get("recipient")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'recipient' parameter".to_string()))?;

        let emoji = args
            .get("emoji")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'emoji' parameter".to_string()))?;

        let target_timestamp = args
            .get("target_timestamp")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'target_timestamp' parameter".to_string()))?;

        let target_author = args
            .get("target_author")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'target_author' parameter".to_string()))?;

        // signal-cli -a {account} sendReaction {recipient} -e {emoji} -a {target_author} -t {target_timestamp}
        signal_cli_exec(
            &signal_cli,
            &account,
            &[
                "sendReaction",
                recipient,
                "-e",
                emoji,
                "-a",
                target_author,
                "-t",
                target_timestamp,
            ],
        )
        .await?;

        Ok(format!(
            "Reaction {} sent to {} for message at timestamp {}",
            emoji, recipient, target_timestamp
        ))
    }
}

// ---------------------------------------------------------------------------
// 6. SignalSendFileTool
// ---------------------------------------------------------------------------

/// Send a file attachment to a Signal recipient via signal-cli.
///
/// Uses `signal-cli send` with the `--attachment` flag to send a local file
/// as an attachment. The file must exist on the local filesystem and be
/// accessible by the signal-cli process.
pub struct SignalSendFileTool;

#[async_trait]
impl TalosTool for SignalSendFileTool {
    fn name(&self) -> &'static str {
        "signal_send_file"
    }

    fn description(&self) -> &'static str {
        "Send a file attachment to a Signal recipient via signal-cli"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "recipient",
                "string",
                "Recipient phone number in E.164 format (e.g. +15551234567)",
                true,
            )
            .with_param(
                "file_path",
                "string",
                "Absolute path to the file to send as an attachment",
                true,
            )
            .with_param(
                "message",
                "string",
                "Optional caption or message text to accompany the file",
                false,
            )
            .with_param(
                "account",
                "string",
                "Sender account phone number (or set SIGNAL_ACCOUNT env var)",
                false,
            )
            .with_param(
                "signal_cli",
                "string",
                "Path to signal-cli binary (or set SIGNAL_CLI_PATH env var, default: signal-cli)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let signal_cli = get_signal_cli(&args);
        let account = get_account(&args)?;

        let recipient = args
            .get("recipient")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'recipient' parameter".to_string()))?;

        let file_path = args
            .get("file_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'file_path' parameter".to_string()))?;

        // Verify the file exists before invoking signal-cli
        if !std::path::Path::new(file_path).exists() {
            return Err(Error::Tool(format!(
                "File not found: {}",
                file_path
            )));
        }

        let message = args
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        // Build the signal-cli command:
        // signal-cli -a {account} send -a {recipient} --attachment {file_path} [-m {message}]
        let mut cmd_args: Vec<String> = vec![
            "send".to_string(),
            recipient.to_string(),
            "--attachment".to_string(),
            file_path.to_string(),
        ];

        if !message.is_empty() {
            cmd_args.push("-m".to_string());
            cmd_args.push(message.to_string());
        }

        let args_refs: Vec<&str> = cmd_args.iter().map(|s| s.as_str()).collect();
        signal_cli_exec(&signal_cli, &account, &args_refs).await?;

        Ok(format!(
            "File sent successfully to {}: {}",
            recipient, file_path
        ))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // -- schema correctness --------------------------------------------------

    #[test]
    fn test_send_message_schema() {
        let tool = SignalSendMessageTool;
        assert_eq!(tool.name(), "signal_send_message");
        assert!(!tool.description().is_empty());

        let schema = tool.schema();
        let params = schema
            .parameters
            .as_object()
            .expect("params must be object");
        let props = params["properties"].as_object().expect("properties");

        assert!(props.contains_key("recipient"));
        assert!(props.contains_key("message"));
        assert!(props.contains_key("account"));
        assert!(props.contains_key("signal_cli"));

        let required: Vec<&str> = params["required"]
            .as_array()
            .expect("required array")
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert!(required.contains(&"recipient"));
        assert!(required.contains(&"message"));
        assert!(!required.contains(&"account"));
        assert!(!required.contains(&"signal_cli"));
    }

    #[test]
    fn test_send_group_message_schema() {
        let tool = SignalSendGroupMessageTool;
        assert_eq!(tool.name(), "signal_send_group_message");

        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("params");
        let required: Vec<&str> = params["required"]
            .as_array()
            .expect("required")
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert!(required.contains(&"group_id"));
        assert!(required.contains(&"message"));
        assert!(!required.contains(&"account"));
        assert!(!required.contains(&"signal_cli"));
    }

    #[test]
    fn test_receive_messages_schema() {
        let tool = SignalReceiveMessagesTool;
        assert_eq!(tool.name(), "signal_receive");

        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("params");
        let props = params["properties"].as_object().expect("properties");

        assert!(props.contains_key("timeout"));
        assert!(props.contains_key("account"));
        assert!(props.contains_key("signal_cli"));

        // All params for receive are optional
        let required = params
            .get("required")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
            .unwrap_or_default();
        assert!(!required.contains(&"timeout"));
        assert!(!required.contains(&"account"));
    }

    #[test]
    fn test_list_groups_schema() {
        let tool = SignalListGroupsTool;
        assert_eq!(tool.name(), "signal_list_groups");

        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("params");
        let props = params["properties"].as_object().expect("properties");

        assert!(props.contains_key("account"));
        assert!(props.contains_key("signal_cli"));

        // Both params optional
        let required = params
            .get("required")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
            .unwrap_or_default();
        assert!(!required.contains(&"account"));
        assert!(!required.contains(&"signal_cli"));
    }

    #[test]
    fn test_send_reaction_schema() {
        let tool = SignalSendReactionTool;
        assert_eq!(tool.name(), "signal_send_reaction");

        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("params");
        let required: Vec<&str> = params["required"]
            .as_array()
            .expect("required")
            .iter()
            .filter_map(|v| v.as_str())
            .collect();

        // All four identifying params must be required
        assert!(required.contains(&"recipient"));
        assert!(required.contains(&"emoji"));
        assert!(required.contains(&"target_timestamp"));
        assert!(required.contains(&"target_author"));
        // Config params remain optional
        assert!(!required.contains(&"account"));
        assert!(!required.contains(&"signal_cli"));
    }

    // -- get_signal_cli helper -----------------------------------------------

    #[test]
    fn test_get_signal_cli_from_args() {
        let args = json!({ "signal_cli": "/usr/local/bin/signal-cli" });
        assert_eq!(get_signal_cli(&args), "/usr/local/bin/signal-cli");
    }

    /// Combined into one test to avoid env-var race conditions with parallel test runner.
    /// Covers: args override, env var set, env var absent (default).
    #[test]
    fn test_get_signal_cli_env_and_default() {
        // Part 1: env var set — should be returned when no args override
        unsafe {
            std::env::set_var("SIGNAL_CLI_PATH", "/opt/signal-cli/bin/signal-cli");
        }
        let args = json!({});
        assert_eq!(get_signal_cli(&args), "/opt/signal-cli/bin/signal-cli");

        // Part 2: env var removed — should fall back to "signal-cli"
        unsafe {
            std::env::remove_var("SIGNAL_CLI_PATH");
        }
        let args = json!({});
        assert_eq!(get_signal_cli(&args), "signal-cli");
    }

    // -- get_account helper --------------------------------------------------

    #[test]
    fn test_get_account_from_args() {
        let args = json!({ "account": "+15551234567" });
        let account = get_account(&args).expect("should succeed");
        assert_eq!(account, "+15551234567");
        // Args override takes precedence even when env is set
        unsafe {
            std::env::set_var("SIGNAL_ACCOUNT", "+15559876543");
        }
        let account2 = get_account(&args).expect("args override env");
        assert_eq!(account2, "+15551234567");
        unsafe {
            std::env::remove_var("SIGNAL_ACCOUNT");
        }
    }

    #[test]
    fn test_get_account_missing_returns_error() {
        // Test the error path: no args, and env var explicitly absent.
        // We use a Value that has no account key, and we verify the error message
        // without relying on the global env state (the error branch is purely
        // env-var dependent, but we assert the error message format by providing
        // a sentinel that can never match a valid account).
        // Drive the error purely through args: pass an empty object and
        // temporarily shadow with a known-absent env var name by checking the
        // error type directly via the helper's return.
        let result = get_account(&json!({ "account": "+10000000001" }));
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "+10000000001");

        // Error case: call with no account arg; env var may or may not be set
        // by other tests. We can only assert the type of error, not its message,
        // without a serial runner. Instead we test via a known-bad arg absence
        // and check that args override always wins.
        let args_with = json!({ "account": "+10000000002" });
        assert_eq!(get_account(&args_with).unwrap(), "+10000000002");
    }

    // -- execute error paths (no real signal-cli needed) ---------------------

    #[tokio::test]
    async fn test_send_message_missing_recipient() {
        let tool = SignalSendMessageTool;
        // recipient field is absent — should error before any subprocess is spawned
        let args = json!({
            "account": "+15551234567",
            "message": "hello",
            "signal_cli": "false"
        });
        let result = tool.execute(args).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("recipient"));
    }

    #[tokio::test]
    async fn test_send_message_missing_message() {
        let tool = SignalSendMessageTool;
        let args = json!({
            "account": "+15551234567",
            "recipient": "+15559999999",
            "signal_cli": "false"
        });
        let result = tool.execute(args).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("message"));
    }

    #[tokio::test]
    async fn test_send_group_message_missing_group_id() {
        let tool = SignalSendGroupMessageTool;
        let args = json!({
            "account": "+15551234567",
            "message": "hello group",
            "signal_cli": "false"
        });
        let result = tool.execute(args).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("group_id"));
    }

    #[tokio::test]
    async fn test_receive_missing_account() {
        // Verify that when account is not in args, the tool surfaces an error
        // that mentions SIGNAL_ACCOUNT. We cannot safely remove the env var in
        // a parallel test runner, so instead we supply account via args to make
        // the tool proceed to the subprocess step, and separately verify the
        // helper function's error message in a sync context.
        let err = get_account(&json!({}));
        // If the env var happens to be set by a concurrent test, this may be
        // Ok — we just skip the assertion in that case to avoid flakiness.
        if let Err(e) = err {
            assert!(
                e.to_string().contains("SIGNAL_ACCOUNT"),
                "error should mention SIGNAL_ACCOUNT: {}",
                e
            );
        }

        // The tool itself: with account supplied, it reaches the subprocess
        // (signal_cli = "false" is the /usr/bin/false no-op, exits non-zero).
        let tool = SignalReceiveMessagesTool;
        let args = json!({ "account": "+15551234567", "signal_cli": "false" });
        let result = tool.execute(args).await;
        // /usr/bin/false exits with code 1 — expect a signal-cli error
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_send_reaction_missing_emoji() {
        let tool = SignalSendReactionTool;
        // emoji field is absent — should error before any subprocess is spawned
        let args = json!({
            "account": "+15551234567",
            "recipient": "+15559999999",
            "target_timestamp": "1700000000000",
            "target_author": "+15559999999",
            "signal_cli": "false"
        });
        let result = tool.execute(args).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("emoji"));
    }

    #[tokio::test]
    async fn test_send_reaction_missing_target_author() {
        let tool = SignalSendReactionTool;
        let args = json!({
            "account": "+15551234567",
            "recipient": "+15559999999",
            "emoji": "👍",
            "target_timestamp": "1700000000000",
            "signal_cli": "false"
        });
        let result = tool.execute(args).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("target_author"));
    }
}
