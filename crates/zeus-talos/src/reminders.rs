//! Reminders tools (macOS)

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::Value;
use zeus_core::{Result, ToolSchema};

#[cfg(target_os = "macos")]
use crate::run_applescript;

/// List reminders
pub struct RemindersListTool;

#[async_trait]
impl TalosTool for RemindersListTool {
    fn name(&self) -> &'static str {
        "reminders_list"
    }
    fn description(&self) -> &'static str {
        "List reminders from Apple Reminders"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("list", "string", "Reminders list name (optional)", false)
            .with_param(
                "completed",
                "boolean",
                "Include completed (default false)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let completed = args
                .get("completed")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            let script = format!(
                r#"
                set reminderList to ""
                tell application "Reminders"
                    repeat with r in reminders
                        if {} or not completed of r then
                            set reminderList to reminderList & (name of r) & linefeed
                        end if
                    end repeat
                end tell
                return reminderList
            "#,
                completed
            );

            run_applescript(&script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Reminders tools only available on macOS".to_string())
        }
    }
}

/// Complete a reminder
pub struct RemindersCompleteTool;

#[async_trait]
impl TalosTool for RemindersCompleteTool {
    fn name(&self) -> &'static str {
        "reminders_complete"
    }
    fn description(&self) -> &'static str {
        "Mark a reminder as completed"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "name",
            "string",
            "Reminder name to complete",
            true,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            use zeus_core::Error;

            let name = args
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing name".to_string()))?;

            let script = format!(
                r#"
                tell application "Reminders"
                    set matchingReminders to (every reminder whose name is "{}")
                    if (count of matchingReminders) > 0 then
                        set completed of (item 1 of matchingReminders) to true
                        return "Completed"
                    else
                        return "Reminder not found"
                    end if
                end tell
            "#,
                crate::sanitize_applescript(name)
            );

            run_applescript(&script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Reminders tools only available on macOS".to_string())
        }
    }
}

/// Create a reminder
pub struct RemindersCreateTool;

#[async_trait]
impl TalosTool for RemindersCreateTool {
    fn name(&self) -> &'static str {
        "reminders_create"
    }
    fn description(&self) -> &'static str {
        "Create a new reminder"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("title", "string", "Reminder title", true)
            .with_param("due", "string", "Due date (natural language)", false)
            .with_param("list", "string", "Reminders list name", false)
            .with_param("notes", "string", "Additional notes", false)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            use zeus_core::Error;

            let title = args
                .get("title")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing title".to_string()))?;

            let list = args
                .get("list")
                .and_then(|v| v.as_str())
                .unwrap_or("Reminders");

            let script = format!(
                r#"
                tell application "Reminders"
                    tell list "{}"
                        make new reminder with properties {{name:"{}"}}
                    end tell
                end tell
                return "Reminder created"
            "#,
                crate::sanitize_applescript(list),
                crate::sanitize_applescript(title)
            );

            run_applescript(&script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Reminders tools only available on macOS".to_string())
        }
    }
}

// === REMINDERS ADDITIONS ===

/// Delete a reminder by name
pub struct RemindersDeleteTool;

#[async_trait]
impl TalosTool for RemindersDeleteTool {
    fn name(&self) -> &'static str {
        "reminders_delete"
    }
    fn description(&self) -> &'static str {
        "Delete a reminder by name"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "name",
            "string",
            "Reminder name to delete",
            true,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            use zeus_core::Error;

            let name = args
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing name".to_string()))?;

            let script = format!(
                r#"
                tell application "Reminders"
                    set matchingReminders to (every reminder whose name is "{}")
                    if (count of matchingReminders) > 0 then
                        delete (item 1 of matchingReminders)
                        return "Reminder deleted"
                    else
                        return "Reminder not found"
                    end if
                end tell
            "#,
                crate::sanitize_applescript(name)
            );

            run_applescript(&script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Reminders tools only available on macOS".to_string())
        }
    }
}

/// Search reminders by keyword
pub struct RemindersSearchTool;

#[async_trait]
impl TalosTool for RemindersSearchTool {
    fn name(&self) -> &'static str {
        "reminders_search"
    }
    fn description(&self) -> &'static str {
        "Search reminders by keyword"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "query",
            "string",
            "Search query",
            true,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            use zeus_core::Error;

            let query = args
                .get("query")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing query".to_string()))?;

            let escaped = crate::sanitize_applescript(query);
            let script = format!(
                r#"
                set reminderList to ""
                tell application "Reminders"
                    repeat with r in reminders
                        if (name of r) contains "{}" then
                            set reminderList to reminderList & (name of r)
                            if due date of r is not missing value then
                                set reminderList to reminderList & " (due: " & (due date of r as string) & ")"
                            end if
                            set reminderList to reminderList & linefeed
                        end if
                    end repeat
                end tell
                if reminderList is "" then
                    return "No matching reminders found"
                end if
                return reminderList
            "#,
                escaped
            );

            run_applescript(&script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Reminders tools only available on macOS".to_string())
        }
    }
}

/// List all reminder lists
pub struct RemindersListsTool;

#[async_trait]
impl TalosTool for RemindersListsTool {
    fn name(&self) -> &'static str {
        "reminders_lists"
    }
    fn description(&self) -> &'static str {
        "List all reminder lists"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let _ = &args;
            let script = r#"
                set listNames to ""
                tell application "Reminders"
                    repeat with l in lists
                        set listNames to listNames & (name of l) & linefeed
                    end repeat
                end tell
                return listNames
            "#;

            run_applescript(script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Reminders tools only available on macOS".to_string())
        }
    }
}

/// Get reminders due today
pub struct RemindersDueTodayTool;

#[async_trait]
impl TalosTool for RemindersDueTodayTool {
    fn name(&self) -> &'static str {
        "reminders_due_today"
    }
    fn description(&self) -> &'static str {
        "Get reminders due today"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let _ = &args;
            let script = r#"
                set reminderList to ""
                tell application "Reminders"
                    set todayStart to current date
                    set time of todayStart to 0
                    set todayEnd to todayStart + (1 * days)
                    repeat with r in reminders
                        if completed of r is false and due date of r is not missing value then
                            if due date of r is greater than or equal to todayStart and due date of r is less than todayEnd then
                                set reminderList to reminderList & (name of r) & " (due: " & (due date of r as string) & ")" & linefeed
                            end if
                        end if
                    end repeat
                end tell
                if reminderList is "" then
                    return "No reminders due today"
                end if
                return reminderList
            "#;

            run_applescript(script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Reminders tools only available on macOS".to_string())
        }
    }
}

/// Update a reminder
pub struct RemindersUpdateTool;

#[async_trait]
impl TalosTool for RemindersUpdateTool {
    fn name(&self) -> &'static str {
        "reminders_update"
    }
    fn description(&self) -> &'static str {
        "Update an existing reminder"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("name", "string", "Current reminder name to find", true)
            .with_param("new_name", "string", "New reminder name", false)
            .with_param("due_date", "string", "New due date", false)
            .with_param("notes", "string", "New notes/body", false)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            use zeus_core::Error;

            let name = args
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing name".to_string()))?;

            let escaped_name = crate::sanitize_applescript(name);

            let mut updates = Vec::new();

            if let Some(new_name) = args.get("new_name").and_then(|v| v.as_str()) {
                updates.push(format!(
                    r#"set name of r to "{}""#,
                    crate::sanitize_applescript(new_name)
                ));
            }

            if let Some(due_date) = args.get("due_date").and_then(|v| v.as_str()) {
                updates.push(format!(
                    r#"set due date of r to date "{}""#,
                    crate::sanitize_applescript(due_date)
                ));
            }

            if let Some(notes) = args.get("notes").and_then(|v| v.as_str()) {
                updates.push(format!(
                    r#"set body of r to "{}""#,
                    crate::sanitize_applescript(notes)
                ));
            }

            if updates.is_empty() {
                return Ok("No updates specified".to_string());
            }

            let update_lines = updates.join("\n                        ");

            let script = format!(
                r#"
                tell application "Reminders"
                    set matchingReminders to (every reminder whose name is "{}")
                    if (count of matchingReminders) > 0 then
                        set r to item 1 of matchingReminders
                        {}
                        return "Reminder updated"
                    else
                        return "Reminder not found"
                    end if
                end tell
            "#,
                escaped_name, update_lines
            );

            run_applescript(&script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Reminders tools only available on macOS".to_string())
        }
    }
}
