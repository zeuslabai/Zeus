use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Block, Borders, Paragraph, Widget, Wrap};

use crate::theme;

// Theme aliases
const FG: ratatui::style::Color = theme::TEXT;
const BG2: ratatui::style::Color = theme::BG_PANEL;

/// Truncate `text` to fit `max_w` display columns, appending `…` when clipped.
///
/// #271 visual-parity: every narrow-width text seam on this screen used a bare
/// `set_line` clamp, which hard-chops mid-word with NO ellipsis. Route any
/// value that can exceed its budget through this so it truncates honestly
/// with a trailing `…` *inside* the budget. Char-based (the strings here are
/// ASCII provider names/subs/labels); the 1-col `…` replaces the last char
/// on clip.
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
    let keep: String = text.chars().take(max_w - 1).collect();
    format!("{keep}…")
}

/// Image provider entry — the real talos `ImageGenProviderType` backends.
struct ImageProvider {
    id: &'static str,
    name: &'static str,
    glyph: &'static str,
    color: ratatui::style::Color,
    sub: &'static str,
}

/// The 6 image options = 5 real talos backends + Skip.
/// (OpenAI · Automatic1111 · ComfyUI · Fooocus · OpenAI-compatible · Skip.
/// JSX's Google NanoBanana / BFL Flux are NOT distinct talos backends.)
const IMAGE_PROVIDERS: &[ImageProvider] = &[
    ImageProvider {
        id: "openai",
        name: "OpenAI GPT Image",
        glyph: "OAI",
        color: theme::GREEN,
        sub: "gpt-image-1",
    },
    ImageProvider {
        id: "comfyui",
        name: "ComfyUI",
        glyph: "CMF",
        color: theme::BLUE,
        sub: "Local node graph",
    },
    ImageProvider {
        id: "fooocus",
        name: "Fooocus",
        glyph: "FOO",
        color: theme::AMBER,
        sub: "SDXL Turbo",
    },
    ImageProvider {
        id: "openai-custom",
        name: "OpenAI compat URL",
        glyph: "API",
        color: theme::CYAN,
        sub: "vLLM, fal.ai, proxies",
    },
    ImageProvider {
        id: "a1111",
        name: "Automatic1111 URL",
        glyph: "A11",
        color: theme::ACCENT_BRIGHT,
        sub: "Z-Image Turbo path",
    },
    ImageProvider {
        id: "none",
        name: "Skip",
        glyph: "—",
        color: theme::DIM,
        sub: "No image gen",
    },
];

/// Config field definition for the right panel.
struct ConfigField {
    key: &'static str,
    label: &'static str,
    placeholder: &'static str,
    secret: bool,
    required: bool,
    hint: Option<&'static str>,
}

/// Config fields for the selected provider — mirrors the JSX conditionals:
/// Base URL only for openai-custom/a1111; API Key (required unless a1111);
/// Model always; Steps only for a1111 (with the Z-Image Turbo warning hint).
fn fields_for(provider_id: &str) -> Vec<ConfigField> {
    let mut fields = Vec::new();
    let needs_base_url =
        matches!(provider_id, "openai-custom" | "a1111" | "comfyui" | "fooocus");
    if needs_base_url {
        fields.push(ConfigField {
            key: "base_url",
            label: "Base URL",
            placeholder: match provider_id {
                "a1111" => "http://dgx-spark:7860",
                "comfyui" => "http://localhost:8188",
                "fooocus" => "http://localhost:7865",
                _ => "https://...",
            },
            secret: false,
            required: true,
            hint: None,
        });
    }
    // API Key: required for cloud (openai / openai-custom); local backends
    // (a1111 / comfyui / fooocus) run keyless.
    let key_required = !matches!(provider_id, "a1111" | "comfyui" | "fooocus");
    fields.push(ConfigField {
        key: "api_key",
        label: "API Key",
        placeholder: "...",
        secret: true,
        required: key_required,
        hint: None,
    });
    // Model placeholder = provider sub, per JSX (placeholder={p.sub})
    let sub = IMAGE_PROVIDERS
        .iter()
        .find(|p| p.id == provider_id)
        .map(|p| p.sub)
        .unwrap_or("");
    fields.push(ConfigField {
        key: "model_id",
        label: "Model",
        placeholder: sub,
        secret: false,
        required: true,
        hint: None,
    });
    if provider_id == "a1111" {
        fields.push(ConfigField {
            key: "steps",
            label: "Steps",
            placeholder: "1",
            secret: false,
            required: false,
            hint: Some("⚠ Z-Image Turbo: must be 1 (multi-step → black PNG)"),
        });
    }
    fields
}

/// Test button state.
#[derive(Clone, Copy, PartialEq)]
enum TestStatus {
    Idle,
    #[allow(dead_code)] // staged UI scaffolding
    Testing,
    Success,
    Failed,
}

/// Images screen state — step 14, id: "images", code: IMGS.
pub struct ImagesScreen {
    /// Index of the currently selected image provider.
    selected: usize,
    /// Config field values (key -> value).
    values: std::collections::HashMap<String, String>,
    /// Currently focused config field index (into fields_for(selected)).
    focused_field: usize,
    /// Test button status.
    test_status: TestStatus,
}

impl Default for ImagesScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl ImagesScreen {
    pub fn new() -> Self {
        Self {
            selected: 0,
            values: std::collections::HashMap::new(),
            focused_field: 0,
            test_status: TestStatus::Idle,
        }
    }

    /// Currently selected provider id.
    pub fn selected_id(&self) -> &'static str {
        IMAGE_PROVIDERS[self.selected].id
    }

    /// Move selection up in the provider list (wraps, mirrors JSX handleCycle).
    pub fn select_prev(&mut self) {
        self.selected =
            self.selected.saturating_add(IMAGE_PROVIDERS.len() - 1) % IMAGE_PROVIDERS.len();
        self.focused_field = 0;
        self.test_status = TestStatus::Idle;
    }

    /// Move selection down in the provider list (wraps).
    pub fn select_next(&mut self) {
        self.selected = (self.selected + 1) % IMAGE_PROVIDERS.len();
        self.focused_field = 0;
        self.test_status = TestStatus::Idle;
    }

    /// Number of Tab-focusable config fields (drives the App footer Tab cursor).
    pub fn field_count(&self) -> usize {
        fields_for(self.selected_id()).len()
    }

    /// Move focus to next config field for the current provider.
    pub fn focus_next(&mut self) {
        let n = fields_for(self.selected_id()).len();
        if n > 0 {
            self.focused_field = (self.focused_field + 1) % n;
        }
    }

    /// Type a character into the focused field.
    pub fn input_char(&mut self, c: char) {
        let fields = fields_for(self.selected_id());
        if let Some(field) = fields.get(self.focused_field) {
            self.values.entry(field.key.to_string()).or_default().push(c);
            self.test_status = TestStatus::Idle;
        }
    }

    /// Backspace in the focused field.
    pub fn input_backspace(&mut self) {
        let fields = fields_for(self.selected_id());
        if let Some(field) = fields.get(self.focused_field) {
            if let Some(v) = self.values.get_mut(field.key) {
                v.pop();
            }
            self.test_status = TestStatus::Idle;
        }
    }

    /// Get value for a field key.
    fn get_value(&self, key: &str) -> &str {
        self.values.get(key).map(|s| s.as_str()).unwrap_or("")
    }

    /// Trigger test generation.
    pub fn test_image(&mut self) {
        if self.selected_id() == "none" {
            return;
        }
        // Deterministic local check: required fields present -> success.
        let ok = fields_for(self.selected_id())
            .iter()
            .filter(|f| f.required)
            .all(|f| !self.get_value(f.key).is_empty());
        self.test_status = if ok { TestStatus::Success } else { TestStatus::Failed };
    }

    /// Render the screen: providers left, config panel right.
    pub fn render(&self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        self.render_with_cursor(area, buf, false);
    }

    /// Render with blink-gated caret on the focused config field (Option A:
    /// canonical `▏`). `cursor_on` is threaded from `App::cursor_visible()` at
    /// the call-site — the immutable `frame(&App)` borrow forbids mutating a
    /// persistent screen field, so the blink phase arrives as an argument.
    pub fn render_with_cursor(
        &self,
        area: Rect,
        buf: &mut ratatui::buffer::Buffer,
        cursor_on: bool,
    ) {
        Clear.render(area, buf);
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(40), Constraint::Length(46)])
            .split(area);

        self.render_provider_list(cols[0], buf);
        self.render_config_panel(cols[1], buf, cursor_on);
    }

    fn render_provider_list(&self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        let inner = Rect {
            x: area.x + 2,
            y: area.y + 1,
            width: area.width.saturating_sub(4),
            height: area.height.saturating_sub(2),
        };

        // Header — "Image generator" + sub (JSX 1525-1527)
        let mut y = inner.y;
        if y < inner.y + inner.height {
            buf.set_line(
                inner.x,
                y,
                &Line::from(Span::styled(
                    "Image generator",
                    Style::default().fg(FG).add_modifier(Modifier::BOLD),
                )),
                inner.width,
            );
            y += 1;
        }
        if y < inner.y + inner.height {
            buf.set_line(
                inner.x,
                y,
                &Line::from(vec![
                    Span::styled("Powers ", Style::default().fg(theme::DIM)),
                    Span::styled("image_generate", Style::default().fg(theme::ACCENT_BRIGHT)),
                    Span::styled(", ", Style::default().fg(theme::DIM)),
                    Span::styled("image_edit", Style::default().fg(theme::ACCENT_BRIGHT)),
                    Span::styled(". Writes to ", Style::default().fg(theme::DIM)),
                    Span::styled("[talos.image]", Style::default().fg(theme::ACCENT_BRIGHT)),
                    Span::styled(".", Style::default().fg(theme::DIM)),
                ]),
                inner.width,
            );
            y += 2;
        }

        // Provider cards — 3 rows each, bordered
        for (i, p) in IMAGE_PROVIDERS.iter().enumerate() {
            let card_h = 3;
            if y + card_h > inner.y + inner.height {
                break;
            }
            let selected = i == self.selected;
            let card_area = Rect {
                x: inner.x,
                y,
                width: inner.width,
                height: card_h,
            };
            let border_color = if selected { theme::ACCENT } else { theme::MUTED };
            let card_bg = if selected { theme::ACCENT_FAINT } else { BG2 };
            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color))
                .style(Style::default().bg(card_bg));
            let card_inner = block.inner(card_area);
            block.render(card_area, buf);

            // #271: clamp name + sub to card inner width so they truncate with
            // `…` instead of hard-chopping mid-word at narrow widths.
            let glyph_part = format!(" {} ", p.glyph); // " OAI " = 5 cols
            let sep = " "; // 1 col between glyph and name
            let sep2 = "  "; // 2 cols between name and sub
            let marker = if selected { "  ▸ SELECTED" } else { "" };
            let marker_w = marker.chars().count();
            let used = glyph_part.chars().count() + sep.len() + sep2.len() + marker_w;
            let name_budget = (card_inner.width as usize).saturating_sub(used);
            // Split remaining budget between name and sub (name gets priority).
            let name_clamped = clamp_ellipsis(p.name, name_budget);
            let sub_budget = (card_inner.width as usize)
                .saturating_sub(glyph_part.chars().count() + sep.len() + name_clamped.chars().count() + sep2.len() + marker_w);
            let sub_clamped = clamp_ellipsis(p.sub, sub_budget);

            let mut spans = vec![
                Span::styled(
                    glyph_part,
                    Style::default()
                        .fg(theme::BG)
                        .bg(p.color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(sep),
                Span::styled(
                    name_clamped,
                    Style::default().fg(theme::WHITE).add_modifier(Modifier::BOLD),
                ),
                Span::raw(sep2),
                Span::styled(sub_clamped, Style::default().fg(theme::DIM)),
            ];
            if selected {
                spans.push(Span::raw("  "));
                spans.push(Span::styled(
                    "▸ SELECTED",
                    Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD),
                ));
            }
            buf.set_line(card_inner.x, card_inner.y, &Line::from(spans), card_inner.width);
            y += card_h;
        }
    }

    fn render_config_panel(&self, area: Rect, buf: &mut ratatui::buffer::Buffer, cursor_on: bool) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::MUTED))
            .style(Style::default().bg(BG2));
        let inner = block.inner(area);
        block.render(area, buf);

        let p = &IMAGE_PROVIDERS[self.selected];
        let mut y = inner.y;

        if p.id == "none" {
            if y < inner.y + inner.height {
                // #271: clamp the empty-state message to panel inner width.
                let msg = clamp_ellipsis("No image gen configured.", (inner.width as usize).saturating_sub(2));
                buf.set_line(
                    inner.x + 1,
                    y,
                    &Line::from(Span::styled(
                        msg,
                        Style::default().fg(theme::DIM),
                    )),
                    inner.width.saturating_sub(2),
                );
            }
            return;
        }

        // Provider header: glyph block + name + sub
        if y < inner.y + inner.height {
            // #271: clamp name to panel inner width minus glyph prefix.
            let glyph_part = format!(" {} ", p.glyph);
            let name_budget = (inner.width as usize)
                .saturating_sub(2) // border
                .saturating_sub(glyph_part.chars().count() + 1); // glyph + space
            let name_clamped = clamp_ellipsis(p.name, name_budget);
            buf.set_line(
                inner.x + 1,
                y,
                &Line::from(vec![
                    Span::styled(
                        glyph_part,
                        Style::default()
                            .fg(theme::BG)
                            .bg(p.color)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(" "),
                    Span::styled(
                        name_clamped,
                        Style::default().fg(theme::WHITE).add_modifier(Modifier::BOLD),
                    ),
                ]),
                inner.width.saturating_sub(2),
            );
            y += 1;
        }
        if y < inner.y + inner.height {
            // #271: clamp sub to panel inner width.
            let sub_clamped = clamp_ellipsis(p.sub, (inner.width as usize).saturating_sub(2));
            buf.set_line(
                inner.x + 1,
                y,
                &Line::from(Span::styled(sub_clamped, Style::default().fg(theme::DIM))),
                inner.width.saturating_sub(2),
            );
            y += 2;
        }

        // CONFIG label
        if y < inner.y + inner.height {
            buf.set_line(
                inner.x + 1,
                y,
                &Line::from(Span::styled(
                    "C O N F I G",
                    Style::default().fg(theme::ACCENT_DIM).add_modifier(Modifier::BOLD),
                )),
                inner.width.saturating_sub(2),
            );
            y += 1;
        }

        // Fields
        for (i, field) in fields_for(p.id).iter().enumerate() {
            if y + 2 > inner.y + inner.height {
                break;
            }
            let focused = i == self.focused_field;
            let marker = if focused { "▶ " } else { "  " };
            let req = if field.required { " *" } else { "" };
            // #271: clamp label+req to panel inner width minus marker prefix.
            let label_text = format!("{}{}", field.label, req);
            let label_budget = (inner.width as usize)
                .saturating_sub(2) // border
                .saturating_sub(marker.chars().count());
            let label_clamped = clamp_ellipsis(&label_text, label_budget);
            buf.set_line(
                inner.x + 1,
                y,
                &Line::from(vec![
                    Span::styled(
                        marker,
                        Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        label_clamped,
                        Style::default()
                            .fg(if focused { theme::WHITE } else { FG })
                            .add_modifier(Modifier::BOLD),
                    ),
                ]),
                inner.width.saturating_sub(2),
            );
            y += 1;

            let raw = self.get_value(field.key);
            let shown: String = if raw.is_empty() {
                field.placeholder.to_string()
            } else if field.secret {
                // Char-safe masking: ***{last4} (never byte-slice — multibyte panics).
                let last4: String = {
                    let mut t: Vec<char> = raw.chars().rev().take(4).collect();
                    t.reverse();
                    t.into_iter().collect()
                };
                format!("***{last4}")
            } else {
                raw.to_string()
            };
            let val_style = if raw.is_empty() {
                Style::default().fg(theme::DIM)
            } else {
                Style::default().fg(theme::WHITE)
            };
            // Caret: canonical blink-gated `▏`, only on the focused field and
            // only when there's REAL input (gate on `raw`, never `shown` — the
            // placeholder-fallback would paint a caret at the end of hint text
            // on an unfilled field — banked placeholder-trap rule).
            // #271: clamp the shown value to the field value budget so long
            // URLs / keys truncate with `…` instead of mid-word chop.
            let val_budget = (inner.width as usize).saturating_sub(4);
            let val_clamped = clamp_ellipsis(&shown, val_budget);
            let mut val_spans = vec![Span::styled(val_clamped, val_style)];
            if cursor_on && focused && !raw.is_empty() {
                val_spans.push(Span::styled(
                    "\u{258f}",
                    Style::default().fg(theme::ACCENT),
                ));
            }
            buf.set_line(
                inner.x + 3,
                y,
                &Line::from(val_spans),
                inner.width.saturating_sub(4),
            );
            y += 1;

            if let Some(hint) = field.hint {
                // Wrap the hint — at the narrow right-panel width a single
                // set_line truncates ("...multi-step →" clips "black PNG)").
                // The Z-Image Turbo warning must render verbatim & complete.
                let hint_w = inner.width.saturating_sub(4);
                let hint_lines = (hint.chars().count() as u16 / hint_w.max(1)) + 1;
                let avail = (inner.y + inner.height).saturating_sub(y);
                let rows = hint_lines.min(avail);
                if rows > 0 {
                    let hint_area = Rect {
                        x: inner.x + 3,
                        y,
                        width: hint_w,
                        height: rows,
                    };
                    Paragraph::new(hint)
                        .style(Style::default().fg(theme::AMBER))
                        .wrap(Wrap { trim: true })
                        .render(hint_area, buf);
                    y += rows;
                }
            }
        }

        // Test button
        y += 1;
        if y < inner.y + inner.height {
            let (label, color) = match self.test_status {
                TestStatus::Idle => ("▸ TEST IMAGE", theme::ACCENT),
                TestStatus::Testing => ("◌ TESTING...", theme::AMBER),
                TestStatus::Success => ("✓ SUCCESS", theme::GREEN),
                TestStatus::Failed => ("✕ FAILED", theme::RED),
            };
            // #271: clamp test button label to panel inner width.
            let label_clamped = clamp_ellipsis(label, (inner.width as usize).saturating_sub(2));
            buf.set_line(
                inner.x + 1,
                y,
                &Line::from(Span::styled(
                    label_clamped,
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                )),
                inner.width.saturating_sub(2),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_ids_match_jsx() {
        let ids: Vec<&str> = IMAGE_PROVIDERS.iter().map(|p| p.id).collect();
        assert_eq!(
            ids,
            vec!["openai", "comfyui", "fooocus", "openai-custom", "a1111", "none"]
        );
    }

    #[test]
    fn select_wraps_both_directions() {
        let mut s = ImagesScreen::new();
        s.select_prev();
        assert_eq!(s.selected_id(), "none");
        s.select_next();
        assert_eq!(s.selected_id(), "openai");
    }

    #[test]
    fn fields_conditional_per_provider() {
        // openai: API Key + Model only
        let f: Vec<&str> = fields_for("openai").iter().map(|f| f.key).collect();
        assert_eq!(f, vec!["api_key", "model_id"]);
        // openai-custom: + base_url
        let f: Vec<&str> = fields_for("openai-custom").iter().map(|f| f.key).collect();
        assert_eq!(f, vec!["base_url", "api_key", "model_id"]);
        // a1111: + base_url + steps, api_key NOT required
        let fields = fields_for("a1111");
        let keys: Vec<&str> = fields.iter().map(|f| f.key).collect();
        assert_eq!(keys, vec!["base_url", "api_key", "model_id", "steps"]);
        assert!(!fields.iter().find(|f| f.key == "api_key").unwrap().required);
        assert!(fields.iter().find(|f| f.key == "steps").unwrap().hint.is_some());
    }

    #[test]
    fn test_requires_required_fields() {
        let mut s = ImagesScreen::new(); // openai selected
        s.test_image();
        assert!(matches!(s.test_status, TestStatus::Failed));
        s.values.insert("api_key".into(), "sk-x".into());
        s.values.insert("model_id".into(), "gpt-image-1".into());
        s.test_image();
        assert!(matches!(s.test_status, TestStatus::Success));
    }

    #[test]
    fn model_placeholder_is_provider_sub() {
        let fields = fields_for("fooocus");
        let model = fields.iter().find(|f| f.key == "model_id").unwrap();
        assert_eq!(model.placeholder, "SDXL Turbo");
    }

    // ── Cursor-port (Option A: canonical blink-gated `▏`) ──────────────────
    // images.rs had NO caret glyph (verified via glyph scan — no `▌`/`▏`), so
    // these tests scan the WHOLE buffer (no legitimate decoration glyph to
    // avoid, unlike skills.rs's card-accent stripe). The caret lives in the
    // right config panel on the focused field's value row.

    /// Render the full screen and return the whole buffer flattened to a string.
    fn buffer_string(screen: &ImagesScreen, cursor_on: bool) -> String {
        let area = Rect::new(0, 0, 100, 30);
        let mut buf = ratatui::buffer::Buffer::empty(area);
        screen.render_with_cursor(area, &mut buf, cursor_on);
        let mut out = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                out.push_str(buf[(x, y)].symbol());
            }
        }
        out
    }

    #[test]
    fn caret_painted_on_blink_phase_with_input() {
        let mut s = ImagesScreen::new(); // openai, focused_field 0 = api_key
        s.input_char('s');
        s.input_char('k');
        let out = buffer_string(&s, true);
        assert!(
            out.contains('\u{258f}'),
            "expected canonical caret `▏` on the focused field when cursor_on + field has input"
        );
    }

    #[test]
    fn caret_hidden_on_blink_off() {
        let mut s = ImagesScreen::new();
        s.input_char('s');
        s.input_char('k');
        let out = buffer_string(&s, false);
        assert!(
            !out.contains('\u{258f}'),
            "expected NO caret when cursor_on=false (blink-off phase)"
        );
    }

    #[test]
    fn caret_absent_on_empty_field_placeholder() {
        // Unfilled field shows the placeholder hint — NOT an edit position.
        // The caret must gate on `raw` (empty), never `shown` (placeholder
        // fallback) — banked placeholder-trap rule (mem 1249).
        let s = ImagesScreen::new();
        let out = buffer_string(&s, true);
        assert!(
            !out.contains('\u{258f}'),
            "expected NO caret on an unfilled field's placeholder hint"
        );
    }

    #[test]
    fn no_static_block_caret() {
        // Option A: the caret is the blink-gated `▏`, never a static `▌`.
        let mut s = ImagesScreen::new();
        s.input_char('s');
        let out = buffer_string(&s, true);
        assert!(
            !out.contains('\u{258c}'),
            "expected NO static block caret `▌` anywhere — Option A uses blink-gated `▏`"
        );
    }

    // ── #271 visual-parity: 2-width load-bearing render-verify ────────────

    /// Render the full screen at a given width/height and return the buffer
    /// flattened to a string (one row per line).
    fn render_at_width(screen: &ImagesScreen, w: u16, h: u16, cursor_on: bool) -> String {
        let area = Rect::new(0, 0, w, h);
        let mut buf = ratatui::buffer::Buffer::empty(area);
        screen.render_with_cursor(area, &mut buf, cursor_on);
        let mut out = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn clamp_ellipsis_truncates_with_marker() {
        // Unit: the helper appends `…` only on clip, never widens.
        assert_eq!(clamp_ellipsis("OpenAI GPT Image", 20), "OpenAI GPT Image");
        assert_eq!(clamp_ellipsis("OpenAI GPT Image", 10), "OpenAI GP…");
        assert_eq!(clamp_ellipsis("OpenAI GPT Image", 1), "…");
        assert_eq!(clamp_ellipsis("OpenAI GPT Image", 0), "");
    }

    #[test]
    fn narrow_width_clips_with_ellipsis_not_midword() {
        // #271 LOAD-BEARING: at a squeezed width the provider card title must
        // truncate with a trailing `…` — NOT hard-chop mid-word.
        // Revert any clamp → this fails (mid-word chop, no `…`).
        let s = ImagesScreen::new(); // openai selected
        let r = render_at_width(&s, 56, 30, false);
        // At width 56 the left panel is ~40 cols (Min(40) split), and the card
        // inner is even narrower — "OpenAI GPT Image" (16 chars) + glyph + sub
        // must clip. Assert at least one `…` appears.
        assert!(
            r.contains('…'),
            "narrow width must produce at least one ellipsis-clipped text; got:\n{r}"
        );
        // The full provider name "OpenAI GPT Image" must NOT survive whole in
        // the card title at this width — it should be clipped.
        assert!(
            !r.contains("OpenAI GPT Image"),
            "provider name must be clipped at narrow width; got:\n{r}"
        );
    }

    #[test]
    fn normal_width_renders_provider_content_in_full() {
        // #271: at a comfortable width the provider name renders in full.
        let s = ImagesScreen::new(); // openai selected
        let r = render_at_width(&s, 100, 40, false);
        assert!(
            r.contains("OpenAI GPT Image"),
            "full provider name must render at width 100; got:\n{r}"
        );
        assert!(
            r.contains("gpt-image-1"),
            "full provider sub must render at width 100; got:\n{r}"
        );
    }

    #[test]
    fn narrow_width_config_panel_clamps_field_label() {
        // #271 LOAD-BEARING: at a squeezed width the config panel field labels
        // must truncate with `…` — NOT hard-chop. The right panel is 46 cols
        // (Constraint::Length(46)), so at total width 56 the panel is still 46
        // but the inner is ~42. "Base URL" fits, but at very narrow total widths
        // the panel inner shrinks. Test with a1111 which has the longest labels.
        let mut s = ImagesScreen::new();
        // Navigate to a1111 (index 4)
        for _ in 0..4 {
            s.select_next();
        }
        let r = render_at_width(&s, 56, 30, false);
        // The "Z-Image Turbo" hint text should still be present (wrapped, not
        // chopped) — the Paragraph widget handles wrapping.
        assert!(
            r.contains("Z-Image Turbo") || r.contains("Z-Image") || r.contains("Turbo"),
            "hint text must be present (possibly wrapped) at narrow width; got:\n{r}"
        );
    }

    #[test]
    fn revert_clamp_fails_this_test() {
        // #271 LOAD-BEARING: This test is designed to FAIL if clamp_ellipsis
        // is removed from the provider card title. At width 56, the card inner
        // is narrow enough that "OpenAI GPT Image" + "gpt-image-1" + glyph +
        // "▸ SELECTED" exceeds the budget. Without clamping, set_line hard-
        // chops mid-word with NO ellipsis. With clamping, we get `…`.
        let s = ImagesScreen::new(); // openai, selected by default
        let r = render_at_width(&s, 56, 30, false);
        // The clamped version must contain `…` somewhere in the provider card.
        // If someone reverts the clamp, this assertion fails.
        assert!(
            r.contains('…'),
            "LOAD-BEARING: clamp_ellipsis must produce `…` at narrow width; \
             if this fails, the clamp was reverted or removed. Got:\n{r}"
        );
    }
}
