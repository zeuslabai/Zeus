//! Deploy — deployment targets, history, fleet stats
//!
//! Advanced subview (id: `deploy`). Wired live off the deploy store (#185,
//! merakizzz "wire all"). The panel's real subject is **deployment targets +
//! deployment history**, backed by:
//!   - `GET /v1/deploy/targets`  → TARGETS (name · provider · env · url · active)
//!   - `GET /v1/deploy/history`  → RECENT DEPLOYMENTS (target · version · status · trigger)
//!   - `GET /v1/deploy/stats`    → summary line (targets / deployments / live / failed)
//!
//! This replaces the previous daemon-health mock (zeus-daemon/discord-gateway
//! uptime + restart counts) — that was a domain mismatch (the panel showed
//! local service liveness, but `/v1/deploy/*` is a remote deploy-target store).
//! Per merakizzz's adjudication it now renders the deploy-target subject it's
//! actually backed by; there is no per-target uptime/restart field, so those
//! fabricated columns are dropped rather than faked. No live data → honest
//! "fetching from /v1/deploy/…" empty states, no fabricated fallback (#284
//! de-mock). Theme tokens, geometric glyphs, no emoji.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Clear, Widget};

use crate::api::{DeployStatsResponse, DeployTargetResponse, DeploymentResponse};
use crate::prod::draw::BufferClampExt;
use crate::theme;

/// One TARGETS row. Owned so live entries carry server-fetched fields.
struct TargetRow {
    name: String,
    provider: String,
    environment: String,
    url: String,
    active: bool,
}

/// One RECENT DEPLOYMENTS row.
struct DeployRow {
    target: String,
    version: String,
    status: String,
    trigger: String,
}

/// Build TARGETS rows: live overlay when `Some` and non-empty; empty otherwise
/// (no fabricated fallback — #284 de-mock).
fn build_targets(live: Option<&[DeployTargetResponse]>) -> Vec<TargetRow> {
    match live {
        Some(targets) if !targets.is_empty() => targets
            .iter()
            .map(|t| TargetRow {
                name: dash_if_empty(&t.name),
                provider: dash_if_empty(&t.provider),
                environment: dash_if_empty(&t.environment),
                url: dash_if_empty(&t.url),
                active: t.active,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Build RECENT DEPLOYMENTS rows: live overlay when `Some` and non-empty;
/// empty otherwise (no fabricated fallback — #284 de-mock).
fn build_deployments(live: Option<&[DeploymentResponse]>) -> Vec<DeployRow> {
    match live {
        Some(deps) if !deps.is_empty() => deps
            .iter()
            .map(|d| DeployRow {
                target: dash_if_empty(&d.target_name),
                version: dash_if_empty(&d.version),
                status: dash_if_empty(&d.status),
                trigger: dash_if_empty(&d.trigger),
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn dash_if_empty(s: &str) -> String {
    if s.is_empty() {
        "—".to_string()
    } else {
        s.to_string()
    }
}

/// Color a deployment/target status string by health.
fn status_color(status: &str) -> ratatui::style::Color {
    match status {
        "live" | "active" => theme::GREEN,
        "building" | "deploying" | "pending" => theme::YELLOW,
        "failed" | "cancelled" | "rolled_back" => theme::RED,
        _ => theme::DIM,
    }
}

/// Render the `deploy` subview body into `area`.
pub fn render(
    area: Rect,
    buf: &mut Buffer,
    targets_live: Option<&[DeployTargetResponse]>,
    history_live: Option<&[DeploymentResponse]>,
    stats: Option<&DeployStatsResponse>,
) {
    Clear.render(area, buf);
    if area.width < 8 || area.height < 3 {
        return;
    }

    let left = area.x + 2;
    let mut y = area.y + 1;
    let max_y = area.y + area.height;

    let targets = build_targets(targets_live);
    let deployments = build_deployments(history_live);

    // ── TARGETS ─────────────────────────────────────────────────────────
    buf.set_string_clamped(
        left,
        y,
        "TARGETS",
        Style::default()
            .fg(theme::ACCENT_DIM)
            .add_modifier(Modifier::BOLD),
    );
    y += 1;

    if targets.is_empty() {
        buf.set_string_clamped(
            left,
            y,
            "No targets — fetching from /v1/deploy/targets…",
            Style::default().fg(theme::DIM),
        );
        y += 1;
    }

    for t in &targets {
        if y >= max_y {
            break;
        }
        let dot = if t.active { "●" } else { "○" };
        let dot_color = if t.active { theme::GREEN } else { theme::DIM };
        buf.set_string_clamped(left, y, dot, Style::default().fg(dot_color));
        buf.set_string_clamped(
            left + 2,
            y,
            &t.name,
            Style::default()
                .fg(theme::TEXT)
                .add_modifier(Modifier::BOLD),
        );
        buf.set_string_clamped(left + 22, y, &t.provider, Style::default().fg(theme::CYAN));
        buf.set_string_clamped(left + 33, y, &t.environment, Style::default().fg(theme::DIM));
        if left + 46 < area.x + area.width {
            buf.set_string_clamped(left + 46, y, &t.url, Style::default().fg(theme::DIM));
        }
        y += 1;
    }

    y += 1;
    if y >= max_y {
        return;
    }

    // ── RECENT DEPLOYMENTS ──────────────────────────────────────────────
    buf.set_string_clamped(
        left,
        y,
        "RECENT DEPLOYMENTS",
        Style::default()
            .fg(theme::ACCENT_DIM)
            .add_modifier(Modifier::BOLD),
    );
    y += 1;

    if deployments.is_empty() {
        buf.set_string_clamped(
            left,
            y,
            "No deployments — fetching from /v1/deploy/history…",
            Style::default().fg(theme::DIM),
        );
        y += 1;
    }

    for d in &deployments {
        if y >= max_y {
            break;
        }
        buf.set_string_clamped(
            left,
            y,
            &d.target,
            Style::default().fg(theme::TEXT),
        );
        buf.set_string_clamped(left + 20, y, &d.version, Style::default().fg(theme::DIM));
        buf.set_string_clamped(
            left + 32,
            y,
            &d.status,
            Style::default().fg(status_color(&d.status)),
        );
        buf.set_string_clamped(left + 44, y, &d.trigger, Style::default().fg(theme::DIM));
        y += 1;
    }

    // ── Summary ─────────────────────────────────────────────────────────
    if y + 1 < max_y {
        y += 1;
        let summary = match stats {
            Some(s) => format!(
                "{} targets · {} deployments · {} live · {} failed",
                s.total_targets, s.total_deployments, s.live_deployments, s.failed_deployments
            ),
            None if !targets.is_empty() || !deployments.is_empty() => format!(
                "{} targets · {} recent deployments",
                targets.len(),
                deployments.len()
            ),
            None => "— targets · — deployments".to_string(),
        };
        buf.set_string_clamped(left, y, &summary, Style::default().fg(theme::DIM));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(name: &str, provider: &str, env: &str, url: &str, active: bool) -> DeployTargetResponse {
        DeployTargetResponse {
            name: name.to_string(),
            provider: provider.to_string(),
            environment: env.to_string(),
            url: url.to_string(),
            active,
        }
    }

    fn dep(target: &str, version: &str, status: &str, trigger: &str) -> DeploymentResponse {
        DeploymentResponse {
            target_name: target.to_string(),
            version: version.to_string(),
            status: status.to_string(),
            trigger: trigger.to_string(),
        }
    }

    #[test]
    fn live_targets_overlay_const() {
        let t = vec![target("prod", "vercel", "production", "https://x.io", true)];
        let rows = build_targets(Some(&t));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "prod");
        assert_eq!(rows[0].provider, "vercel");
        assert_eq!(rows[0].url, "https://x.io");
        assert!(rows[0].active);
    }

    #[test]
    fn empty_target_fields_dash() {
        let t = vec![target("", "", "", "", false)];
        let rows = build_targets(Some(&t));
        assert_eq!(rows[0].name, "—");
        assert_eq!(rows[0].provider, "—");
        assert_eq!(rows[0].url, "—");
        assert!(!rows[0].active);
    }

    #[test]
    fn none_targets_yields_empty() {
        let rows = build_targets(None);
        assert!(rows.is_empty(), "no fabricated fallback (#284)");
    }

    #[test]
    fn empty_live_targets_yields_empty() {
        let rows = build_targets(Some(&[]));
        assert!(rows.is_empty(), "no fabricated fallback (#284)");
    }

    #[test]
    fn live_deployments_overlay_const() {
        let d = vec![dep("web", "v2.0", "live", "push")];
        let rows = build_deployments(Some(&d));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].target, "web");
        assert_eq!(rows[0].status, "live");
    }

    #[test]
    fn none_deployments_yields_empty() {
        let rows = build_deployments(None);
        assert!(rows.is_empty(), "no fabricated fallback (#284)");
    }

    #[test]
    fn status_colors_by_health() {
        assert_eq!(status_color("live"), theme::GREEN);
        assert_eq!(status_color("building"), theme::YELLOW);
        assert_eq!(status_color("failed"), theme::RED);
        assert_eq!(status_color("unknown"), theme::DIM);
    }

    /// Fidelity-gate: render the panel into a real buffer and confirm both
    /// section headers + live target/deployment data + the live stats summary
    /// actually paint (TestBackend dump, not token-match — merakizzz's rule).
    #[test]
    fn render_paints_both_sections_and_live_summary() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let targets = vec![target("vercel-web", "vercel", "production", "https://web.io", true)];
        let history = vec![dep("vercel-web", "v3.1", "live", "push")];
        let stats = DeployStatsResponse {
            total_targets: 4,
            total_deployments: 12,
            live_deployments: 9,
            failed_deployments: 1,
        };

        let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
        term.draw(|f| {
            let area = f.area();
            render(area, f.buffer_mut(), Some(&targets), Some(&history), Some(&stats));
        })
        .unwrap();

        let buf = term.backend().buffer().clone();
        let dump: String = buf
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();

        // Both section headers paint.
        assert!(dump.contains("TARGETS"), "TARGETS header missing");
        assert!(dump.contains("RECENT DEPLOYMENTS"), "deployments header missing");
        // Live target data overlays (name + provider + url).
        assert!(dump.contains("vercel-web"), "live target name missing");
        assert!(dump.contains("https://web.io"), "live target url missing");
        // Live deployment row paints (version + status).
        assert!(dump.contains("v3.1"), "live deployment version missing");
        // Live stats summary (not the const fallback count).
        assert!(
            dump.contains("4 targets · 12 deployments · 9 live · 1 failed"),
            "live stats summary missing"
        );
    }

    /// Fidelity-gate: with no live data, honest empty states paint both
    /// sections + a dashed summary (no fabricated fallback — #284 de-mock).
    #[test]
    fn render_no_live_data_shows_honest_empty() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
        term.draw(|f| {
            let area = f.area();
            render(area, f.buffer_mut(), None, None, None);
        })
        .unwrap();

        let buf = term.backend().buffer().clone();
        let dump: String = buf
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();

        assert!(dump.contains("TARGETS"));
        assert!(dump.contains("RECENT DEPLOYMENTS"));
        assert!(dump.contains("fetching from /v1/deploy/targets"), "honest empty state missing");
        assert!(dump.contains("fetching from /v1/deploy/history"), "honest empty state missing");
        // No fabricated data leaks through.
        assert!(!dump.contains("vercel-web"), "fabricated target leaked");
        assert!(dump.contains("— targets · — deployments"), "dashed summary missing");
    }

    #[test]
    fn tiny_rect_no_panic() {
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        render(Rect::new(0, 0, 1, 1), &mut buf, None, None, None);
    }
}
