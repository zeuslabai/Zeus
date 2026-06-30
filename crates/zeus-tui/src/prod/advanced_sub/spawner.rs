//! Spawner — Active subagents, kill, logs
//!
//! Advanced subview (id: `spawner`). Showing active subagent processes, their
//! status, runtime, and channel bindings.
//!
//! Wires live off `GET /v1/spawner/active` (`SpawnResponse`): name←agent_id,
//! task←task, runtime←(now − started_at). The endpoint lists only running
//! spawns, so status is implicitly "running" (green). `channels` is not
//! exposed by the spawn tracker → honestly `0` for live rows (server-extension
//! gap, same class as agents host/role). When no live data is present the
//! panel shows an honest "awaiting" line — no fabricated roster (#284).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use crate::api::SpawnResponse;
use crate::prod::draw::BufferClampExt;
use crate::theme;
use ratatui::style::Color;
use ratatui::widgets::{Clear, Widget};

/// An owned spawn row — built from live `/v1/spawner/active` data. Live rows
/// are all "running" (the endpoint lists only active
/// spawns); `channels` is unbacked by the tracker → honestly `0`.
struct Row {
    name: String,
    task: String,
    status: String,
    status_color: Color,
    runtime_sec: u32,
    channels: u8,
}

/// Elapsed whole seconds from an RFC3339 `started_at` to now (0 on parse fail
/// or future timestamp). Honest runtime — never fabricated.
fn elapsed_secs(started_at: &str) -> u32 {
    chrono::DateTime::parse_from_rfc3339(started_at)
        .map(|t| {
            (chrono::Utc::now() - t.with_timezone(&chrono::Utc))
                .num_seconds()
                .max(0)
                .min(u32::MAX as i64) as u32
        })
        .unwrap_or(0)
}

/// Build the rows to render. Live overlay when `Some` and non-empty
/// (name/task/runtime honestly backed; status="running", channels 0). No live
/// data → empty (render shows an honest "awaiting" line).
fn build_rows(live: Option<&[SpawnResponse]>) -> Vec<Row> {
    match live {
        Some(ss) if !ss.is_empty() => ss
            .iter()
            .map(|s| Row {
                name: s.agent_id.clone(),
                task: s.task.clone(),
                status: "running".to_string(),
                status_color: theme::GREEN,
                runtime_sec: elapsed_secs(&s.started_at),
                channels: 0,
            })
            .collect(),
        // Pre-fetch or no active spawns → no fabricated roster (#284 de-mock).
        _ => Vec::new(),
    }
}

/// Render the `spawner` subview body into `area`.
///
/// `live` carries `/v1/spawner/active` when fetched; rows overlay live spawns
/// (status="running", channels honestly 0). No live data → honest empty state.
pub fn render(area: Rect, buf: &mut Buffer, live: Option<&[SpawnResponse]>) {
    Clear.render(area, buf);
    if area.width < 20 || area.height < 6 {
        return;
    }

    let mut y = area.y;

    // Header row
    let header = "NAME                 TASK           STATUS    TIME   CH";
    buf.set_string_clamped(area.x + 1, y, header, Style::default().fg(theme::DIM).add_modifier(Modifier::BOLD));
    y += 1;

    // Separator
    let sep_width = (area.width as usize).saturating_sub(2).min(58);
    let sep = "─".repeat(sep_width);
    buf.set_string_clamped(area.x + 1, y, &sep, Style::default().fg(theme::DARK));
    y += 1;

    let rows = build_rows(live);

    // Rows
    if rows.is_empty() {
        // Honest empty state — no fabricated spawns.
        buf.set_string_clamped(
            area.x + 1,
            y,
            "No active subagents — fetching from /v1/spawner/active…",
            Style::default().fg(theme::DIM),
        );
        return;
    }
    for spawn in &rows {
        if y >= area.y + area.height {
            break;
        }
        let time_str = if spawn.runtime_sec > 0 {
            format!("{}s", spawn.runtime_sec)
        } else {
            "-".to_string()
        };
        let line = format!(
            "{:20} {:14} {:9} {:6} {}",
            spawn.name, spawn.task, "", time_str, spawn.channels
        );
        buf.set_string_clamped(area.x + 1, y, &line, Style::default().fg(theme::TEXT));
        // Status color — draw into the reserved status column without doubling
        // the first letter over pre-rendered plain text (rrunning-style drift).
        let status_x = area.x + 1 + 37;
        buf.set_string_clamped(status_x, y, &spawn.status, Style::default().fg(spawn.status_color));
        y += 1;
    }

    // Summary — honest live counts (all active spawns are running).
    if y + 1 < area.y + area.height {
        y += 1;
        let running = rows.iter().filter(|r| r.status == "running").count();
        let summary = format!("{} subagents · {} running", rows.len(), running);
        buf.set_string_clamped(area.x + 1, y, &summary, Style::default().fg(theme::DIM));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spawn(id: &str, task: &str, started_at: &str) -> SpawnResponse {
        SpawnResponse {
            agent_id: id.to_string(),
            task: task.to_string(),
            role: "worker".to_string(),
            started_at: started_at.to_string(),
        }
    }

    #[test]
    fn none_yields_no_fabricated_rows() {
        // Pre-fetch (None) → zero rows: no invented roster.
        assert!(build_rows(None).is_empty());
    }

    #[test]
    fn empty_live_yields_no_fabricated_rows() {
        // Empty fleet → zero rows: no invented roster.
        assert!(build_rows(Some(&[])).is_empty());
    }

    #[test]
    fn live_rows_are_honest() {
        let live = vec![
            spawn("subagent-aaa", "memory_recall", "2020-01-01T00:00:00Z"),
            spawn("subagent-bbb", "web_search", "2020-01-01T00:00:00Z"),
        ];
        let rows = build_rows(Some(&live));
        assert_eq!(rows.len(), 2);
        // name/task honestly backed from live data.
        assert_eq!(rows[0].name, "subagent-aaa");
        assert_eq!(rows[0].task, "memory_recall");
        // active spawns are all "running" by definition.
        assert_eq!(rows[0].status, "running");
        // runtime is real elapsed (started 2020 → large positive), never 0.
        assert!(rows[0].runtime_sec > 0);
        // channels unbacked by the tracker → honestly 0, never fabricated.
        assert_eq!(rows[0].channels, 0);
    }

    #[test]
    fn elapsed_secs_handles_bad_and_future_input() {
        // Unparseable → 0, never panics or fabricates.
        assert_eq!(elapsed_secs("not-a-timestamp"), 0);
        assert_eq!(elapsed_secs(""), 0);
        // Far-future timestamp clamps to 0 (no negative runtime).
        assert_eq!(elapsed_secs("2999-01-01T00:00:00Z"), 0);
    }

    #[test]
    fn tiny_rect_no_panic() {
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        render(Rect::new(0, 0, 1, 1), &mut buf, None);
    }
}
