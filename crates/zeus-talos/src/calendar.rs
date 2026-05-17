//! Calendar tools (macOS)

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::Value;
use zeus_core::{Result, ToolSchema};

#[cfg(target_os = "macos")]
use crate::run_applescript;

/// List calendar events
pub struct CalendarListTool;

#[async_trait]
impl TalosTool for CalendarListTool {
    fn name(&self) -> &'static str {
        "calendar_list"
    }
    fn description(&self) -> &'static str {
        "List calendar events for today or a specific date range"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("calendar", "string", "Calendar name (optional)", false)
            .with_param(
                "days",
                "integer",
                "Number of days to look ahead (default 7)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let days = args.get("days").and_then(|v| v.as_u64()).unwrap_or(7);

            let script = format!(
                r#"
                set eventList to ""
                tell application "Calendar"
                    set endDate to (current date) + ({} * days)
                    repeat with cal in calendars
                        repeat with evt in (events of cal whose start date is greater than (current date) and start date is less than endDate)
                            set eventList to eventList & (name of cal) & ": " & (summary of evt) & " @ " & (start date of evt as string) & linefeed
                        end repeat
                    end repeat
                end tell
                return eventList
            "#,
                days
            );

            run_applescript(&script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Calendar tools only available on macOS".to_string())
        }
    }
}

/// Create a calendar event
pub struct CalendarCreateTool;

#[async_trait]
impl TalosTool for CalendarCreateTool {
    fn name(&self) -> &'static str {
        "calendar_create"
    }
    fn description(&self) -> &'static str {
        "Create a new calendar event"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("title", "string", "Event title", true)
            .with_param(
                "start",
                "string",
                "Start date/time (natural language)",
                true,
            )
            .with_param("end", "string", "End date/time (natural language)", false)
            .with_param("calendar", "string", "Calendar name", false)
            .with_param("notes", "string", "Event notes", false)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            use zeus_core::Error;

            let title = args
                .get("title")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing title".to_string()))?;

            let start = args
                .get("start")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing start".to_string()))?;

            let end = args.get("end").and_then(|v| v.as_str()).unwrap_or(start);

            let calendar = args
                .get("calendar")
                .and_then(|v| v.as_str())
                .unwrap_or("Calendar");

            let notes = args.get("notes").and_then(|v| v.as_str()).unwrap_or("");

            let script = format!(
                r#"
                tell application "Calendar"
                    tell calendar "{}"
                        set newEvent to make new event with properties {{summary:"{}", start date:date "{}", end date:date "{}", description:"{}"}}
                    end tell
                end tell
                return "Event created"
            "#,
                crate::sanitize_applescript(calendar),
                crate::sanitize_applescript(title),
                crate::sanitize_applescript(start),
                crate::sanitize_applescript(end),
                crate::sanitize_applescript(notes),
            );

            run_applescript(&script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Calendar tools only available on macOS".to_string())
        }
    }
}

// === CALENDAR ADDITIONS ===

/// Delete a calendar event by title
pub struct CalendarDeleteTool;

#[async_trait]
impl TalosTool for CalendarDeleteTool {
    fn name(&self) -> &'static str {
        "calendar_delete"
    }
    fn description(&self) -> &'static str {
        "Delete a calendar event by title"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("title", "string", "Event title to delete", true)
            .with_param(
                "calendar",
                "string",
                "Calendar name (optional, searches all if omitted)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            use zeus_core::Error;

            let title = args
                .get("title")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing title".to_string()))?;

            let escaped_title = crate::sanitize_applescript(title);

            let calendar_clause = if let Some(cal) = args.get("calendar").and_then(|v| v.as_str()) {
                let escaped_cal = crate::sanitize_applescript(cal);
                format!(r#"set targetCals to {{calendar "{}"}}"#, escaped_cal)
            } else {
                "set targetCals to calendars".to_string()
            };

            let script = format!(
                r#"
                tell application "Calendar"
                    {}
                    set deleted to false
                    repeat with cal in targetCals
                        set matchingEvents to (every event of cal whose summary is "{}")
                        repeat with evt in matchingEvents
                            delete evt
                            set deleted to true
                        end repeat
                    end repeat
                    if deleted then
                        return "Event deleted"
                    else
                        return "Event not found"
                    end if
                end tell
            "#,
                calendar_clause, escaped_title
            );

            run_applescript(&script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Calendar tools only available on macOS".to_string())
        }
    }
}

/// Search calendar events by keyword
pub struct CalendarSearchTool;

#[async_trait]
impl TalosTool for CalendarSearchTool {
    fn name(&self) -> &'static str {
        "calendar_search"
    }
    fn description(&self) -> &'static str {
        "Search calendar events by keyword"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("query", "string", "Search query", true)
            .with_param(
                "days",
                "integer",
                "Number of days to search ahead (default 30)",
                false,
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

            let days = args.get("days").and_then(|v| v.as_u64()).unwrap_or(30);

            let escaped = crate::sanitize_applescript(query);
            let script = format!(
                r#"
                set eventList to ""
                tell application "Calendar"
                    set startDate to current date
                    set endDate to startDate + ({} * days)
                    repeat with cal in calendars
                        repeat with evt in (events of cal whose start date is greater than startDate and start date is less than endDate)
                            if (summary of evt) contains "{}" then
                                set eventList to eventList & (name of cal) & ": " & (summary of evt) & " @ " & (start date of evt as string) & linefeed
                            end if
                        end repeat
                    end repeat
                end tell
                if eventList is "" then
                    return "No matching events found"
                end if
                return eventList
            "#,
                days, escaped
            );

            run_applescript(&script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Calendar tools only available on macOS".to_string())
        }
    }
}

/// List all calendars
pub struct CalendarListCalendarsTool;

#[async_trait]
impl TalosTool for CalendarListCalendarsTool {
    fn name(&self) -> &'static str {
        "calendar_list_calendars"
    }
    fn description(&self) -> &'static str {
        "List all available calendars"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let _ = &args;
            let script = r#"
                set calList to ""
                tell application "Calendar"
                    repeat with cal in calendars
                        set calList to calList & (name of cal) & linefeed
                    end repeat
                end tell
                return calList
            "#;

            run_applescript(script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Calendar tools only available on macOS".to_string())
        }
    }
}

/// Get upcoming events within the next N hours
pub struct CalendarUpcomingTool;

#[async_trait]
impl TalosTool for CalendarUpcomingTool {
    fn name(&self) -> &'static str {
        "calendar_upcoming"
    }
    fn description(&self) -> &'static str {
        "Get upcoming calendar events within the next N hours"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "hours",
            "integer",
            "Number of hours to look ahead (default 24)",
            false,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let hours = args.get("hours").and_then(|v| v.as_u64()).unwrap_or(24);

            let script = format!(
                r#"
                set eventList to ""
                tell application "Calendar"
                    set startDate to current date
                    set endDate to startDate + ({} * hours)
                    repeat with cal in calendars
                        repeat with evt in (events of cal whose start date is greater than or equal to startDate and start date is less than endDate)
                            set eventList to eventList & (name of cal) & ": " & (summary of evt) & " @ " & (start date of evt as string) & linefeed
                        end repeat
                    end repeat
                end tell
                if eventList is "" then
                    return "No upcoming events"
                end if
                return eventList
            "#,
                hours
            );

            run_applescript(&script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Calendar tools only available on macOS".to_string())
        }
    }
}

/// Update a calendar event
pub struct CalendarUpdateTool;

#[async_trait]
impl TalosTool for CalendarUpdateTool {
    fn name(&self) -> &'static str {
        "calendar_update"
    }
    fn description(&self) -> &'static str {
        "Update an existing calendar event"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("title", "string", "Current event title to find", true)
            .with_param("new_title", "string", "New event title", false)
            .with_param("new_start", "string", "New start date/time", false)
            .with_param("new_end", "string", "New end date/time", false)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            use zeus_core::Error;

            let title = args
                .get("title")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing title".to_string()))?;

            let escaped_title = crate::sanitize_applescript(title);

            let mut updates = Vec::new();

            if let Some(new_title) = args.get("new_title").and_then(|v| v.as_str()) {
                updates.push(format!(
                    r#"set summary of evt to "{}""#,
                    crate::sanitize_applescript(new_title)
                ));
            }

            if let Some(new_start) = args.get("new_start").and_then(|v| v.as_str()) {
                updates.push(format!(
                    r#"set start date of evt to date "{}""#,
                    crate::sanitize_applescript(new_start)
                ));
            }

            if let Some(new_end) = args.get("new_end").and_then(|v| v.as_str()) {
                updates.push(format!(
                    r#"set end date of evt to date "{}""#,
                    crate::sanitize_applescript(new_end)
                ));
            }

            if updates.is_empty() {
                return Ok("No updates specified".to_string());
            }

            let update_lines = updates.join("\n                            ");

            let script = format!(
                r#"
                tell application "Calendar"
                    set updated to false
                    repeat with cal in calendars
                        set matchingEvents to (every event of cal whose summary is "{}")
                        repeat with evt in matchingEvents
                            {}
                            set updated to true
                        end repeat
                    end repeat
                    if updated then
                        return "Event updated"
                    else
                        return "Event not found"
                    end if
                end tell
            "#,
                escaped_title, update_lines
            );

            run_applescript(&script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Calendar tools only available on macOS".to_string())
        }
    }
}
