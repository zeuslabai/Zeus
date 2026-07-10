use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Widget};

use crate::theme;

/// Auth mode — matches JSX modes array (line 651).
pub struct AuthMode {
    pub id: &'static str,
    pub label: &'static str,
    pub desc: &'static str,
}

pub const AUTH_MODES: &[AuthMode] = &[
    AuthMode {
        id: "key",
        label: "API Key",
        desc: "Paste a provider-issued API key",
    },
    AuthMode {
        id: "token",
        label: "Setup Token",
        desc: "Paste an existing setup token",
    },
    AuthMode {
        id: "browser",
        label: "Browser OAuth",
        desc: "Authenticate via browser callback",
    },
];

/// OAuth step — matches JSX OAuth flow steps (line 753).
struct OAuthStep {
    state: &'static str, // "done", "active", "pending"
    text: &'static str,
}

const OAUTH_STEPS: &[OAuthStep] = &[
    OAuthStep {
        state: "done",
        text: "Opening browser to authentication URL...",
    },
    OAuthStep {
        state: "done",
        text: "Waiting for callback on http://127.0.0.1:8765/callback...",
    },
    OAuthStep {
        state: "active",
        text: "Received callback. Validating token...",
    },
    OAuthStep {
        state: "pending",
        text: "Storing token to credentials.json...",
    },
];

/// Auth screen — step 4 of onboarding.
/// Matches JSX AuthStep component (line 649).
pub struct AuthScreen {
    pub provider_name: &'static str,
    /// Canonical provider id (e.g. "sakana", "anthropic") from the provider
    /// registry. Used to resolve the real `env_key` for the config-write
    /// preview — the persist keys `[credentials]` by `Provider::env_key()`,
    /// NOT by the lowercased display name (#268).
    pub provider_id: &'static str,
    pub provider_color: ratatui::style::Color,
    pub key_fmt: &'static str,
    pub selected_mode: usize,
    pub api_key: String,
    pub test_status: Option<&'static str>, // None | Some("testing") | Some("success") | Some("error")
    /// Blink phase from `App.cursor_visible()` (106's tick seam). When true,
    /// the insertion-point cursor glyph is painted after the input text.
    pub cursor_on: bool,
}

impl AuthScreen {
    pub fn new(
        provider_name: &'static str,
        provider_color: ratatui::style::Color,
        key_fmt: &'static str,
    ) -> Self {
        Self {
            provider_name,
            // Default to the lowercased display name as the id; real
            // construction (app.rs) passes the canonical registry id.
            provider_id: "anthropic",
            provider_color,
            key_fmt,
            selected_mode: 0,
            api_key: String::new(),
            test_status: None,
            cursor_on: false,
        }
    }

    pub fn render(&self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        Clear.render(area, buf);
        // Outer padding: 16px top/bottom, 18px left/right (matches JSX padding: "16px 18px")
        let outer = Block::default();
        let inner = outer.inner(area);
        outer.render(area, buf);

        if inner.width < 20 || inner.height < 10 {
            return;
        }

        let x = inner.x;
        let y = inner.y;
        let w = inner.width;

        let mut cy = y;

        // ── Header: "Authenticate with {provider}" ──
        let header = Line::from(vec![
            Span::styled(
                "Authenticate with ",
                Style::default()
                    .fg(theme::TEXT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                self.provider_name,
                Style::default()
                    .fg(self.provider_color)
                    .add_modifier(Modifier::BOLD),
            ),
        ]);
        buf.set_line(x, cy, &header, w);
        cy += 1;

        let is_ollama = self.provider_id == "ollama";

        // Subtitle
        let subtitle = if is_ollama {
            Line::from(vec![
                Span::styled(
                    "Ollama runs locally; Zeus stores ",
                    Style::default().fg(theme::DIM),
                ),
                Span::styled("OLLAMA_HOST", Style::default().fg(theme::ACCENT_BRIGHT)),
                Span::styled(
                    " and polls /api/tags for models.",
                    Style::default().fg(theme::DIM),
                ),
            ])
        } else {
            Line::from(vec![
                Span::styled("Credentials persist to ", Style::default().fg(theme::DIM)),
                Span::styled(
                    "~/.zeus/config.toml [credentials]",
                    Style::default().fg(theme::ACCENT_BRIGHT),
                ),
                Span::styled(" with 0600 permissions.", Style::default().fg(theme::DIM)),
            ])
        };
        buf.set_line(x, cy, &subtitle, w);
        cy += 2;

        // ── Mode tabs ──
        let tab_y = cy;
        if is_ollama {
            let tab_w = w;
            buf.set_string(
                x,
                tab_y,
                "Ollama URL",
                Style::default()
                    .fg(theme::TEXT)
                    .add_modifier(Modifier::BOLD),
            );
            buf.set_string(
                x,
                tab_y + 1,
                "Enter the URL + port of the running Ollama server",
                Style::default().fg(theme::MUTED),
            );
            for dx in 0..tab_w {
                buf[(x + dx, tab_y + 2)]
                    .set_fg(theme::ACCENT)
                    .set_symbol("─");
            }
        } else {
            let tab_w = w / 3;
            for (i, mode) in AUTH_MODES.iter().enumerate() {
                let tx = x + (i as u16) * tab_w;
                let selected = i == self.selected_mode;

                // JSX: tabs carry NO background fill — selection is conveyed by the
                // bottom border accent + bold fg only (line 671-677). Paint the panel
                // bg uniformly so no highlight box leaks behind the selected tab.
                for dy in 0..3 {
                    for dx in 0..tab_w {
                        buf[(tx + dx, tab_y + dy)].set_bg(theme::BG);
                    }
                }

                // Tab label
                let label_style = if selected {
                    Style::default()
                        .fg(theme::TEXT)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme::DIM)
                };
                buf.set_string(tx, tab_y, mode.label, label_style);
                buf.set_string(tx, tab_y + 1, mode.desc, Style::default().fg(theme::MUTED));

                // JSX: the tab row has a 1px muted baseline under ALL tabs
                // (borderBottom: 1px solid C.muted, line 669); the selected tab
                // overlays a 2px accent rule (borderBottom: 2px solid C.accent).
                for dx in 0..tab_w {
                    let cell = &mut buf[(tx + dx, tab_y + 2)];
                    if selected {
                        cell.set_fg(theme::ACCENT).set_symbol("─");
                    } else {
                        // Muted baseline carries under the unselected tabs.
                        cell.set_fg(theme::MUTED).set_symbol("─");
                    }
                }
            }
        }
        cy = tab_y + 4;

        // ── Key/Token input field ──
        if self.selected_mode <= 1 {
            let field_label = if is_ollama {
                "Ollama URL/Port"
            } else if self.selected_mode == 0 {
                "API Key"
            } else {
                "Setup Token"
            };
            let field_hint = if is_ollama {
                "http://localhost:11434"
            } else if self.selected_mode == 0 {
                self.key_fmt
            } else {
                "paste token here"
            };

            // ── Section label (JSX 685/726: accentDim, letterSpacing 3, uppercase) ──
            // JSX renders `API KEY` / `SETUP TOKEN` letter-spaced above the field.
            let section = if is_ollama {
                "O L L A M A   H O S T"
            } else if self.selected_mode == 0 {
                "A P I   K E Y"
            } else {
                "S E T U P   T O K E N"
            };
            buf.set_string(
                x,
                cy,
                section,
                Style::default()
                    .fg(theme::ACCENT_DIM)
                    .add_modifier(Modifier::BOLD),
            );
            cy += 1;

            // ── Token-mode "↻ Detected" banner (JSX 728) ──
            if self.selected_mode == 1 {
                // bg2 box, amber left rail, detected-token message.
                let banner = Line::from(vec![
                    Span::styled(
                        "↻ Detected",
                        Style::default()
                            .fg(theme::AMBER)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(" setup token at ", Style::default().fg(theme::DIM)),
                    Span::styled(
                        "~/.zeus/setup-token",
                        Style::default().fg(theme::ACCENT_BRIGHT),
                    ),
                    Span::styled(" · pre-populating", Style::default().fg(theme::DIM)),
                ]);
                // Amber left rail.
                buf[(x, cy)].set_fg(theme::AMBER).set_symbol("▏");
                buf.set_line(x + 2, cy, &banner, w.saturating_sub(2));
                cy += 2;
            }

            // Input box.
            //
            // Focus-highlight (JSX `Field`, line 283-309): the active text field
            // is the canonical reusable focus pattern — focused → accent border +
            // accent label, unfocused → muted border + dim label. The Auth page
            // has a single text input (the API-key field) which is always the
            // focused field, so the box paints in the accent state. This becomes
            // the template every onboarding page reuses.
            // JSX renders the auth field as a full-width row inside the auth
            // pane. Keep the same shape in TUI: no 60-col island/gap at normal
            // widths, and clip content inside the border at narrow widths.
            let field_w = w;
            let field_block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme::ACCENT))
                .title(Span::styled(
                    format!(" {} ", field_label),
                    Style::default()
                        .fg(theme::ACCENT)
                        .add_modifier(Modifier::BOLD),
                ));
            let field_area = Rect::new(x, cy, field_w, 3);
            field_block.render(field_area, buf);

            // Input content
            let display_val = if self.api_key.is_empty() {
                field_hint.to_string()
            } else if is_ollama {
                self.api_key.trim_end_matches('/').to_string()
            } else {
                // Mask the key as `***{last4}` (JSX 784). Char-based via
                // rev().take(4) so a non-ASCII paste can't panic on a byte split.
                let mut last4: Vec<char> = self.api_key.chars().rev().take(4).collect();
                last4.reverse();
                let last4: String = last4.into_iter().collect();
                format!("***{}", last4)
            };
            let val_style = if self.api_key.is_empty() {
                Style::default().fg(theme::MUTED)
            } else {
                Style::default().fg(theme::TEXT)
            };
            let field_inner_w = field_w.saturating_sub(4);
            let _ = buf.set_stringn(
                x + 2,
                cy + 1,
                &display_val,
                field_inner_w as usize,
                val_style,
            );
            // Insertion-point cursor, gated on the shared blink phase (106's
            // `cursor_visible()` via `cursor_on`). This field is always focused
            // on the Auth page, so a focused EMPTY field must still show a caret
            // — at the input ORIGIN (col 0 after the label), NOT at the end of
            // the placeholder hint. merakizzz's live test: the old guard
            // (`!is_empty()`) suppressed the empty caret, and an amber glyph was
            // invisible against the bg anyway. Fix: paint at origin when empty,
            // after the text when non-empty, and in RED for live visibility.
            // Width is char-count: the masked value `***{last4}` is pure ASCII,
            // so cells == chars.
            if self.cursor_on {
                let cursor_col = if self.api_key.is_empty() {
                    // Input origin — col 0 of the field, where typing begins.
                    x + 2
                } else {
                    x + 2 + display_val.chars().count() as u16
                };
                // Clamp inside the field box (border at x + field_w - 1).
                if cursor_col < x + field_w - 1 {
                    buf.set_string(cursor_col, cy + 1, "▏", Style::default().fg(theme::RED));
                }
            }
            cy += 4;

            // Validation hint
            if !self.api_key.is_empty() {
                if is_ollama {
                    buf.set_string(
                        x,
                        cy,
                        "✓ URL will be polled at /api/tags",
                        Style::default().fg(theme::GREEN),
                    );
                } else {
                    let valid = self
                        .api_key
                        .starts_with(self.key_fmt.replace("...", "").as_str());
                    if valid {
                        buf.set_string(
                            x,
                            cy,
                            "✓ Key format matches expected prefix",
                            Style::default().fg(theme::GREEN),
                        );
                    } else {
                        buf.set_string(
                            x,
                            cy,
                            "✕ Key format doesn't match expected prefix",
                            Style::default().fg(theme::RED),
                        );
                    }
                }
                cy += 1;
            }

            // ── Test-connection button (JSX 697: rendered in ALL states) ──
            // JSX: bordered button box, label + border + bg keyed to testStatus.
            // None → `▸ TEST CONNECTION` accent; testing → accent; success → green; error → red.
            let (btn_label, btn_color) = match self.test_status {
                None => ("▸ TEST CONNECTION", theme::ACCENT),
                Some("testing") => ("▸ TESTING...", theme::ACCENT), // JSX: accent, not yellow
                Some("success") => ("✓ AUTH OK", theme::GREEN),
                Some("error") => ("✕ AUTH FAILED", theme::RED),
                _ => ("▸ TEST CONNECTION", theme::ACCENT),
            };
            let btn_w = (btn_label.chars().count() as u16) + 4;
            let btn_block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(btn_color));
            let btn_area = Rect::new(x, cy, btn_w.min(w), 3);
            btn_block.render(btn_area, buf);
            buf.set_string(
                x + 2,
                cy + 1,
                btn_label,
                Style::default().fg(btn_color).add_modifier(Modifier::BOLD),
            );
            cy += 4;

            // Test status detail line (error sub-message)
            if let Some(status) = self.test_status {
                match status {
                    "testing" => {}
                    "success" => {
                        // JSX 717-723: success shows the probe summary beside the button.
                        buf.set_string(
                            x,
                            cy,
                            "● /v1/models returned 200 · 184ms · 47 models available",
                            Style::default().fg(theme::GREEN),
                        );
                        cy += 1;
                    }
                    "error" => {
                        // Button already shows `✕ AUTH FAILED`; detail line carries the cause.
                        buf.set_string(
                            x,
                            cy,
                            "✕ 401 Unauthorized — check the API key",
                            Style::default().fg(theme::RED),
                        );
                        cy += 1;
                    }
                    _ => {}
                }
                cy += 1;
            }
        }

        // ── OAuth flow (browser mode) ──
        if self.selected_mode == 2 {
            buf.set_string(
                x,
                cy,
                "OAUTH FLOW",
                Style::default()
                    .fg(theme::ACCENT_DIM)
                    .add_modifier(Modifier::BOLD),
            );
            cy += 1;

            for step in OAUTH_STEPS {
                let (glyph, glyph_color, text_color) = match step.state {
                    "done" => ("✓", theme::DIM, theme::DIM),
                    "active" => ("▸", theme::TEXT, theme::TEXT),
                    "pending" => ("○", theme::MUTED, theme::MUTED),
                    _ => ("○", theme::MUTED, theme::MUTED),
                };

                buf.set_string(x, cy, glyph, Style::default().fg(glyph_color));
                buf.set_string(x + 2, cy, step.text, Style::default().fg(text_color));

                if step.state == "active" {
                    let dots_x = x + 2 + step.text.len() as u16 + 1;
                    buf.set_string(dots_x, cy, "...", Style::default().fg(theme::FIRE_ORANGE));
                }

                cy += 1;
            }
        }

        // ── Config preview ──
        cy += 1;
        // JSX renders this as a full-width write-preview card. The old 60-col
        // block clipped/gapped badly at both narrow and normal widths; keep the
        // box full-width and clip each interior line inside the borders.
        let preview_w = w;
        let preview_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::BORDER));
        let preview_area = Rect::new(x, cy, preview_w, 5);
        preview_block.render(preview_area, buf);

        let preview_x = x + 2;
        let preview_y = cy + 1;
        let preview_inner_w = preview_w.saturating_sub(4);
        let _ = buf.set_stringn(
            preview_x,
            preview_y,
            "WILL WRITE TO ~/.zeus/config.toml",
            preview_inner_w as usize,
            Style::default().fg(theme::DIM).add_modifier(Modifier::BOLD),
        );
        let _ = buf.set_stringn(
            preview_x,
            preview_y + 1,
            "[credentials]",
            preview_inner_w as usize,
            Style::default().fg(theme::DIM),
        );

        let key_display = if is_ollama {
            let host = if self.api_key.trim().is_empty() {
                "http://localhost:11434"
            } else {
                self.api_key.trim()
            };
            format!("\"{}\"", host.trim_end_matches('/'))
        } else if self.api_key.is_empty() {
            "\"\"".to_string()
        } else {
            // Same `***{last4}` masking as the input field — char-safe.
            let mut last4: Vec<char> = self.api_key.chars().rev().take(4).collect();
            last4.reverse();
            let last4: String = last4.into_iter().collect();
            format!("\"***{}\"", last4)
        };

        // Mirror the REAL persist (#268): the key lands as
        // `[credentials] <ENV_KEY> = "<key>"`, keyed by the canonical
        // `Provider::env_key()` — NOT `provider = "<display name>"`. Resolve
        // the env_key via the same `from_prefix` path app.rs uses so the
        // preview can never drift from the actual config write.
        let env_key = zeus_core::Provider::from_prefix(self.provider_id).env_key();
        // gemini-cli has an empty env_key (OAuth-only) — fall back to a
        // stable placeholder so the preview line is never blank.
        let env_key = if env_key.is_empty() {
            "API_KEY"
        } else {
            env_key
        };
        let preview_line = Line::from(vec![
            Span::styled(env_key, Style::default().fg(theme::TEXT)),
            Span::styled(" = ", Style::default().fg(theme::MUTED)),
            Span::styled(key_display, Style::default().fg(theme::ACCENT_BRIGHT)),
        ]);
        buf.set_line(preview_x, preview_y + 2, &preview_line, preview_inner_w);
    }
}

#[cfg(test)]
mod tests {
    use super::AuthScreen;
    use ratatui::{Terminal, backend::TestBackend};

    fn render_key(key: &str) {
        let mut term = Terminal::new(TestBackend::new(120, 40)).unwrap();
        term.draw(|f| {
            let mut s = AuthScreen::new("Anthropic", crate::theme::ACCENT, "sk-ant-...");
            s.api_key = key.to_string();
            s.render(f.area(), f.buffer_mut());
        })
        .unwrap();
    }

    #[test]
    fn long_ascii_key_does_not_panic() {
        render_key("sk-ant-api03-AbCdEfGhIjKlMnOpQrStUvWxYz0123456789AbCdEfGhIjKlMnOpQ");
    }

    #[test]
    fn multibyte_key_does_not_panic() {
        // A non-ASCII char straddling byte 8 panics a naive `&s[..8]` byte slice.
        render_key("sk-anté-墨汁-unicode-paste-é");
    }

    /// Scrape all buffer text into a single string for substring assertions.
    fn render_to_string(key: &str) -> String {
        let mut term = Terminal::new(TestBackend::new(120, 40)).unwrap();
        term.draw(|f| {
            let mut s = AuthScreen::new("Anthropic", crate::theme::ACCENT, "sk-ant-...");
            s.api_key = key.to_string();
            s.render(f.area(), f.buffer_mut());
        })
        .unwrap();
        let buf = term.backend().buffer().clone();
        let mut out = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn masking_uses_star3_last4_shape() {
        // JSX 784: masked key renders as `***{last4}` — NOT first-8-plus-stars.
        let rendered = render_to_string("sk-ant-api03-SECRETKEY-WXYZ");
        assert!(
            rendered.contains("***WXYZ"),
            "expected `***WXYZ` last-4 masking in buffer; got:\n{rendered}"
        );
        // The leading secret must NOT be visible.
        assert!(
            !rendered.contains("SECRETKEY"),
            "raw key body leaked into masked render"
        );
    }

    #[test]
    fn masking_short_key_last4() {
        // Keys shorter than 4 chars: rev().take(4) just yields the whole key.
        let rendered = render_to_string("ab");
        assert!(
            rendered.contains("***ab"),
            "short key should render `***ab`; got:\n{rendered}"
        );
    }

    /// Render with explicit blink phase + key, scrape buffer to string.
    fn render_with_cursor(key: &str, cursor_on: bool) -> String {
        let mut term = Terminal::new(TestBackend::new(120, 40)).unwrap();
        term.draw(|f| {
            let mut s = AuthScreen::new("Anthropic", crate::theme::ACCENT, "sk-ant-...");
            s.api_key = key.to_string();
            s.cursor_on = cursor_on;
            s.render(f.area(), f.buffer_mut());
        })
        .unwrap();
        let buf = term.backend().buffer().clone();
        let mut out = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn cursor_painted_on_blink_phase_with_input() {
        // cursor_on=true + non-empty key → the insertion-point glyph is painted.
        let rendered = render_with_cursor("sk-ant-test", true);
        assert!(
            rendered.contains('▏'),
            "expected cursor glyph `▏` when cursor_on && key present; got:\n{rendered}"
        );
    }

    #[test]
    fn cursor_hidden_on_off_phase() {
        // cursor_on=false → no cursor glyph (the blink "off" half-cycle).
        let rendered = render_with_cursor("sk-ant-test", false);
        assert!(
            !rendered.contains('▏'),
            "cursor glyph must not paint on the blink-off phase"
        );
    }

    #[test]
    fn cursor_painted_red_at_origin_on_empty_focused_field() {
        // merakizzz mandate #2: a focused EMPTY field MUST show a caret at the
        // input origin (col 0 after the label), in RED. The old guard
        // suppressed it; the live test exposed the gap. Assert the glyph paints
        // AND that it's RED for live-visibility (amber was invisible on bg).
        let cell = caret_cell("", true);
        let cell = cell.expect("empty focused field must paint a caret `▏` at input origin");
        assert_eq!(
            cell.fg,
            crate::theme::RED,
            "empty-field caret must be RED for live visibility (amber bit us before)"
        );
    }

    #[test]
    fn cursor_red_with_input_too() {
        // Non-empty caret is also RED (single colour for the focused insertion
        // point — the page template uses one caret colour).
        let cell = caret_cell("sk-ant-test", true).expect("caret must paint with input");
        assert_eq!(cell.fg, crate::theme::RED, "insertion caret must be RED");
    }

    fn render_auth_to_string(
        width: u16,
        key: &str,
        provider_id: &'static str,
        key_fmt: &'static str,
    ) -> String {
        let mut term = Terminal::new(TestBackend::new(width, 40)).unwrap();
        term.draw(|f| {
            let mut s = AuthScreen::new("Anthropic", crate::theme::ACCENT, key_fmt);
            s.provider_id = provider_id;
            s.api_key = key.to_string();
            s.render(f.area(), f.buffer_mut());
        })
        .unwrap();
        let buf = term.backend().buffer().clone();
        let mut out = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn auth_key_field_uses_full_width_at_normal_width() {
        let rendered = render_auth_to_string(100, "fish_123456789", "sakana", "fish_...");
        let field_row = rendered
            .lines()
            .find(|line| line.contains('┌') && line.contains("API Key"))
            .expect("buffer dump should include the API Key field border");
        assert!(
            field_row.ends_with('┐'),
            "API key field must span the normal-width auth pane; got:\n{rendered}"
        );
    }

    #[test]
    fn auth_write_preview_keeps_env_key_at_normal_width() {
        let rendered = render_auth_to_string(100, "fish_123456789", "sakana", "fish_...");
        assert!(
            rendered.contains("[credentials]"),
            "preview must include [credentials] header; got:\n{rendered}"
        );
        assert!(
            rendered.contains("SAKANA_API_KEY = \"***6789\""),
            "preview must preserve canonical Provider::env_key write shape; got:\n{rendered}"
        );
        assert!(
            !rendered.contains("provider =") && !rendered.contains("api_key ="),
            "preview regressed to stale provider/api_key shape; got:\n{rendered}"
        );
        let preview_row = rendered
            .lines()
            .find(|line| line.contains("WILL WRITE TO ~/.zeus/config.toml"))
            .expect("buffer dump should include the write-preview header");
        assert!(
            preview_row.ends_with('│'),
            "write-preview card must span the normal-width auth pane; got:\n{rendered}"
        );
    }

    #[test]
    fn ollama_url_renders_plaintext_and_skips_key_prefix_warning() {
        let rendered =
            render_auth_to_string(100, "http://localhost:11434", "ollama", "sk-ollama-...");
        assert!(
            rendered.contains("http://localhost:11434"),
            "Ollama host is a URL, not a secret; got:\n{rendered}"
        );
        assert!(
            rendered.contains("OLLAMA_HOST = \"http://localhost:11434\""),
            "Ollama preview must write the plaintext host; got:\n{rendered}"
        );
        assert!(
            rendered.contains("URL will be polled at /api/tags"),
            "Ollama validation should describe URL polling; got:\n{rendered}"
        );
        assert!(
            !rendered.contains("Key format"),
            "Ollama URL mode must not use API-key prefix validation; got:\n{rendered}"
        );
        assert!(
            !rendered.contains("***1434"),
            "Ollama URL must not be masked like a secret; got:\n{rendered}"
        );
    }

    #[test]
    fn auth_write_preview_keeps_env_key_at_narrow_width() {
        let rendered = render_auth_to_string(34, "fish_123456789", "sakana", "fish_...");
        assert!(
            rendered.contains("[credentials]"),
            "narrow preview must keep [credentials] visible; got:\n{rendered}"
        );
        assert!(
            rendered.contains("SAKANA_API_KEY = \"***6789\""),
            "narrow preview must not clip or rename the canonical env-key line; got:\n{rendered}"
        );
    }

    #[test]
    fn focus_highlight_accent_border() {
        // merakizzz mandate #1: the focused input box paints an ACCENT border
        // (the reusable focus pattern). At least one cell of the field box must
        // carry the accent fg. We scan the rendered buffer for any cell whose fg
        // is ACCENT among the box-drawing border glyphs.
        let mut term = Terminal::new(TestBackend::new(120, 40)).unwrap();
        term.draw(|f| {
            let s = AuthScreen::new("Anthropic", crate::theme::ACCENT, "sk-ant-...");
            s.render(f.area(), f.buffer_mut());
        })
        .unwrap();
        let buf = term.backend().buffer().clone();
        let mut found = false;
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                let c = &buf[(x, y)];
                let sym = c.symbol();
                let is_border = sym
                    .chars()
                    .any(|ch| matches!(ch, '─' | '│' | '┌' | '┐' | '└' | '┘'));
                if is_border && c.fg == crate::theme::ACCENT {
                    found = true;
                }
            }
        }
        assert!(
            found,
            "focused input box must paint an ACCENT border (reusable focus pattern)"
        );
    }

    /// Locate the caret cell `▏` in a render and return a clone (for fg checks).
    /// Returns None if no caret was painted.
    fn caret_cell(key: &str, cursor_on: bool) -> Option<ratatui::buffer::Cell> {
        let mut term = Terminal::new(TestBackend::new(120, 40)).unwrap();
        term.draw(|f| {
            let mut s = AuthScreen::new("Anthropic", crate::theme::ACCENT, "sk-ant-...");
            s.api_key = key.to_string();
            s.cursor_on = cursor_on;
            s.render(f.area(), f.buffer_mut());
        })
        .unwrap();
        let buf = term.backend().buffer().clone();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                if buf[(x, y)].symbol() == "▏" {
                    return Some(buf[(x, y)].clone());
                }
            }
        }
        None
    }
}
