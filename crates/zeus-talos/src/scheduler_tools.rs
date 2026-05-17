//! Scheduler management tools
//!
//! Provides tools to create, list, and delete scheduled tasks.
//! Tasks are stored in SQLite and executed by the zeus-prometheus CronScheduler.

use async_trait::async_trait;
use serde_json::{Value, json};
use zeus_core::{Error, Result, ToolSchema};

use crate::TalosTool;

/// Get the scheduler database path
fn scheduler_db_path() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".zeus")
        .join("scheduler.db")
}

/// Ensure the scheduler database and table exist.
fn ensure_db() -> Result<rusqlite::Connection> {
    let path = scheduler_db_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| Error::Tool(format!("Failed to create dir: {}", e)))?;
    }
    let conn = rusqlite::Connection::open(&path)
        .map_err(|e| Error::Tool(format!("Failed to open scheduler db: {}", e)))?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS scheduled_tasks (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            schedule TEXT NOT NULL,
            schedule_type TEXT NOT NULL,
            task_payload TEXT NOT NULL,
            enabled INTEGER DEFAULT 1,
            last_run INTEGER,
            next_run INTEGER NOT NULL,
            last_status TEXT,
            last_output TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );",
    )
    .map_err(|e| Error::Tool(format!("Failed to init db: {}", e)))?;
    Ok(conn)
}

// ============================================================================
// schedule_create
// ============================================================================

pub struct ScheduleCreateTool;

#[async_trait]
impl TalosTool for ScheduleCreateTool {
    fn name(&self) -> &'static str {
        "schedule_create"
    }

    fn description(&self) -> &'static str {
        "Create a new scheduled task with a cron expression or human-readable schedule"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new("schedule_create", "Create a new scheduled task")
            .with_param(
                "name",
                "string",
                "Name for the task",
                true,
            )
            .with_param(
                "schedule",
                "string",
                "Cron expression (e.g. '*/5 * * * *') or human-readable (e.g. 'every 5 minutes', 'daily at 9am')",
                true,
            )
            .with_param(
                "task_type",
                "string",
                "Type: 'shell', 'prompt', or 'note'",
                true,
            )
            .with_param(
                "payload",
                "string",
                "Shell command, LLM prompt, or note content",
                true,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'name' parameter".to_string()))?;
        let schedule = args
            .get("schedule")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'schedule' parameter".to_string()))?;
        let task_type = args
            .get("task_type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'task_type' parameter".to_string()))?;
        let payload = args
            .get("payload")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'payload' parameter".to_string()))?;

        // Build task_payload JSON based on task_type
        let task_payload = match task_type {
            "shell" => json!({"type": "shell", "command": payload}),
            "prompt" => json!({"type": "llm_prompt", "prompt": payload}),
            "note" => json!({"type": "workspace_note", "content": payload}),
            other => {
                return Err(Error::Tool(format!(
                    "Unknown task_type '{}'. Use 'shell', 'prompt', or 'note'",
                    other
                )));
            }
        };

        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().timestamp();

        // For next_run, we need to parse the cron. Try to normalize human-readable first.
        let cron_expr = normalize_schedule(schedule);

        // Compute next_run from the cron expression
        let normalized_7field = normalize_cron_for_parse(&cron_expr);
        let next_run = match cron::Schedule::from_str(&normalized_7field) {
            Ok(sched) => sched
                .upcoming(chrono::Utc)
                .next()
                .map(|dt| dt.timestamp())
                .unwrap_or(now + 3600),
            Err(_) => {
                return Err(Error::Tool(format!(
                    "Invalid schedule expression: '{}'",
                    schedule
                )));
            }
        };

        let conn = ensure_db()?;
        conn.execute(
            "INSERT INTO scheduled_tasks (id, name, schedule, schedule_type, task_payload, enabled, next_run, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, ?7, ?7)",
            rusqlite::params![id, name, cron_expr, "cron", task_payload.to_string(), next_run, now],
        )
        .map_err(|e| Error::Tool(format!("Failed to insert task: {}", e)))?;

        Ok(format!(
            "Created scheduled task '{}' (id: {})\nSchedule: {}\nNext run: {}",
            name,
            id,
            cron_expr,
            chrono::DateTime::from_timestamp(next_run, 0)
                .map(|dt| dt.format("%Y-%m-%d %H:%M UTC").to_string())
                .unwrap_or_else(|| "unknown".to_string())
        ))
    }
}

// ============================================================================
// schedule_list
// ============================================================================

pub struct ScheduleListTool;

#[async_trait]
impl TalosTool for ScheduleListTool {
    fn name(&self) -> &'static str {
        "schedule_list"
    }

    fn description(&self) -> &'static str {
        "List all scheduled tasks"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "schedule_list",
            "List all scheduled tasks with their status",
        )
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        let conn = ensure_db()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, name, schedule, task_payload, enabled, last_run, next_run, last_status FROM scheduled_tasks ORDER BY next_run",
            )
            .map_err(|e| Error::Tool(format!("Query failed: {}", e)))?;

        let tasks: Vec<String> = stmt
            .query_map([], |row| {
                let id: String = row.get(0)?;
                let name: String = row.get(1)?;
                let schedule: String = row.get(2)?;
                let _payload: String = row.get(3)?;
                let enabled: bool = row.get(4)?;
                let last_run: Option<i64> = row.get(5)?;
                let next_run: i64 = row.get(6)?;
                let last_status: Option<String> = row.get(7)?;

                let status = if enabled { "enabled" } else { "disabled" };
                let last = last_run
                    .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0))
                    .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_else(|| "never".to_string());
                let next = chrono::DateTime::from_timestamp(next_run, 0)
                    .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_else(|| "unknown".to_string());
                let last_st = last_status.unwrap_or_else(|| "-".to_string());

                Ok(format!(
                    "  {} | {} | {} | {} | last: {} ({}) | next: {}",
                    &id[..zeus_core::floor_char_boundary(&id, 8)],
                    status,
                    name,
                    schedule,
                    last,
                    last_st,
                    next
                ))
            })
            .map_err(|e| Error::Tool(format!("Query failed: {}", e)))?
            .filter_map(|r| r.ok())
            .collect();

        if tasks.is_empty() {
            Ok("No scheduled tasks.".to_string())
        } else {
            Ok(format!(
                "Scheduled tasks ({}):\n{}",
                tasks.len(),
                tasks.join("\n")
            ))
        }
    }
}

// ============================================================================
// schedule_delete
// ============================================================================

pub struct ScheduleDeleteTool;

#[async_trait]
impl TalosTool for ScheduleDeleteTool {
    fn name(&self) -> &'static str {
        "schedule_delete"
    }

    fn description(&self) -> &'static str {
        "Delete a scheduled task by ID"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new("schedule_delete", "Delete a scheduled task").with_param(
            "id",
            "string",
            "Task ID (or first 8 characters)",
            true,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let id = args
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'id' parameter".to_string()))?;

        let conn = ensure_db()?;

        // Support partial ID matching (first 8 chars)
        let deleted = if id.len() < 36 {
            conn.execute(
                "DELETE FROM scheduled_tasks WHERE id LIKE ?1",
                rusqlite::params![format!("{}%", id)],
            )
        } else {
            conn.execute(
                "DELETE FROM scheduled_tasks WHERE id = ?1",
                rusqlite::params![id],
            )
        }
        .map_err(|e| Error::Tool(format!("Delete failed: {}", e)))?;

        if deleted == 0 {
            Err(Error::Tool(format!("No task found with id '{}'", id)))
        } else {
            Ok(format!("Deleted {} task(s)", deleted))
        }
    }
}

// ============================================================================
// Helpers
// ============================================================================

use std::str::FromStr;

/// Normalize human-readable schedule to cron expression.
fn normalize_schedule(input: &str) -> String {
    let s = input.trim().to_lowercase();

    // Check for "every N minutes"
    if let Some(n) = parse_every_n(&s, "minute") {
        return format!("*/{} * * * *", n);
    }

    // Check for "every N hours"
    if let Some(n) = parse_every_n(&s, "hour") {
        return format!("0 */{} * * *", n);
    }

    // Hourly
    if s == "hourly" || s == "every hour" {
        return "0 * * * *".to_string();
    }

    // Daily variants
    if s == "daily" || s == "every day" {
        return "0 9 * * *".to_string();
    }

    // "daily at Xam/pm" or "daily at HH:MM"
    if let Some(time_str) = s.strip_prefix("daily at ")
        && let Some((hour, minute)) = parse_time(time_str)
    {
        return format!("{} {} * * *", minute, hour);
    }

    // Weekly variants
    if s == "weekly" || s == "every week" {
        return "0 9 * * 1".to_string();
    }

    if let Some(day_str) = s.strip_prefix("weekly on ")
        && let Some(dow) = parse_day_of_week(day_str)
    {
        return format!("0 9 * * {}", dow);
    }

    // Monthly
    if s == "monthly" || s == "every month" {
        return "0 9 1 * *".to_string();
    }

    // Pass through as raw cron
    input.trim().to_string()
}

/// Parse "every N minutes/hours" pattern.
fn parse_every_n(s: &str, unit: &str) -> Option<u32> {
    // "every 5 minutes" or "every 2 hours"
    let prefix = "every ";
    if !s.starts_with(prefix) {
        return None;
    }
    let rest = &s[prefix.len()..];

    // Check it ends with the unit (singular or plural)
    if !rest.ends_with(unit) && !rest.ends_with(&format!("{}s", unit)) {
        return None;
    }

    // Extract the number
    let parts: Vec<&str> = rest.split_whitespace().collect();
    if parts.len() == 2 {
        parts[0].parse::<u32>().ok()
    } else {
        None
    }
}

/// Parse a time string like "3pm", "15:00", "9am", "14:30"
fn parse_time(s: &str) -> Option<(u32, u32)> {
    let s = s.trim();

    // HH:MM format
    if s.contains(':') {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() == 2 {
            let hour = parts[0].parse::<u32>().ok()?;
            let minute = parts[1]
                .trim_end_matches(|c: char| c.is_alphabetic())
                .parse::<u32>()
                .ok()?;
            if hour < 24 && minute < 60 {
                return Some((hour, minute));
            }
        }
        return None;
    }

    // Xam/Xpm format
    if s.ends_with("am") {
        let hour = s.trim_end_matches("am").trim().parse::<u32>().ok()?;
        if hour <= 12 {
            return Some((if hour == 12 { 0 } else { hour }, 0));
        }
    }
    if s.ends_with("pm") {
        let hour = s.trim_end_matches("pm").trim().parse::<u32>().ok()?;
        if hour <= 12 {
            return Some((if hour == 12 { 12 } else { hour + 12 }, 0));
        }
    }

    None
}

/// Parse day of week name to 0-6 (Sun=0).
fn parse_day_of_week(s: &str) -> Option<u32> {
    match s.trim() {
        "sunday" | "sun" => Some(0),
        "monday" | "mon" => Some(1),
        "tuesday" | "tue" => Some(2),
        "wednesday" | "wed" => Some(3),
        "thursday" | "thu" => Some(4),
        "friday" | "fri" => Some(5),
        "saturday" | "sat" => Some(6),
        _ => None,
    }
}

/// Normalize to 7-field cron for the `cron` crate.
fn normalize_cron_for_parse(expr: &str) -> String {
    let fields: Vec<&str> = expr.split_whitespace().collect();
    match fields.len() {
        5 => format!("0 {} *", expr),
        6 => format!("{} *", expr),
        7 => expr.to_string(),
        _ => expr.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_schedule_every_minutes() {
        assert_eq!(normalize_schedule("every 5 minutes"), "*/5 * * * *");
        assert_eq!(normalize_schedule("every 15 minutes"), "*/15 * * * *");
    }

    #[test]
    fn test_normalize_schedule_every_hours() {
        assert_eq!(normalize_schedule("every 2 hours"), "0 */2 * * *");
        assert_eq!(normalize_schedule("hourly"), "0 * * * *");
    }

    #[test]
    fn test_normalize_schedule_daily() {
        assert_eq!(normalize_schedule("daily"), "0 9 * * *");
        assert_eq!(normalize_schedule("daily at 3pm"), "0 15 * * *");
        assert_eq!(normalize_schedule("daily at 15:00"), "0 15 * * *");
        assert_eq!(normalize_schedule("daily at 9am"), "0 9 * * *");
    }

    #[test]
    fn test_normalize_schedule_weekly() {
        assert_eq!(normalize_schedule("weekly"), "0 9 * * 1");
        assert_eq!(normalize_schedule("weekly on friday"), "0 9 * * 5");
        assert_eq!(normalize_schedule("weekly on sunday"), "0 9 * * 0");
    }

    #[test]
    fn test_normalize_schedule_monthly() {
        assert_eq!(normalize_schedule("monthly"), "0 9 1 * *");
    }

    #[test]
    fn test_normalize_schedule_passthrough() {
        assert_eq!(normalize_schedule("*/10 * * * *"), "*/10 * * * *");
        assert_eq!(normalize_schedule("0 9 * * 1"), "0 9 * * 1");
    }

    #[test]
    fn test_parse_time() {
        assert_eq!(parse_time("3pm"), Some((15, 0)));
        assert_eq!(parse_time("9am"), Some((9, 0)));
        assert_eq!(parse_time("12pm"), Some((12, 0)));
        assert_eq!(parse_time("12am"), Some((0, 0)));
        assert_eq!(parse_time("15:00"), Some((15, 0)));
        assert_eq!(parse_time("14:30"), Some((14, 30)));
    }

    #[test]
    fn test_parse_day_of_week() {
        assert_eq!(parse_day_of_week("monday"), Some(1));
        assert_eq!(parse_day_of_week("fri"), Some(5));
        assert_eq!(parse_day_of_week("sunday"), Some(0));
    }

    #[tokio::test]
    async fn test_schedule_create_tool_schema() {
        let tool = ScheduleCreateTool;
        assert_eq!(tool.name(), "schedule_create");
        let schema = tool.schema();
        assert_eq!(schema.name, "schedule_create");
    }

    #[tokio::test]
    async fn test_schedule_list_tool_schema() {
        let tool = ScheduleListTool;
        assert_eq!(tool.name(), "schedule_list");
    }

    #[tokio::test]
    async fn test_schedule_delete_tool_schema() {
        let tool = ScheduleDeleteTool;
        assert_eq!(tool.name(), "schedule_delete");
    }

    #[tokio::test]
    async fn test_schedule_create_and_list() {
        // Use a temp db path
        let tmp = tempfile::TempDir::new().expect("TempDir::new should succeed");
        let db_path = tmp.path().join("test_scheduler.db");

        // We can't easily override scheduler_db_path() in tests, so let's test
        // the ensure_db and SQL logic directly
        let conn = rusqlite::Connection::open(&db_path).expect("should open file");
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS scheduled_tasks (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                schedule TEXT NOT NULL,
                schedule_type TEXT NOT NULL,
                task_payload TEXT NOT NULL,
                enabled INTEGER DEFAULT 1,
                last_run INTEGER,
                next_run INTEGER NOT NULL,
                last_status TEXT,
                last_output TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );",
        )
        .expect("operation should succeed");

        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT INTO scheduled_tasks (id, name, schedule, schedule_type, task_payload, enabled, next_run, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, ?7, ?7)",
            rusqlite::params!["test-id", "Test Task", "*/5 * * * *", "cron", r#"{"type":"shell","command":"echo hi"}"#, now + 300, now],
        ).expect("operation should succeed");

        // Verify we can read it back
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM scheduled_tasks", [], |r| r.get(0))
            .expect("key should exist");
        assert_eq!(count, 1);

        // Delete
        conn.execute(
            "DELETE FROM scheduled_tasks WHERE id = ?1",
            rusqlite::params!["test-id"],
        )
        .expect("operation should succeed");
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM scheduled_tasks", [], |r| r.get(0))
            .expect("key should exist");
        assert_eq!(count, 0);
    }
}
