use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Block, Borders, Widget};

use crate::theme;

/// Clamp `text` to `max_w` display columns, appending `…` on truncation
/// (mirrors the #271 model/fallback/memory idiom — honest ellipsis, never
/// a bare mid-word chop). Char-safe: counts/takes `chars`, never byte slices.
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

// Theme aliases
const FG: ratatui::style::Color = theme::TEXT;
const BG2: ratatui::style::Color = theme::BG_PANEL;

/// Voice provider entry — matches JSX VOICE_PROVIDERS (line 150).
struct VoiceProvider {
    id: &'static str,
    name: &'static str,
    glyph: &'static str,
    color: ratatui::style::Color,
    sub: &'static str,
}

/// All 5 voice providers from the JSX prototype.
const VOICE_PROVIDERS: &[VoiceProvider] = &[
    VoiceProvider {
        id: "elevenlabs",
        name: "ElevenLabs",
        glyph: "11L",
        color: theme::ACCENT,
        sub: "Premium quality voices",
    },
    VoiceProvider {
        id: "openai-tts",
        name: "OpenAI TTS",
        glyph: "OAI",
        color: theme::GREEN,
        sub: "Native multimodal",
    },
    VoiceProvider {
        id: "edge",
        name: "Edge TTS",
        glyph: "EDG",
        color: theme::CYAN,
        sub: "Microsoft edge-tts (free)",
    },
    VoiceProvider {
        id: "custom",
        name: "Custom Endpoint",
        glyph: "API",
        color: theme::AMBER,
        sub: "Self-hosted Piper / Kokoro",
    },
    VoiceProvider {
        id: "none",
        name: "Skip",
        glyph: "—",
        color: theme::DIM,
        sub: "No voice configured",
    },
];

/// Config field definition for the right panel.
struct ConfigField {
    key: &'static str,
    label: &'static str,
    placeholder: &'static str,
    secret: bool,
    required: bool,
}

/// Standard credential fields (API providers: ElevenLabs / OpenAI TTS / Edge).
/// JSX shape: API Key (secret, required) + Voice ID (default "default").
const STD_FIELDS: &[ConfigField] = &[
    ConfigField {
        key: "api_key",
        label: "API Key",
        placeholder: "...",
        secret: true,
        required: true,
    },
    ConfigField {
        key: "voice_id",
        label: "Voice ID",
        placeholder: "default",
        secret: false,
        required: false,
    },
];

/// Custom-endpoint fields — JSX adds a required Base URL (Piper / Kokoro).
const CUSTOM_FIELDS: &[ConfigField] = &[
    ConfigField {
        key: "api_key",
        label: "API Key",
        placeholder: "...",
        secret: true,
        required: true,
    },
    ConfigField {
        key: "voice_id",
        label: "Voice ID",
        placeholder: "default",
        secret: false,
        required: false,
    },
    ConfigField {
        key: "base_url",
        label: "Base URL",
        placeholder: "http://localhost:5000",
        secret: false,
        required: true,
    },
];

/// Field set for a given provider id. Custom adds Base URL; others use STD.
fn fields_for(provider_id: &str) -> &'static [ConfigField] {
    match provider_id {
        "custom" => CUSTOM_FIELDS,
        _ => STD_FIELDS,
    }
}

/// Test button state.
#[derive(Clone, Copy, PartialEq)]
enum TestStatus {
    Idle,
    Testing,
    Success,
    #[allow(dead_code)] // staged UI scaffolding
    Failed,
}

/// Voice screen state — step 13, id: "voice", code: VOIC.
pub struct VoiceScreen {
    /// Index of the currently selected voice provider.
    selected: usize,
    /// Config field values (key -> value).
    values: std::collections::HashMap<String, String>,
    /// Currently focused config field index.
    focused_field: usize,
    /// Test button status.
    test_status: TestStatus,
}

impl Default for VoiceScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl VoiceScreen {
    /// Id of the currently selected voice provider (e.g. "elevenlabs", "none").
    pub fn selected_id(&self) -> &'static str {
        VOICE_PROVIDERS[self.selected].id
    }

    /// Credential field set for the current provider (custom adds Base URL).
    fn fields(&self) -> &'static [ConfigField] {
        fields_for(self.selected_id())
    }

    /// Clamp the focused-field index into the current provider's field set.
    /// Required because custom (3 fields) → std (2 fields) can leave the
    /// focus index dangling past the end after a provider switch.
    fn clamp_focus(&mut self) {
        let n = self.fields().len();
        if self.focused_field >= n {
            self.focused_field = n.saturating_sub(1);
        }
    }

    pub fn new() -> Self {
        Self {
            selected: 0,
            values: std::collections::HashMap::new(),
            focused_field: 0,
            test_status: TestStatus::Idle,
        }
    }

    /// Move selection up in the provider list.
    pub fn select_prev(&mut self) {
        self.selected = self.selected.saturating_add(VOICE_PROVIDERS.len() - 1) % VOICE_PROVIDERS.len();
        self.clamp_focus();
    }

    /// Move selection down in the provider list.
    pub fn select_next(&mut self) {
        self.selected = (self.selected + 1) % VOICE_PROVIDERS.len();
        self.clamp_focus();
    }

    /// Move focus to previous config field.
    pub fn focus_prev(&mut self) {
        let n = self.fields().len();
        self.focused_field = self.focused_field.saturating_add(n - 1) % n;
    }

    /// Number of Tab-focusable config fields (drives the App footer Tab cursor).
    pub fn field_count(&self) -> usize {
        self.fields().len()
    }

    /// Move focus to next config field.
    pub fn focus_next(&mut self) {
        let n = self.fields().len();
        self.focused_field = (self.focused_field + 1) % n;
    }

    /// Set value for the currently focused field.
    pub fn set_current_value(&mut self, value: String) {
        let field = &self.fields()[self.focused_field];
        self.values.insert(field.key.to_string(), value);
    }

    /// Append a typed char to the focused config field. Uses `String::push`
    /// (char-aware) so multibyte paste can never split a code point.
    pub fn input_char(&mut self, c: char) {
        let key = self.fields()[self.focused_field].key.to_string();
        self.values.entry(key).or_default().push(c);
    }

    /// Delete the last char of the focused config field. `String::pop` removes
    /// a whole `char`, never a partial UTF-8 byte.
    pub fn input_backspace(&mut self) {
        let key = self.fields()[self.focused_field].key;
        if let Some(v) = self.values.get_mut(key) {
            v.pop();
        }
    }

    /// Get value for a field key.
    fn get_value(&self, key: &str) -> &str {
        self.values.get(key).map(|s| s.as_str()).unwrap_or("")
    }

    /// Trigger test voice.
    pub fn test_voice(&mut self) {
        let provider = &VOICE_PROVIDERS[self.selected];
        if provider.id == "none" {
            return;
        }
        self.test_status = TestStatus::Testing;
        // In real implementation, this would call the voice API.
        // For now, simulate success after a brief delay.
        self.test_status = TestStatus::Success;
    }

    /// Render the voice screen.
    pub fn render(&self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        Clear.render(area, buf);
        // Outer block with opaque bg
        Block::default()
            .style(Style::default().bg(theme::BG))
            .render(area, buf);

        // Main layout: left (providers) | right (config)
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(40),
                Constraint::Percentage(60),
            ])
            .split(area);

        self.render_providers(chunks[0], buf);
        self.render_config(chunks[1], buf);
    }

    /// Render left panel: provider list.
    fn render_providers(&self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        let provider = &VOICE_PROVIDERS[self.selected];

        // Header
        let header_lines = [Line::from(vec![
                Span::styled("Voice / TTS provider", Style::default().fg(FG).add_modifier(Modifier::BOLD)),
            ]),
            Line::from(vec![
                Span::raw("Powers "),
                Span::styled("voice_say", Style::default().fg(theme::ACCENT_BRIGHT)),
                Span::raw(", "),
                Span::styled("voice_call", Style::default().fg(theme::ACCENT_BRIGHT)),
                Span::raw(", and Twilio outbound calls."),
            ])];

        let header_area = Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: 4,
        };

        for (i, line) in header_lines.iter().enumerate() {
            if header_area.y + (i as u16) < area.y + area.height {
                buf.set_line(header_area.x, header_area.y + i as u16, line, header_area.width);
            }
        }

        // Provider list
        let list_area = Rect {
            x: area.x,
            y: area.y + 4,
            width: area.width,
            height: area.height - 4,
        };

        for (i, p) in VOICE_PROVIDERS.iter().enumerate() {
            let y = list_area.y + i as u16 * 3;
            if y + 2 > list_area.y + list_area.height {
                break;
            }

            let is_selected = i == self.selected;
            let border_color = if is_selected { provider.color } else { theme::BORDER };
            let bg = if is_selected { BG2 } else { theme::BG };

            // Card block
            let card_area = Rect {
                x: list_area.x,
                y,
                width: list_area.width,
                height: 3,
            };

            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color))
                .style(Style::default().bg(bg))
                .render(card_area, buf);

            // Glyph box
            let glyph_area = Rect {
                x: card_area.x + 1,
                y: card_area.y + 1,
                width: 4,
                height: 1,
            };

            Block::default()
                .style(Style::default().bg(p.color).fg(theme::BG))
                .render(glyph_area, buf);

            let glyph_line = Line::from(vec![
                Span::styled(p.glyph, Style::default().fg(theme::BG).add_modifier(Modifier::BOLD)),
            ]);
            buf.set_line(glyph_area.x, glyph_area.y, &glyph_line, glyph_area.width);

            // Name + sub
            let text_area = Rect {
                x: card_area.x + 6,
                y: card_area.y + 1,
                width: card_area.width - 7,
                height: 1,
            };

            let name_style = if is_selected {
                Style::default().fg(theme::WHITE).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(FG)
            };

            // Name flows left (rarely long); sub gets the remainder and
            // clamps with ellipsis so it never hard-chops mid-word at narrow
            // card widths (e.g. "Microsoft edge-tts (free)").
            let name_w = p.name.chars().count();
            let sub_budget = (text_area.width as usize).saturating_sub(name_w + 2);
            let sub_clamped = clamp_ellipsis(p.sub, sub_budget);
            let text_line = Line::from(vec![
                Span::styled(p.name, name_style),
                Span::raw("  "),
                Span::styled(sub_clamped, Style::default().fg(theme::DIM)),
            ]);
            buf.set_line(text_area.x, text_area.y, &text_line, text_area.width);
        }
    }

    /// Render right panel: config fields or "none" warning.
    fn render_config(&self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        let provider = &VOICE_PROVIDERS[self.selected];

        // Left border separator
        let separator_area = Rect {
            x: area.x,
            y: area.y,
            width: 1,
            height: area.height,
        };

        for y in 0..area.height {
            buf.set_string(separator_area.x, separator_area.y + y, "│", Style::default().fg(theme::BORDER));
        }

        let content_area = Rect {
            x: area.x + 2,
            y: area.y,
            width: area.width - 2,
            height: area.height,
        };

        if provider.id == "none" {
            self.render_none_warning(content_area, buf);
        } else {
            self.render_provider_config(content_area, buf, provider);
        }
    }

    /// Render "none" provider warning.
    fn render_none_warning(&self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        let warning_area = Rect {
            x: area.x,
            y: area.y + 2,
            width: area.width,
            height: 4,
        };

        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::YELLOW))
            .style(Style::default().bg(BG2))
            .render(warning_area, buf);

        let lines = [Line::from(vec![
                Span::styled("⚠ NO VOICE CONFIGURED", Style::default().fg(theme::YELLOW).add_modifier(Modifier::BOLD)),
            ]),
            Line::from(vec![
                Span::raw("Voice tools will be unavailable. Re-run "),
                Span::styled("zeus onboard --resume voice", Style::default().fg(theme::ACCENT_BRIGHT)),
                Span::raw(" later."),
            ])];

        for (i, line) in lines.iter().enumerate() {
            if warning_area.y + 1 + (i as u16) < warning_area.y + warning_area.height - 1 {
                buf.set_line(warning_area.x + 2, warning_area.y + 1 + i as u16, line, warning_area.width - 4);
            }
        }
    }

    /// Render provider config fields.
    fn render_provider_config(&self, area: Rect, buf: &mut ratatui::buffer::Buffer, provider: &VoiceProvider) {
        // Provider header
        let header_area = Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: 4,
        };

        // Glyph box
        let glyph_area = Rect {
            x: header_area.x,
            y: header_area.y + 1,
            width: 4,
            height: 1,
        };

        Block::default()
            .style(Style::default().bg(provider.color).fg(theme::BG))
            .render(glyph_area, buf);

        let glyph_line = Line::from(vec![
            Span::styled(provider.glyph, Style::default().fg(theme::BG).add_modifier(Modifier::BOLD)),
        ]);
        buf.set_line(glyph_area.x, glyph_area.y, &glyph_line, glyph_area.width);

        // Name + sub
        let name_area = Rect {
            x: header_area.x + 6,
            y: header_area.y + 1,
            width: header_area.width - 6,
            height: 1,
        };

        let name_line = Line::from(vec![
            Span::styled(provider.name, Style::default().fg(theme::WHITE).add_modifier(Modifier::BOLD)),
        ]);
        buf.set_line(name_area.x, name_area.y, &name_line, name_area.width);

        let sub_clamped = clamp_ellipsis(provider.sub, name_area.width as usize);
        let sub_line = Line::from(vec![
            Span::styled(sub_clamped, Style::default().fg(theme::DIM)),
        ]);
        buf.set_line(name_area.x, name_area.y + 1, &sub_line, name_area.width);

        // CREDENTIALS label
        let cred_label_area = Rect {
            x: area.x,
            y: area.y + 5,
            width: area.width,
            height: 1,
        };

        let cred_label = Line::from(vec![
            Span::styled("CREDENTIALS", Style::default().fg(theme::ACCENT_DIM).add_modifier(Modifier::BOLD)),
        ]);
        buf.set_line(cred_label_area.x, cred_label_area.y, &cred_label, cred_label_area.width);

        // Config fields
        let fields_start_y = area.y + 7;
        for (i, field) in self.fields().iter().enumerate() {
            let field_y = fields_start_y + i as u16 * 3;
            if field_y + 2 > area.y + area.height {
                break;
            }

            let is_focused = i == self.focused_field;
            let value = self.get_value(field.key);
            // Char-safe `***{last4}` masking (same idiom as Auth/ChanConfig):
            // `chars().rev().take(4)` can never split a multibyte code point,
            // unlike a byte-slice on `value[len-4..]`.
            let display_value: String = if field.secret && !value.is_empty() {
                let last4: String = value.chars().rev().take(4).collect::<Vec<_>>()
                    .into_iter().rev().collect();
                format!("***{}", last4)
            } else if value.is_empty() {
                field.placeholder.to_string()
            } else {
                value.to_string()
            };

            // Field label
            let label_line = Line::from(vec![
                Span::styled(field.label, Style::default().fg(FG)),
                if field.required {
                    Span::styled(" *", Style::default().fg(theme::ACCENT))
                } else {
                    Span::raw("")
                },
            ]);
            buf.set_line(area.x, field_y, &label_line, area.width);

            // Field input box
            let input_area = Rect {
                x: area.x,
                y: field_y + 1,
                width: area.width,
                height: 1,
            };

            let border_color = if is_focused { theme::ACCENT } else { theme::BORDER };
            let bg = if is_focused { BG2 } else { theme::BG };

            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color))
                .style(Style::default().bg(bg))
                .render(input_area, buf);

            // Clamp the value to the input-box interior so a long Base URL
            // (e.g. http://localhost:5000) truncates with ellipsis inside the
            // border instead of hard-chopping at the box edge.
            let value_budget = (input_area.width as usize).saturating_sub(2);
            let value_clamped = clamp_ellipsis(&display_value, value_budget);
            let value_line = Line::from(vec![
                Span::styled(value_clamped, Style::default().fg(if value.is_empty() { theme::DIM } else { FG })),
            ]);
            buf.set_line(input_area.x + 1, input_area.y, &value_line, input_area.width - 2);
        }

        // Test button
        let test_y = fields_start_y + self.fields().len() as u16 * 3 + 1;
        if test_y + 1 < area.y + area.height {
            let test_area = Rect {
                x: area.x,
                y: test_y,
                width: 20,
                height: 1,
            };

            let (test_label, test_color) = match self.test_status {
                TestStatus::Idle => ("▸ TEST VOICE", theme::ACCENT),
                TestStatus::Testing => ("◌ TESTING...", theme::AMBER),
                TestStatus::Success => ("✓ SUCCESS", theme::GREEN),
                TestStatus::Failed => ("✕ FAILED", theme::RED),
            };

            let test_line = Line::from(vec![
                Span::styled(test_label, Style::default().fg(test_color).add_modifier(Modifier::BOLD)),
            ]);
            buf.set_line(test_area.x, test_area.y, &test_line, test_area.width);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;

    /// Render the screen into a fresh Buffer of the given width and return it
    /// as a flat string per row (joined by '\n'). Mirrors the #271
    /// render-verify-at-two-widths bar — the screen has no built-in harness,
    /// so we drive it through a sized Buffer directly.
    fn render_at_width(screen: &VoiceScreen, w: u16, h: u16) -> Vec<String> {
        let area = Rect { x: 0, y: 0, width: w, height: h };
        let mut buf = Buffer::empty(area);
        screen.render(area, &mut buf);
        (0..h)
            .map(|y| {
                (0..w)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect()
    }

    /// Extract only the left (provider-list) column — the first `split_at`
    /// chars of each row. The 40%/60% layout puts the card list on the left;
    /// the config detail panel (more room) is to the right. Card borders `│`
    /// live INSIDE this column, so we slice by column index, not by `│`.
    fn left_column(rows: &[String], split_at: usize) -> String {
        rows.iter()
            .map(|r| r.chars().take(split_at).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn clamp_ellipsis_truncates_with_marker_not_midword_chop() {
        assert_eq!(clamp_ellipsis("Microsoft edge-tts (free)", 25), "Microsoft edge-tts (free)");
        assert_eq!(clamp_ellipsis("Microsoft edge-tts (free)", 8), "Microso…");
        assert_eq!(clamp_ellipsis("hello", 0), "");
        assert_eq!(clamp_ellipsis("hello", 1), "…");
        // Never emits more columns than the budget.
        for w in 1..30 {
            assert!(clamp_ellipsis("Self-hosted Piper / Kokoro", w).chars().count() <= w);
        }
    }

    #[test]
    fn narrow_width_card_sub_clamps_with_ellipsis() {
        // At a narrow width the left card column (40% of 56 = ~22 cols) is too
        // tight for "Premium quality voices"; the card sub must end in '…',
        // NOT a bare mid-word chop, and must NOT bleed past the card border
        // into the config column.
        let s = VoiceScreen::new(); // index 0 = ElevenLabs, sub 23 chars
        assert_eq!(s.selected_id(), "elevenlabs");
        let rows = render_at_width(&s, 56, 24);
        let split = (rows.first().map(|r| r.chars().count()).unwrap_or(0) * 40) / 100;
        let left = left_column(&rows, split);
        // The full untruncated sub must NOT appear in the card column.
        assert!(
            !left.contains("Premium quality voices"),
            "card sub should be clamped at narrow width, got full string in left column:\n{left}"
        );
        // An ellipsis must be present in the card column (honest truncation).
        assert!(
            left.contains('…'),
            "expected an ellipsis marker in the card column at narrow width:\n{left}"
        );
    }

    #[test]
    fn normal_width_card_sub_renders_in_full() {
        // At a comfortable width the full card sub renders — no premature clamp.
        let s = VoiceScreen::new();
        let rows = render_at_width(&s, 140, 24);
        let split = (rows.first().map(|r| r.chars().count()).unwrap_or(0) * 40) / 100;
        let left = left_column(&rows, split);
        assert!(
            left.contains("Premium quality voices"),
            "full card sub should render at normal width, no premature clamp:\n{left}"
        );
    }
}

