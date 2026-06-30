use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::theme;

/// 19 onboarding steps — matches STEPS array in zeus-tui-onboarding.jsx line 74.
pub const STEPS: &[(&str, &str)] = &[
    ("welcome", "WLCM"),
    ("mode", "MODE"),
    ("provider", "PROV"),
    ("auth", "AUTH"),
    ("model", "MODL"),
    ("backup", "BLLM"),
    ("channels", "CHNL"),
    ("channel_config", "CCFG"),
    ("gateway", "GWAY"),
    ("agent", "AGNT"),
    ("workspace", "WORK"),
    ("security", "SECU"),
    ("features", "FEAT"),
    ("voice", "VOIC"),
    ("images", "IMGS"),
    ("orchestration", "ORCH"),
    ("memory", "MNEM"),
    ("skills", "SKIL"),
    ("complete", "DONE"),
];

/// TopBar widget — matches JSX TopBar (line 206).
/// Renders: `ZEUS │ ONBOARDING │ Step N of 19 │ CODE │ [face] │ ● config draft │ ~/.zeus/config.toml`
pub struct TopBar {
    pub step_idx: usize,
    pub hostname: String,
}

impl Widget for TopBar {
    fn render(self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        if area.height == 0 {
            return;
        }

        // Fill background
        for x in area.left()..area.right() {
            buf[(x, area.y)]
                .set_symbol(" ")
                .set_style(Style::default().bg(theme::BG_PANEL));
        }

        // ZEUS
        let mut spans = vec![
            Span::styled(
                " ZEUS ",
                Style::default()
                    .fg(theme::FIRE_ORANGE)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("│ ", Style::default().fg(theme::MUTED)),
        ];

        // ONBOARDING
        spans.push(Span::styled(
            "ONBOARDING ",
            Style::default()
                .fg(theme::ACCENT_DIM)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled("│ ", Style::default().fg(theme::MUTED)));

        // Step N of 19
        let step_text = format!("Step {} of {} ", step_idx_display(self.step_idx), STEPS.len());
        spans.push(Span::styled(step_text, Style::default().fg(theme::DIM)));
        spans.push(Span::styled("│ ", Style::default().fg(theme::MUTED)));

        // Step code
        if let Some((_, code)) = STEPS.get(self.step_idx) {
            spans.push(Span::styled(
                format!("{} ", code),
                Style::default().fg(theme::ACCENT_DIM).add_modifier(Modifier::BOLD),
            ));
        }

        // Right side: face placeholder + status
        spans.push(Span::raw("  "));
        spans.push(Span::styled("│ ", Style::default().fg(theme::MUTED)));
        spans.push(Span::styled("● ", Style::default().fg(theme::GREEN)));
        spans.push(Span::styled("config draft ", Style::default().fg(theme::DIM)));
        spans.push(Span::styled("│ ", Style::default().fg(theme::MUTED)));
        spans.push(Span::styled(
            "~/.zeus/config.toml",
            Style::default().fg(theme::DIM),
        ));

        let line = Line::from(spans);
        buf.set_line(area.x, area.y, &line, area.width);
    }
}

fn step_idx_display(idx: usize) -> usize {
    idx + 1
}
