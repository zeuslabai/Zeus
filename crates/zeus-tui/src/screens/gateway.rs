use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Block, Borders, Widget};

use crate::theme;

// Theme aliases
const FG: ratatui::style::Color = theme::TEXT;
const BG2: ratatui::style::Color = theme::BG_PANEL;

fn clamp_ellipsis(text: &str, max_w: usize) -> String {
    if max_w == 0 {
        return String::new();
    }
    if text.chars().count() <= max_w {
        return text.to_string();
    }
    if max_w == 1 {
        return "…".to_string();
    }
    let kept: String = text.chars().take(max_w - 1).collect();
    format!("{}…", kept)
}

fn write_clamped(
    buf: &mut ratatui::buffer::Buffer,
    x: u16,
    y: u16,
    text: &str,
    width: u16,
    style: Style,
) {
    if width == 0 {
        return;
    }
    let clamped = clamp_ellipsis(text, width as usize);
    let _ = buf.set_stringn(x, y, clamped, width as usize, style);
}

fn required_field_label(label: &str, focused: bool) -> Span<'static> {
    let text = format!("{} * ", label);
    let style = if focused {
        Style::default().fg(FG).bg(theme::BG_HIGHLIGHT).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(FG).add_modifier(Modifier::BOLD)
    };
    Span::styled(text, style)
}

/// Service option for installing the gateway.
struct Service {
    #[allow(dead_code)] // staged UI scaffolding
    id: &'static str,
    name: &'static str,
    glyph: &'static str,
    color: ratatui::style::Color,
    sub: &'static str,
    path: Option<&'static str>,
}

const SERVICES: &[Service] = &[
    Service { id: "launchd", name: "launchd", glyph: "MAC", color: theme::FIRE_ORANGE, sub: "macOS native (recommended)", path: Some("~/Library/LaunchAgents/ai.zeuslab.gateway.plist") },
    Service { id: "systemd", name: "systemd", glyph: "LIN", color: theme::GREEN, sub: "Linux native", path: Some("/etc/systemd/system/zeus-gateway.service") },
    Service { id: "rcd", name: "rc.d", glyph: "BSD", color: theme::CYAN, sub: "FreeBSD native", path: Some("/usr/local/etc/rc.d/zeus_gateway") },
    Service { id: "manual", name: "Manual start", glyph: "—", color: theme::DIM, sub: "I'll start zeus manually", path: None },
];

/// Feature toggle entry.
struct Feature {
    #[allow(dead_code)] // staged UI scaffolding
    key: &'static str,
    label: &'static str,
    desc: &'static str,
    #[allow(dead_code)] // staged for follow-up: real toggle default wiring
    default: bool,
}

const FEATURES: &[Feature] = &[
    Feature { key: "agent_processing", label: "Agent Processing Loop", desc: "Background heartbeat + cron + watchdog", default: true },
    Feature { key: "webui", label: "WebUI Co-host", desc: "Serves Leptos frontend on the same port (or 8081 if 8080 is taken)", default: true },
    Feature { key: "mcp", label: "MCP Server", desc: "Model Context Protocol endpoint for Claude Desktop / cursor", default: false },
];

/// Gateway screen — step 8 of onboarding.
/// Matches JSX GatewayStep component (line 1191).
pub struct GatewayScreen {
    /// Host value
    pub host: String,
    /// Port value
    pub port: String,
    /// Which field is focused: 0=host, 1=port
    pub focused_field: usize,
    /// Feature toggles (agent_processing, webui, mcp)
    pub features: [bool; 3],
    /// Selected service mode index (0=launchd, 1=systemd, 2=rcd, 3=manual)
    pub service_mode: usize,
    /// Whether port 8080 is detected in use
    pub port_in_use: bool,
    /// Blink phase from `App::cursor_visible()` — drives the insertion cursor
    /// on the focused field (set by the caller each frame).
    pub cursor_on: bool,
}

impl Default for GatewayScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl GatewayScreen {
    pub fn new() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: "8080".to_string(),
            focused_field: 0,
            cursor_on: false,
            features: [true, true, false],
            service_mode: 0,
            port_in_use: false,
        }
    }

    pub fn move_up(&mut self) {
        if self.focused_field > 0 {
            self.focused_field -= 1;
        }
    }

    pub fn move_down(&mut self) {
        if self.focused_field < 1 {
            self.focused_field += 1;
        }
    }

    pub fn move_left(&mut self) {
        if self.service_mode > 0 {
            self.service_mode -= 1;
        }
    }

    pub fn move_right(&mut self) {
        if self.service_mode < SERVICES.len() - 1 {
            self.service_mode += 1;
        }
    }

    pub fn toggle_feature(&mut self, idx: usize) {
        if idx < self.features.len() {
            self.features[idx] = !self.features[idx];
        }
    }

    pub fn toggle_service(&mut self) {
        // Cycle through service modes
        self.service_mode = (self.service_mode + 1) % SERVICES.len();
    }
}

impl Widget for &GatewayScreen {
    fn render(self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        Clear.render(area, buf);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::BORDER))
            .style(Style::default().bg(BG2));
        let inner = block.inner(area);
        block.render(area, buf);

        let mut cy = inner.y;

        // ── BIND section ──
        let section_label = Line::from(vec![
            Span::styled("BIND", Style::default().fg(theme::ACCENT_DIM).add_modifier(Modifier::BOLD)),
        ]);
        buf.set_line(inner.x, cy, &section_label, inner.width);
        cy += 1;

        // Host field
        let host_focused = self.focused_field == 0;
        let host_label = required_field_label("Host", host_focused);
        let host_budget = inner.width.saturating_sub(8 + u16::from(self.cursor_on && host_focused));
        let host_value = Span::styled(
            clamp_ellipsis(&self.host, host_budget as usize),
            Style::default().fg(theme::FIRE_ORANGE),
        );
        let mut host_spans = vec![host_label, host_value];
        // Insertion cursor — focused field only, blink-gated.
        if self.cursor_on && host_focused && !self.host.is_empty() {
            host_spans.push(Span::styled("▏", Style::default().fg(theme::AMBER)));
        }
        let host_line = Line::from(host_spans);
        buf.set_line(inner.x, cy, &host_line, inner.width);
        cy += 1;

        // Host hint (JSX: hint="Use 0.0.0.0 to expose on LAN")
        let host_hint = Line::from(vec![Span::styled(
            clamp_ellipsis("Use 0.0.0.0 to expose on LAN", inner.width as usize),
            Style::default().fg(theme::DIM),
        )]);
        buf.set_line(inner.x, cy, &host_hint, inner.width);
        cy += 1;

        // Port field
        let port_focused = self.focused_field == 1;
        let port_label = required_field_label("Port", port_focused);
        let port_budget = inner.width.saturating_sub(8 + u16::from(self.cursor_on && port_focused));
        let port_value = Span::styled(
            clamp_ellipsis(&self.port, port_budget as usize),
            Style::default().fg(theme::FIRE_ORANGE),
        );
        let mut port_spans = vec![port_label, port_value];
        if self.cursor_on && port_focused && !self.port.is_empty() {
            port_spans.push(Span::styled("▏", Style::default().fg(theme::AMBER)));
        }
        let port_line = Line::from(port_spans);
        buf.set_line(inner.x, cy, &port_line, inner.width);
        cy += 1;

        // Port probe state: keep BIND validation visible even when the
        // current host/port is available, instead of rendering plain fields
        // until the error state appears.
        if self.port_in_use && self.port == "8080" {
            let warning_text = clamp_ellipsis(
                "⚠ Port 8080 in use by PID 47291 (zeus). Pick a different port or stop the existing instance.",
                inner.width as usize,
            );
            let warning = Line::from(vec![Span::styled(
                warning_text,
                Style::default().fg(theme::YELLOW),
            )]);
            buf.set_line(inner.x, cy, &warning, inner.width);
            cy += 1;
        } else {
            let prefix = "● PROBE ";
            let target = format!("{}:{} available", self.host, self.port);
            let budget = inner.width.saturating_sub(prefix.chars().count() as u16);
            let probe = Line::from(vec![
                Span::styled(prefix, Style::default().fg(theme::GREEN).add_modifier(Modifier::BOLD)),
                Span::styled(clamp_ellipsis(&target, budget as usize), Style::default().fg(theme::DIM)),
            ]);
            buf.set_line(inner.x, cy, &probe, inner.width);
            cy += 1;
        }

        cy += 1; // spacing

        // ── FEATURES section ──
        let features_label = Line::from(vec![
            Span::styled("FEATURES", Style::default().fg(theme::ACCENT_DIM).add_modifier(Modifier::BOLD)),
        ]);
        buf.set_line(inner.x, cy, &features_label, inner.width);
        cy += 1;

        for (idx, feature) in FEATURES.iter().enumerate() {
            let enabled = self.features[idx];
            let toggle_bg = if enabled { theme::FIRE_ORANGE } else { theme::DARK };
            let toggle_fg = if enabled { theme::BG } else { theme::DIM };
            let toggle_text = if enabled { "[ON] " } else { "[OFF]" };

            let toggle_span = Span::styled(toggle_text, Style::default().fg(toggle_fg).bg(toggle_bg).add_modifier(Modifier::BOLD));
            let label_text = format!(" {}", feature.label);
            let label_budget = inner.width.saturating_sub(toggle_text.chars().count() as u16);
            let label_span = Span::styled(
                clamp_ellipsis(&label_text, label_budget as usize),
                Style::default().fg(FG).add_modifier(Modifier::BOLD),
            );
            let used = toggle_text.chars().count() + label_text.chars().count().min(label_budget as usize);
            let desc_budget = (inner.width as usize).saturating_sub(used);
            let desc_span = Span::styled(
                clamp_ellipsis(&format!(" — {}", feature.desc), desc_budget),
                Style::default().fg(theme::DIM),
            );

            let line = Line::from(vec![toggle_span, label_span, desc_span]);
            buf.set_line(inner.x, cy, &line, inner.width);
            cy += 1;
        }

        cy += 1; // spacing

        // ── INSTALL AS SERVICE section ──
        let service_label = Line::from(vec![
            Span::styled("INSTALL AS SERVICE", Style::default().fg(theme::ACCENT_DIM).add_modifier(Modifier::BOLD)),
        ]);
        buf.set_line(inner.x, cy, &service_label, inner.width);
        cy += 1;

        // Service cards in a row
        let gap = 2;
        let card_width = inner.width.saturating_sub(gap * 3) / 4;
        for (idx, service) in SERVICES.iter().enumerate() {
            let x = inner.x + (idx as u16 * (card_width + gap));
            if card_width < 4 || x >= inner.x.saturating_add(inner.width) {
                continue;
            }
            let selected = idx == self.service_mode;
            let card_border = if selected { service.color } else { theme::BORDER };
            let card_bg = if selected { theme::BG_HIGHLIGHT } else { BG2 };

            let card_block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(card_border))
                .style(Style::default().bg(card_bg));
            let card_inner = card_block.inner(Rect::new(x, cy, card_width, 5));
            card_block.render(Rect::new(x, cy, card_width, 5), buf);

            // Glyph
            write_clamped(
                buf,
                card_inner.x,
                card_inner.y,
                service.glyph,
                card_inner.width,
                Style::default().fg(service.color).add_modifier(Modifier::BOLD),
            );
            // Name
            write_clamped(
                buf,
                card_inner.x,
                card_inner.y + 1,
                service.name,
                card_inner.width,
                Style::default().fg(FG).add_modifier(Modifier::BOLD),
            );
            // Subtitle
            write_clamped(
                buf,
                card_inner.x,
                card_inner.y + 2,
                service.sub,
                card_inner.width,
                Style::default().fg(theme::DIM),
            );
        }

        cy += 6;

        // Show install path for selected service
        let selected_service = &SERVICES[self.service_mode];
        if let Some(path) = selected_service.path {
            let label = "WILL INSTALL ";
            let path_budget = inner.width.saturating_sub(label.chars().count() as u16);
            let will_install = Line::from(vec![
                Span::styled(label, Style::default().fg(theme::ACCENT_DIM).add_modifier(Modifier::BOLD)),
                Span::styled(
                    clamp_ellipsis(path, path_budget as usize),
                    Style::default().fg(theme::ACCENT_BRIGHT),
                ),
            ]);
            buf.set_line(inner.x, cy, &will_install, inner.width);
        }

        // Footer hint
        let hint_y = inner.y + inner.height.saturating_sub(1);
        let hint = Line::from(vec![Span::raw(clamp_ellipsis(
            "↑↓←→ select service  •  ↵ toggle service  •  1/2/3 toggle feature  •  ^N continue",
            inner.width as usize,
        ))]);
        buf.set_line(inner.x, hint_y, &hint, inner.width);
    }
}

#[cfg(test)]
mod cursor_tests {
    use super::*;
    use ratatui::buffer::Buffer;
    use ratatui::{backend::TestBackend, Terminal};

    fn render_buffer(focused: usize, cursor_on: bool, width: u16, height: u16) -> Buffer {
        let mut term = Terminal::new(TestBackend::new(width, height)).unwrap();
        term.draw(|f| {
            let mut s = GatewayScreen::new();
            s.focused_field = focused;
            s.cursor_on = cursor_on;
            (&s).render(f.area(), f.buffer_mut());
        })
        .unwrap();
        term.backend().buffer().clone()
    }

    fn buffer_dump(buf: &Buffer) -> String {
        let mut out = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    fn render(focused: usize, cursor_on: bool) -> String {
        buffer_dump(&render_buffer(focused, cursor_on, 120, 40))
    }

    fn assert_gateway_render_clamped(width: u16, height: u16) {
        let dump = buffer_dump(&render_buffer(0, true, width, height));
        assert!(
            dump.contains("BIND"),
            "bind label must render at {width}x{height}:\n{dump}"
        );
        assert!(
            dump.contains("Host * 127.0.0.1▏"),
            "host field + required marker + cursor must render at {width}x{height}:\n{dump}"
        );
        assert!(
            dump.contains("Port * 8080"),
            "port field + required marker must render at {width}x{height}:\n{dump}"
        );
        assert!(
            dump.contains("FEATURES"),
            "features section must render at {width}x{height}:\n{dump}"
        );
        assert!(
            dump.contains("[ON]  Agent Processing Loop"),
            "real feature label must render at {width}x{height}:\n{dump}"
        );
        assert!(
            dump.contains("[OFF] MCP Server"),
            "disabled real feature label must render at {width}x{height}:\n{dump}"
        );
        assert!(
            dump.contains("INSTALL AS SERVICE"),
            "service section must render at {width}x{height}:\n{dump}"
        );
        assert!(
            dump.contains("launchd") && dump.contains("systemd") && dump.contains("rc.d"),
            "service card names must render at {width}x{height}:\n{dump}"
        );
        assert!(
            dump.contains("WILL INSTALL ~/Library/LaunchAgents/ai.zeuslab")
                && (dump.contains("ai.zeuslab.gateway.plist") || dump.contains("…")),
            "install preview must render with honest ellipsis if narrow at {width}x{height}:\n{dump}"
        );
        for line in dump.lines() {
            assert!(
                line.chars().count() <= width as usize,
                "render line exceeded terminal width {width}: {} chars: `{line}`\n{dump}",
                line.chars().count()
            );
        }
    }


    #[test]
    fn gateway_render_clamps_at_narrow_width() {
        assert_gateway_render_clamped(56, 32);
    }

    #[test]
    fn gateway_render_clamps_at_normal_width() {
        assert_gateway_render_clamped(100, 36);
    }

    #[test]
    fn cursor_painted_on_focused_field_during_blink() {
        assert!(
            render(0, true).contains('▏'),
            "expected cursor `▏` on focused host field during blink-on"
        );
    }

    #[test]
    fn cursor_hidden_on_blink_off() {
        assert!(
            !render(0, false).contains('▏'),
            "expected no cursor `▏` during blink-off half-cycle"
        );
    }

    #[test]
    fn cursor_follows_focus() {
        // host (0) and port (1) both have non-empty defaults → each focus
        // index paints its caret on blink-on.
        for f in 0..2 {
            assert!(
                render(f, true).contains('▏'),
                "expected cursor on focused field {f}"
            );
        }
    }
}
