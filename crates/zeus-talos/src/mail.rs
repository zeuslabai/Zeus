//! Apple Mail automation tools

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::Value;
#[cfg(target_os = "macos")]
use zeus_core::Error;
use zeus_core::{Result, ToolSchema};

#[cfg(target_os = "macos")]
use crate::run_applescript;

/// Compose and send an email via Apple Mail
pub struct MailSendTool;

#[async_trait]
impl TalosTool for MailSendTool {
    fn name(&self) -> &'static str {
        "mail_send"
    }
    fn description(&self) -> &'static str {
        "Compose and send an email via Apple Mail"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("to", "string", "Recipient email address", true)
            .with_param("subject", "string", "Email subject line", true)
            .with_param("body", "string", "Email body content", true)
            .with_param(
                "cc",
                "string",
                "CC recipient email address (optional)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let to = args
                .get("to")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing to".to_string()))?;

            let subject = args
                .get("subject")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing subject".to_string()))?;

            let body = args
                .get("body")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing body".to_string()))?;

            let cc = args.get("cc").and_then(|v| v.as_str());

            let cc_block = if let Some(cc_addr) = cc {
                format!(
                    r#"make new to recipient at end of cc recipients with properties {{address:"{}"}}"#,
                    crate::sanitize_applescript(cc_addr)
                )
            } else {
                String::new()
            };

            let script = format!(
                r#"
                tell application "Mail"
                    set newMessage to make new outgoing message with properties {{subject:"{}", content:"{}", visible:true}}
                    tell newMessage
                        make new to recipient at end of to recipients with properties {{address:"{}"}}
                        {}
                    end tell
                    send newMessage
                end tell
                return "Email sent"
            "#,
                crate::sanitize_applescript(subject),
                crate::sanitize_applescript(body),
                crate::sanitize_applescript(to),
                cc_block,
            );

            run_applescript(&script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Mail tools only available on macOS".to_string())
        }
    }
}

/// List recent inbox messages
pub struct MailInboxTool;

#[async_trait]
impl TalosTool for MailInboxTool {
    fn name(&self) -> &'static str {
        "mail_inbox"
    }
    fn description(&self) -> &'static str {
        "List recent messages from the inbox"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "limit",
            "integer",
            "Max messages to return (default 10)",
            false,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10);

            let script = format!(
                r#"
                set msgList to ""
                tell application "Mail"
                    set inboxMessages to messages of inbox
                    set msgCount to 0
                    repeat with m in inboxMessages
                        if msgCount >= {} then exit repeat
                        set msgList to msgList & (msgCount + 1) & ". " & (subject of m) & " | From: " & (sender of m) & " | Date: " & (date received of m as string) & linefeed
                        set msgCount to msgCount + 1
                    end repeat
                end tell
                return msgList
            "#,
                limit
            );

            run_applescript(&script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Mail tools only available on macOS".to_string())
        }
    }
}

/// Read the content of an email by index
pub struct MailReadTool;

#[async_trait]
impl TalosTool for MailReadTool {
    fn name(&self) -> &'static str {
        "mail_read"
    }
    fn description(&self) -> &'static str {
        "Read the content of an email message by index"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "id",
            "integer",
            "Message index (1-based) in the inbox",
            true,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let id = args
                .get("id")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| Error::Tool("Missing id".to_string()))?;

            let script = format!(
                r#"
                tell application "Mail"
                    set m to message {} of inbox
                    set msgInfo to "Subject: " & (subject of m) & linefeed
                    set msgInfo to msgInfo & "From: " & (sender of m) & linefeed
                    set msgInfo to msgInfo & "Date: " & (date received of m as string) & linefeed
                    set msgInfo to msgInfo & "Read: " & (read status of m as string) & linefeed
                    set msgInfo to msgInfo & linefeed & (content of m)
                    return msgInfo
                end tell
            "#,
                id
            );

            run_applescript(&script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Mail tools only available on macOS".to_string())
        }
    }
}

/// Search emails by query
pub struct MailSearchTool;

#[async_trait]
impl TalosTool for MailSearchTool {
    fn name(&self) -> &'static str {
        "mail_search"
    }
    fn description(&self) -> &'static str {
        "Search emails by subject or content"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("query", "string", "Search query string", true)
            .with_param(
                "mailbox",
                "string",
                "Mailbox name to search in (optional, defaults to inbox)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let query = args
                .get("query")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing query".to_string()))?;

            let mailbox = args.get("mailbox").and_then(|v| v.as_str());
            let escaped = crate::sanitize_applescript(query);

            let mailbox_ref = if let Some(mb) = mailbox {
                format!(
                    r#"mailbox "{}" of account 1"#,
                    crate::sanitize_applescript(mb)
                )
            } else {
                "inbox".to_string()
            };

            let script = format!(
                r#"
                set resultList to ""
                set matchCount to 0
                tell application "Mail"
                    set allMessages to messages of {}
                    repeat with m in allMessages
                        if matchCount >= 20 then exit repeat
                        if (subject of m) contains "{}" or (content of m) contains "{}" then
                            set matchCount to matchCount + 1
                            set resultList to resultList & matchCount & ". " & (subject of m) & " | From: " & (sender of m) & " | Date: " & (date received of m as string) & linefeed
                        end if
                    end repeat
                end tell
                if matchCount is 0 then
                    return "No messages found matching the query"
                end if
                return resultList
            "#,
                mailbox_ref, escaped, escaped
            );

            run_applescript(&script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Mail tools only available on macOS".to_string())
        }
    }
}

/// Flag or unflag an email
pub struct MailFlagTool;

#[async_trait]
impl TalosTool for MailFlagTool {
    fn name(&self) -> &'static str {
        "mail_flag"
    }
    fn description(&self) -> &'static str {
        "Flag or unflag an email message"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "id",
                "integer",
                "Message index (1-based) in the inbox",
                true,
            )
            .with_param("flagged", "boolean", "True to flag, false to unflag", true)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let id = args
                .get("id")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| Error::Tool("Missing id".to_string()))?;

            let flagged = args
                .get("flagged")
                .and_then(|v| v.as_bool())
                .ok_or_else(|| Error::Tool("Missing flagged".to_string()))?;

            let script = format!(
                r#"
                tell application "Mail"
                    set flagged status of message {} of inbox to {}
                end tell
                return "Message {} {}"
            "#,
                id,
                flagged,
                id,
                if flagged { "flagged" } else { "unflagged" }
            );

            run_applescript(&script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Mail tools only available on macOS".to_string())
        }
    }
}

/// Move an email to a different mailbox
pub struct MailMoveTool;

#[async_trait]
impl TalosTool for MailMoveTool {
    fn name(&self) -> &'static str {
        "mail_move"
    }
    fn description(&self) -> &'static str {
        "Move an email message to a different mailbox"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "id",
                "integer",
                "Message index (1-based) in the inbox",
                true,
            )
            .with_param("mailbox", "string", "Destination mailbox name", true)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let id = args
                .get("id")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| Error::Tool("Missing id".to_string()))?;

            let mailbox = args
                .get("mailbox")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing mailbox".to_string()))?;

            let script = format!(
                r#"
                tell application "Mail"
                    set targetMailbox to mailbox "{}" of account 1
                    move message {} of inbox to targetMailbox
                end tell
                return "Message moved to {}"
            "#,
                crate::sanitize_applescript(mailbox),
                id,
                crate::sanitize_applescript(mailbox)
            );

            run_applescript(&script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Mail tools only available on macOS".to_string())
        }
    }
}

/// Delete an email message
pub struct MailDeleteTool;

#[async_trait]
impl TalosTool for MailDeleteTool {
    fn name(&self) -> &'static str {
        "mail_delete"
    }
    fn description(&self) -> &'static str {
        "Delete an email message"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "id",
            "integer",
            "Message index (1-based) in the inbox",
            true,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let id = args
                .get("id")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| Error::Tool("Missing id".to_string()))?;

            let script = format!(
                r#"
                tell application "Mail"
                    delete message {} of inbox
                end tell
                return "Message deleted"
            "#,
                id
            );

            run_applescript(&script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Mail tools only available on macOS".to_string())
        }
    }
}

/// List all mailboxes
pub struct MailMailboxesTool;

#[async_trait]
impl TalosTool for MailMailboxesTool {
    fn name(&self) -> &'static str {
        "mail_mailboxes"
    }
    fn description(&self) -> &'static str {
        "List all mailboxes in Apple Mail"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let _ = &args;

        #[cfg(target_os = "macos")]
        {
            let script = r#"
                set mbList to ""
                tell application "Mail"
                    repeat with acct in accounts
                        set acctName to name of acct
                        repeat with mb in mailboxes of acct
                            set mbList to mbList & acctName & "/" & (name of mb) & linefeed
                        end repeat
                    end repeat
                end tell
                return mbList
            "#;

            run_applescript(script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            Ok("Mail tools only available on macOS".to_string())
        }
    }
}

/// Get unread message count
pub struct MailUnreadCountTool;

#[async_trait]
impl TalosTool for MailUnreadCountTool {
    fn name(&self) -> &'static str {
        "mail_unread_count"
    }
    fn description(&self) -> &'static str {
        "Get the count of unread email messages"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "mailbox",
            "string",
            "Mailbox name (optional, defaults to inbox)",
            false,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let mailbox = args.get("mailbox").and_then(|v| v.as_str());

            let mailbox_ref = if let Some(mb) = mailbox {
                format!(
                    r#"mailbox "{}" of account 1"#,
                    crate::sanitize_applescript(mb)
                )
            } else {
                "inbox".to_string()
            };

            let script = format!(
                r#"
                tell application "Mail"
                    set unreadCount to count of (messages of {} whose read status is false)
                    return unreadCount as string
                end tell
            "#,
                mailbox_ref
            );

            run_applescript(&script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Mail tools only available on macOS".to_string())
        }
    }
}

/// Mark an email as read or unread
pub struct MailMarkReadTool;

#[async_trait]
impl TalosTool for MailMarkReadTool {
    fn name(&self) -> &'static str {
        "mail_mark_read"
    }
    fn description(&self) -> &'static str {
        "Mark an email message as read or unread"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "id",
                "integer",
                "Message index (1-based) in the inbox",
                true,
            )
            .with_param(
                "read",
                "boolean",
                "True to mark as read, false for unread",
                true,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let id = args
                .get("id")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| Error::Tool("Missing id".to_string()))?;

            let read = args
                .get("read")
                .and_then(|v| v.as_bool())
                .ok_or_else(|| Error::Tool("Missing read".to_string()))?;

            let script = format!(
                r#"
                tell application "Mail"
                    set read status of message {} of inbox to {}
                end tell
                return "Message {} marked as {}"
            "#,
                id,
                read,
                id,
                if read { "read" } else { "unread" }
            );

            run_applescript(&script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Mail tools only available on macOS".to_string())
        }
    }
}

// ── Extended mail tools ──────────────────────────────────────────────

/// Forward an email message to a recipient
pub struct MailForwardTool;

#[async_trait]
impl TalosTool for MailForwardTool {
    fn name(&self) -> &'static str {
        "mail_forward"
    }
    fn description(&self) -> &'static str {
        "Forward an email message to another recipient"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "id",
                "integer",
                "Message index (1-based) in the inbox",
                true,
            )
            .with_param(
                "to",
                "string",
                "Recipient email address to forward to",
                true,
            )
            .with_param(
                "comment",
                "string",
                "Optional comment to prepend to the forwarded message",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let id = args
                .get("id")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| Error::Tool("Missing id".to_string()))?;
            let to = args
                .get("to")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing to".to_string()))?;
            let comment = args.get("comment").and_then(|v| v.as_str()).unwrap_or("");

            let comment_block = if !comment.is_empty() {
                format!(
                    r#"set content of fwdMsg to "{}" & return & return & (content of fwdMsg)"#,
                    crate::sanitize_applescript(comment)
                )
            } else {
                String::new()
            };

            let script = format!(
                r#"
                tell application "Mail"
                    set origMsg to message {} of inbox
                    set fwdMsg to forward origMsg with opening window
                    tell fwdMsg
                        make new to recipient at end of to recipients with properties {{address:"{}"}}
                        {}
                    end tell
                    send fwdMsg
                end tell
                return "Email forwarded to {}"
            "#,
                id,
                crate::sanitize_applescript(to),
                comment_block,
                crate::sanitize_applescript(to)
            );

            run_applescript(&script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Mail tools only available on macOS".to_string())
        }
    }
}
