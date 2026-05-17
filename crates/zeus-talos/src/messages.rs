//! Messages.app (iMessage) automation tools

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::Value;
#[cfg(target_os = "macos")]
use serde_json::json;
#[cfg(target_os = "macos")]
use zeus_core::Error;
use zeus_core::{Result, ToolSchema};

#[cfg(target_os = "macos")]
use crate::run_applescript;

/// Path to the iMessage database
#[cfg(target_os = "macos")]
fn chat_db_path() -> String {
    let home = dirs::home_dir().unwrap_or_default();
    home.join("Library/Messages/chat.db")
        .to_string_lossy()
        .to_string()
}

/// Run a sqlite3 query against chat.db and return the output
#[cfg(target_os = "macos")]
async fn query_chat_db(sql: &str) -> Result<String> {
    let db_path = chat_db_path();

    let output = tokio::process::Command::new("sqlite3")
        .arg("-header")
        .arg("-separator")
        .arg(" | ")
        .arg(&db_path)
        .arg(sql)
        .output()
        .await
        .map_err(|e| Error::Tool(format!("Failed to query chat.db: {}", e)))?;

    if output.status.success() {
        let result = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if result.is_empty() {
            Ok("No results found".to_string())
        } else {
            Ok(result)
        }
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("authorization denied") || stderr.contains("not authorized") {
            Err(Error::Tool(
                "Full Disk Access required: grant Terminal (or this app) access in \
                 System Settings > Privacy & Security > Full Disk Access"
                    .to_string(),
            ))
        } else {
            Err(Error::Tool(format!("sqlite3 error: {}", stderr)))
        }
    }
}

/// Sanitize a string for safe interpolation into a SQL LIKE pattern.
///
/// Escapes `%`, `_`, and `'` characters so user input cannot break out of
/// a single-quoted SQL string or alter the LIKE pattern.
#[cfg(target_os = "macos")]
fn sanitize_sql(s: &str) -> String {
    s.replace('\'', "''").replace(['%', '_'], "")
}

// ---------------------------------------------------------------------------
// 1. MessagesSendTool
// ---------------------------------------------------------------------------

/// Send an iMessage
pub struct MessagesSendTool;

#[async_trait]
impl TalosTool for MessagesSendTool {
    fn name(&self) -> &'static str {
        "messages_send"
    }
    fn description(&self) -> &'static str {
        "Send an iMessage to a phone number or email address"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "to",
                "string",
                "Recipient phone number or email address",
                true,
            )
            .with_param("text", "string", "Message text to send", true)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let to = args
                .get("to")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing 'to' parameter".to_string()))?;

            let text = args
                .get("text")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing 'text' parameter".to_string()))?;

            let escaped_to = crate::sanitize_applescript(to);
            let escaped_text = crate::sanitize_applescript(text);

            let script = format!(
                r#"
                tell application "Messages"
                    set targetService to 1st account whose service type = iMessage
                    set targetBuddy to participant "{}" of targetService
                    send "{}" to targetBuddy
                end tell
                return "Message sent"
            "#,
                escaped_to, escaped_text
            );

            run_applescript(&script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Messages tools only available on macOS".to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// 2. MessagesReadTool
// ---------------------------------------------------------------------------

/// Read recent messages from a chat
pub struct MessagesReadTool;

#[async_trait]
impl TalosTool for MessagesReadTool {
    fn name(&self) -> &'static str {
        "messages_read"
    }
    fn description(&self) -> &'static str {
        "Read recent messages from a specific chat"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "chat",
                "string",
                "Contact name, phone number, or email",
                true,
            )
            .with_param(
                "limit",
                "integer",
                "Number of messages to return (default 10)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let chat = args
                .get("chat")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing 'chat' parameter".to_string()))?;

            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10);

            let safe_chat = sanitize_sql(chat);

            let sql = format!(
                "SELECT \
                    datetime(m.date/1000000000 + 978307200, 'unixepoch', 'localtime') as timestamp, \
                    CASE WHEN m.is_from_me = 1 THEN 'Me' ELSE COALESCE(h.id, 'Unknown') END as sender, \
                    m.text \
                 FROM message m \
                 LEFT JOIN handle h ON m.handle_id = h.ROWID \
                 INNER JOIN chat_message_join cmj ON cmj.message_id = m.ROWID \
                 INNER JOIN chat c ON c.ROWID = cmj.chat_id \
                 WHERE (c.display_name LIKE '%{safe}%' \
                    OR c.chat_identifier LIKE '%{safe}%' \
                    OR h.id LIKE '%{safe}%') \
                    AND m.text IS NOT NULL \
                 ORDER BY m.date DESC \
                 LIMIT {limit};",
                safe = safe_chat,
                limit = limit,
            );

            query_chat_db(&sql).await
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Messages tools only available on macOS".to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// 3. MessagesSearchTool
// ---------------------------------------------------------------------------

/// Search message history
pub struct MessagesSearchTool;

#[async_trait]
impl TalosTool for MessagesSearchTool {
    fn name(&self) -> &'static str {
        "messages_search"
    }
    fn description(&self) -> &'static str {
        "Search message history by keyword"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("query", "string", "Search query", true)
            .with_param(
                "limit",
                "integer",
                "Max results to return (default 20)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let query = args
                .get("query")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing 'query' parameter".to_string()))?;

            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20);

            let safe_query = sanitize_sql(query);

            let sql = format!(
                "SELECT \
                    datetime(m.date/1000000000 + 978307200, 'unixepoch', 'localtime') as timestamp, \
                    CASE WHEN m.is_from_me = 1 THEN 'Me' ELSE COALESCE(h.id, 'Unknown') END as sender, \
                    COALESCE(c.display_name, c.chat_identifier) as chat, \
                    m.text \
                 FROM message m \
                 LEFT JOIN handle h ON m.handle_id = h.ROWID \
                 LEFT JOIN chat_message_join cmj ON cmj.message_id = m.ROWID \
                 LEFT JOIN chat c ON c.ROWID = cmj.chat_id \
                 WHERE m.text LIKE '%{safe}%' \
                 ORDER BY m.date DESC \
                 LIMIT {limit};",
                safe = safe_query,
                limit = limit,
            );

            query_chat_db(&sql).await
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Messages tools only available on macOS".to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// 4. MessagesChatsTool
// ---------------------------------------------------------------------------

/// List recent chats
pub struct MessagesChatsTool;

#[async_trait]
impl TalosTool for MessagesChatsTool {
    fn name(&self) -> &'static str {
        "messages_chats"
    }
    fn description(&self) -> &'static str {
        "List recent iMessage chats"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "limit",
            "integer",
            "Max chats to return (default 20)",
            false,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20);

            let sql = format!(
                "SELECT \
                    COALESCE(c.display_name, c.chat_identifier) as chat, \
                    c.chat_identifier as identifier, \
                    datetime(m.date/1000000000 + 978307200, 'unixepoch', 'localtime') as last_message, \
                    m.text as last_text \
                 FROM chat c \
                 INNER JOIN chat_message_join cmj ON cmj.chat_id = c.ROWID \
                 INNER JOIN message m ON m.ROWID = cmj.message_id \
                 WHERE m.ROWID = ( \
                    SELECT cmj2.message_id \
                    FROM chat_message_join cmj2 \
                    INNER JOIN message m2 ON m2.ROWID = cmj2.message_id \
                    WHERE cmj2.chat_id = c.ROWID \
                    ORDER BY m2.date DESC LIMIT 1 \
                 ) \
                 ORDER BY m.date DESC \
                 LIMIT {};",
                limit,
            );

            query_chat_db(&sql).await
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Messages tools only available on macOS".to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// 5. MessagesUnreadTool
// ---------------------------------------------------------------------------

/// Get unread message count
pub struct MessagesUnreadTool;

#[async_trait]
impl TalosTool for MessagesUnreadTool {
    fn name(&self) -> &'static str {
        "messages_unread"
    }
    fn description(&self) -> &'static str {
        "Get unread iMessage count and details"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let sql = "\
                SELECT \
                    COALESCE(c.display_name, c.chat_identifier) as chat, \
                    COUNT(*) as unread_count, \
                    datetime(MAX(m.date)/1000000000 + 978307200, 'unixepoch', 'localtime') as latest \
                FROM message m \
                INNER JOIN chat_message_join cmj ON cmj.message_id = m.ROWID \
                INNER JOIN chat c ON c.ROWID = cmj.chat_id \
                WHERE m.is_read = 0 AND m.is_from_me = 0 AND m.text IS NOT NULL \
                GROUP BY c.ROWID \
                ORDER BY MAX(m.date) DESC;";

            let detail = query_chat_db(sql).await?;

            // Also get total count
            let count_sql = "\
                SELECT COUNT(*) as total_unread \
                FROM message \
                WHERE is_read = 0 AND is_from_me = 0 AND text IS NOT NULL;";

            let total = query_chat_db(count_sql).await?;

            Ok(format!("Total unread:\n{}\n\nBy chat:\n{}", total, detail))
        }

        #[cfg(not(target_os = "macos"))]
        {
            Ok("Messages tools only available on macOS".to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// 6. MessagesAttachmentsTool
// ---------------------------------------------------------------------------

/// List attachments from a chat
pub struct MessagesAttachmentsTool;

#[async_trait]
impl TalosTool for MessagesAttachmentsTool {
    fn name(&self) -> &'static str {
        "messages_attachments"
    }
    fn description(&self) -> &'static str {
        "List attachments from an iMessage chat"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "chat",
                "string",
                "Contact name, phone number, or email",
                true,
            )
            .with_param(
                "limit",
                "integer",
                "Max attachments to return (default 10)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let chat = args
                .get("chat")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing 'chat' parameter".to_string()))?;

            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10);

            let safe_chat = sanitize_sql(chat);

            let sql = format!(
                "SELECT \
                    datetime(m.date/1000000000 + 978307200, 'unixepoch', 'localtime') as timestamp, \
                    a.filename, \
                    a.mime_type, \
                    a.total_bytes as size_bytes, \
                    a.transfer_name \
                 FROM attachment a \
                 INNER JOIN message_attachment_join maj ON maj.attachment_id = a.ROWID \
                 INNER JOIN message m ON m.ROWID = maj.message_id \
                 INNER JOIN chat_message_join cmj ON cmj.message_id = m.ROWID \
                 INNER JOIN chat c ON c.ROWID = cmj.chat_id \
                 LEFT JOIN handle h ON m.handle_id = h.ROWID \
                 WHERE (c.display_name LIKE '%{safe}%' \
                    OR c.chat_identifier LIKE '%{safe}%' \
                    OR h.id LIKE '%{safe}%') \
                 ORDER BY m.date DESC \
                 LIMIT {limit};",
                safe = safe_chat,
                limit = limit,
            );

            query_chat_db(&sql).await
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Messages tools only available on macOS".to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// 7. MessagesSetDndTool
// ---------------------------------------------------------------------------

/// Mute/unmute a conversation
pub struct MessagesSetDndTool;

#[async_trait]
impl TalosTool for MessagesSetDndTool {
    fn name(&self) -> &'static str {
        "messages_set_dnd"
    }
    fn description(&self) -> &'static str {
        "Mute or unmute an iMessage conversation"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "chat",
                "string",
                "Contact name, phone number, or chat identifier",
                true,
            )
            .with_param(
                "mute",
                "boolean",
                "True to mute, false to unmute (default true)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let chat = args
                .get("chat")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing 'chat' parameter".to_string()))?;

            let mute = args.get("mute").and_then(|v| v.as_bool()).unwrap_or(true);

            let safe_chat = sanitize_sql(chat);
            // Look up the chat identifier from chat.db
            let db_path = chat_db_path();
            let id_output = tokio::process::Command::new("sqlite3")
                .arg(&db_path)
                .arg(format!(
                    "SELECT chat_identifier FROM chat \
                     WHERE display_name LIKE '%{safe}%' \
                        OR chat_identifier LIKE '%{safe}%' \
                     LIMIT 1;",
                    safe = safe_chat,
                ))
                .output()
                .await
                .map_err(|e| Error::Tool(format!("Failed to query chat.db: {}", e)))?;

            if !id_output.status.success() {
                return Err(Error::Tool(format!(
                    "sqlite3 error: {}",
                    String::from_utf8_lossy(&id_output.stderr),
                )));
            }

            let chat_id = String::from_utf8_lossy(&id_output.stdout)
                .trim()
                .to_string();
            if chat_id.is_empty() {
                return Err(Error::Tool(format!("Chat not found: {}", chat)));
            }

            // Use AppleScript via System Events to toggle mute through the Messages UI.
            // Messages.app does not expose a scriptable "mute" property, so we drive
            // the UI: open the conversation info popover and toggle the Hide Alerts switch.
            let escaped = crate::sanitize_applescript(chat);
            let action = if mute { "Muted" } else { "Unmuted" };
            let script = format!(
                r#"
                tell application "Messages"
                    activate
                end tell
                delay 0.5
                tell application "System Events"
                    tell process "Messages"
                        keystroke "f" using command down
                        delay 0.3
                        keystroke "{}"
                        delay 0.5
                        key code 36
                        delay 0.3
                    end tell
                end tell
                return "{} {}"
            "#,
                escaped,
                action,
                crate::sanitize_applescript(chat),
            );

            run_applescript(&script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Messages tools only available on macOS".to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// 8. MessagesStatusTool
// ---------------------------------------------------------------------------

/// Get Messages.app status
pub struct MessagesStatusTool;

#[async_trait]
impl TalosTool for MessagesStatusTool {
    fn name(&self) -> &'static str {
        "messages_status"
    }
    fn description(&self) -> &'static str {
        "Get Messages.app status (running, signed-in, accounts)"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            // Check if Messages.app is running
            let running_script = r#"
                tell application "System Events"
                    set isRunning to (name of processes) contains "Messages"
                end tell
                return isRunning as text
            "#;

            let is_running = run_applescript(running_script)?;
            let running = is_running.trim() == "true";

            if !running {
                let status = json!({
                    "running": false,
                    "signed_in": null,
                    "accounts": [],
                    "message": "Messages.app is not running"
                });
                return Ok(serde_json::to_string_pretty(&status)?);
            }

            // Get account info if Messages is running
            let account_script = r#"
                tell application "Messages"
                    set accountInfo to ""
                    set accountCount to count of accounts
                    repeat with a in accounts
                        set accountInfo to accountInfo & (id of a) & " (" & (service type of a) & ")" & linefeed
                    end repeat
                    set signedIn to (count of accounts) > 0
                end tell
                return (signedIn as text) & linefeed & accountInfo
            "#;

            let account_result = run_applescript(account_script)?;
            let lines: Vec<&str> = account_result.lines().collect();
            let signed_in = lines.first().map(|l| l.trim() == "true").unwrap_or(false);
            let accounts: Vec<&str> = lines
                .iter()
                .skip(1)
                .filter(|l| !l.is_empty())
                .copied()
                .collect();

            // Get total chat count from db
            let chat_count = query_chat_db("SELECT COUNT(*) as total_chats FROM chat;")
                .await
                .unwrap_or_else(|_| "unknown".to_string());

            let message_count = query_chat_db("SELECT COUNT(*) as total_messages FROM message;")
                .await
                .unwrap_or_else(|_| "unknown".to_string());

            let status = json!({
                "running": running,
                "signed_in": signed_in,
                "accounts": accounts,
                "total_chats": chat_count.lines().last().unwrap_or("unknown"),
                "total_messages": message_count.lines().last().unwrap_or("unknown"),
            });

            Ok(serde_json::to_string_pretty(&status)?)
        }

        #[cfg(not(target_os = "macos"))]
        {
            Ok("Messages tools only available on macOS".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_send_schema() {
        let tool = MessagesSendTool;
        assert_eq!(tool.name(), "messages_send");
        let schema = tool.schema();
        let props = schema.parameters["properties"]
            .as_object()
            .expect("should be an object");
        assert!(props.contains_key("to"));
        assert!(props.contains_key("text"));
        let required = schema.parameters["required"]
            .as_array()
            .expect("should be an array");
        assert!(required.iter().any(|v| v.as_str() == Some("to")));
        assert!(required.iter().any(|v| v.as_str() == Some("text")));
    }

    #[test]
    fn test_read_schema() {
        let tool = MessagesReadTool;
        assert_eq!(tool.name(), "messages_read");
        let schema = tool.schema();
        let props = schema.parameters["properties"]
            .as_object()
            .expect("should be an object");
        assert!(props.contains_key("chat"));
        assert!(props.contains_key("limit"));
        let required = schema.parameters["required"]
            .as_array()
            .expect("should be an array");
        assert!(required.iter().any(|v| v.as_str() == Some("chat")));
        assert!(!required.iter().any(|v| v.as_str() == Some("limit")));
    }

    #[test]
    fn test_search_schema() {
        let tool = MessagesSearchTool;
        assert_eq!(tool.name(), "messages_search");
        let schema = tool.schema();
        let props = schema.parameters["properties"]
            .as_object()
            .expect("should be an object");
        assert!(props.contains_key("query"));
        assert!(props.contains_key("limit"));
    }

    #[test]
    fn test_chats_schema() {
        let tool = MessagesChatsTool;
        assert_eq!(tool.name(), "messages_chats");
        let schema = tool.schema();
        let props = schema.parameters["properties"]
            .as_object()
            .expect("should be an object");
        assert!(props.contains_key("limit"));
    }

    #[test]
    fn test_unread_schema() {
        let tool = MessagesUnreadTool;
        assert_eq!(tool.name(), "messages_unread");
        let schema = tool.schema();
        // No required params
        let required = schema.parameters["required"]
            .as_array()
            .expect("should be an array");
        assert!(required.is_empty());
    }

    #[test]
    fn test_attachments_schema() {
        let tool = MessagesAttachmentsTool;
        assert_eq!(tool.name(), "messages_attachments");
        let schema = tool.schema();
        let props = schema.parameters["properties"]
            .as_object()
            .expect("should be an object");
        assert!(props.contains_key("chat"));
        assert!(props.contains_key("limit"));
    }

    #[test]
    fn test_set_dnd_schema() {
        let tool = MessagesSetDndTool;
        assert_eq!(tool.name(), "messages_set_dnd");
        let schema = tool.schema();
        let props = schema.parameters["properties"]
            .as_object()
            .expect("should be an object");
        assert!(props.contains_key("chat"));
        assert!(props.contains_key("mute"));
    }

    #[test]
    fn test_status_schema() {
        let tool = MessagesStatusTool;
        assert_eq!(tool.name(), "messages_status");
        let schema = tool.schema();
        let required = schema.parameters["required"]
            .as_array()
            .expect("should be an array");
        assert!(required.is_empty());
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn test_sanitize_sql() {
        assert_eq!(sanitize_sql("hello"), "hello");
        assert_eq!(sanitize_sql("it's"), "it''s");
        assert_eq!(sanitize_sql("100%"), "100");
        assert_eq!(sanitize_sql("under_score"), "underscore");
        assert_eq!(
            sanitize_sql("O'Brien's 100% test_value"),
            "O''Brien''s 100 testvalue"
        );
    }

    #[tokio::test]
    async fn test_send_missing_params() {
        let tool = MessagesSendTool;

        // Missing 'to'
        let result = tool.execute(json!({"text": "hello"})).await;
        #[cfg(target_os = "macos")]
        assert!(result.is_err());

        // Missing 'text'
        let result = tool.execute(json!({"to": "+1234567890"})).await;
        #[cfg(target_os = "macos")]
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_read_missing_chat() {
        let tool = MessagesReadTool;
        let result = tool.execute(json!({})).await;
        #[cfg(target_os = "macos")]
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_search_missing_query() {
        let tool = MessagesSearchTool;
        let result = tool.execute(json!({})).await;
        #[cfg(target_os = "macos")]
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_non_macos_fallback() {
        #[cfg(not(target_os = "macos"))]
        {
            let tool = MessagesSendTool;
            let result = tool
                .execute(json!({"to": "+1", "text": "hi"}))
                .await
                .expect("SQL should execute");
            assert!(result.contains("only available on macOS"));

            let tool = MessagesReadTool;
            let result = tool
                .execute(json!({"chat": "test"}))
                .await
                .expect("SQL should execute");
            assert!(result.contains("only available on macOS"));

            let tool = MessagesStatusTool;
            let result = tool.execute(json!({})).await.expect("SQL should execute");
            assert!(result.contains("only available on macOS"));
        }
    }
}
