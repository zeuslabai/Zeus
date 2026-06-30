//! Extensions — Deno/MCP extensions, runtime
//!
//! Advanced subview (id: `extensions`). Renders installed extensions with
//! runtime status. Overlays live `GET /v1/extensions` data when fetched
//! (#185 wiring); shows an honest "awaiting" line until the first poll lands
//! — no fabricated extensions (#284 de-mock).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Clear, Widget};

use crate::api::ExtensionResponse;
use crate::prod::draw::BufferClampExt;
use crate::theme;

/// One rendered extension row (owned so live + const share a type).
struct ExtRow {
    name: String,
    version: String,
    status: String,
    status_color: Color,
    runtime: String,
}

/// Map a status word to its indicator color (shared by live + const).
fn status_color(status: &str) -> Color {
    match status {
        "active" | "running" => theme::GREEN,
        "idle" | "stopped" | "starting" | "stopping" => theme::YELLOW,
        "error" => theme::RED,
        _ => theme::DIM,
    }
}

struct ConstExt {
    name: &'static str,
    version: &'static str,
    status: &'static str,
    runtime: &'static str,
}

const EXTENSIONS: &[ConstExt] = &[
    ConstExt { name: "zeus-mcp", version: "0.8.2", status: "active", runtime: "deno" },
    ConstExt { name: "zeus-browser", version: "0.6.1", status: "active", runtime: "deno" },
    ConstExt { name: "zeus-llm", version: "0.9.0", status: "active", runtime: "native" },
    ConstExt { name: "zeus-memory", version: "0.5.4", status: "idle", runtime: "deno" },
    ConstExt { name: "zeus-deploy", version: "0.3.1", status: "error", runtime: "native" },
];

/// Build the rows to render: live overlay when present + non-empty, else const.
fn build_rows(live: Option<&[ExtensionResponse]>) -> Vec<ExtRow> {
    match live {
        Some(exts) if !exts.is_empty() => exts
            .iter()
            .map(|e| {
                let status = e.status_label();
                ExtRow {
                    status_color: status_color(&status),
                    name: e.name.clone(),
                    version: if e.version.is_empty() {
                        "—".to_string()
                    } else {
                        e.version.clone()
                    },
                    runtime: if e.extension_type.is_empty() {
                        "—".to_string()
                    } else {
                        e.extension_type.clone()
                    },
                    status,
                }
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Render the `extensions` subview body into `area`.
pub fn render(area: Rect, buf: &mut Buffer, live: Option<&[ExtensionResponse]>) {
    Clear.render(area, buf);
    if area.width < 20 || area.height < 6 {
        return;
    }

    let rows = build_rows(live);
    let mut y = area.y;

    // Header row
    let header = "NAME                 VERSION   STATUS    RUNTIME";
    buf.set_string_clamped(area.x + 1, y, header, Style::default().fg(theme::DIM).add_modifier(Modifier::BOLD));
    y += 1;

    // Separator
    let sep_width = (area.width as usize).saturating_sub(2).min(58);
    let sep = "─".repeat(sep_width);
    buf.set_string_clamped(area.x + 1, y, &sep, Style::default().fg(theme::DARK));
    y += 1;

    // Rows
    if rows.is_empty() {
        // Honest empty state — no fabricated extensions.
        buf.set_string_clamped(
            area.x + 1,
            y,
            "No extensions — fetching from /v1/extensions…",
            Style::default().fg(theme::DIM),
        );
        return;
    }
    for ext in &rows {
        if y >= area.y + area.height {
            break;
        }
        let line = format!(
            "{:20} {:9} {:9} {}",
            ext.name, ext.version, ext.status, ext.runtime
        );
        buf.set_string_clamped(area.x + 1, y, &line, Style::default().fg(theme::TEXT));
        // Status color on the word itself
        let status_x = area.x + 1 + 31;
        buf.set_string_clamped(status_x, y, &ext.status, Style::default().fg(ext.status_color));
        y += 1;
    }

    // Summary
    if y + 1 < area.y + area.height {
        y += 1;
        let active = rows.iter().filter(|e| e.status == "active" || e.status == "running").count();
        let idle = rows.iter().filter(|e| matches!(e.status.as_str(), "idle" | "stopped" | "starting" | "stopping")).count();
        let errored = rows.iter().filter(|e| e.status == "error").count();
        let summary = format!(
            "{} extensions · {} active · {} idle · {} error",
            rows.len(), active, idle, errored
        );
        buf.set_string_clamped(area.x + 1, y, &summary, Style::default().fg(theme::DIM));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ext(name: &str, version: &str, status: serde_json::Value, ty: &str) -> ExtensionResponse {
        ExtensionResponse {
            name: name.to_string(),
            version: version.to_string(),
            status,
            extension_type: ty.to_string(),
        }
    }

    #[test]
    fn live_overlays_const_extensions() {
        let live = vec![ext("my-ext", "1.2.3", serde_json::json!("Running"), "mcp")];
        let rows = build_rows(Some(&live));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "my-ext");
        assert_eq!(rows[0].version, "1.2.3");
        assert_eq!(rows[0].status, "running");
        assert_eq!(rows[0].runtime, "mcp");
        assert_eq!(rows[0].status_color, theme::GREEN);
    }

    #[test]
    fn live_handles_tagged_error_status() {
        let live = vec![ext("broken", "0.1.0", serde_json::json!({"Error": "boom"}), "deno")];
        let rows = build_rows(Some(&live));
        assert_eq!(rows[0].status, "error");
        assert_eq!(rows[0].status_color, theme::RED);
    }

    #[test]
    fn empty_live_yields_no_fabricated_rows() {
        assert!(build_rows(Some(&[])).is_empty());
    }

    #[test]
    fn none_yields_no_fabricated_rows() {
        assert!(build_rows(None).is_empty());
    }

    #[test]
    fn tiny_rect_no_panic() {
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        render(Rect::new(0, 0, 1, 1), &mut buf, None);
    }
}
