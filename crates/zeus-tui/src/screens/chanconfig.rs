use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Block, Borders, Widget};

use crate::theme;

/// Per-channel credential field definition.
struct ChannelFieldDef {
    key: &'static str,
    label: &'static str,
    placeholder: &'static str,
    secret: bool,
    required: bool,
    #[allow(dead_code)]
    default: Option<&'static str>,
}

/// State-badge / info-line kind for a channel (mirrors JSX isQR / isAppleScript).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PairKind {
    /// Standard credential-based channel.
    None,
    /// QR pairing (signal, whatsapp): amber `QR PAIRING` badge + ⚠ info line.
    Qr,
    /// AppleScript bridge (imessage): cyan `APPLESCRIPT` badge + ● info line.
    AppleScript,
}

/// Channel definition with fields.
struct ChannelDef {
    id: &'static str,
    name: &'static str,
    glyph: &'static str,
    sdk: &'static str,
    color: fn() -> ratatui::style::Color,
    fields: &'static [ChannelFieldDef],
    pair: PairKind,
}

impl ChannelDef {
    /// AppleScript/QR channels are credential-less → no test button.
    fn has_test(&self) -> bool {
        !self.fields.is_empty()
    }
}

/// Every channel id in the onboarding registry, in declaration order.
///
/// Exposed so the persist layer's completeness guard (`app::persist_tests`) can
/// assert that EVERY registered channel is either persisted by
/// `collect_and_persist` or explicitly classified as deferred — adding a new
/// `ChannelDef` here forces a persist decision instead of a silent drop.
pub fn channel_registry_ids() -> Vec<&'static str> {
    channels().iter().map(|c| c.id).collect()
}

// ── Channel registry (matches JSX CHANNELS + fieldsByChannel 1007–1126 exactly) ──

fn channels() -> &'static [ChannelDef] {
    &[
        ChannelDef {
            id: "telegram",
            name: "Telegram",
            glyph: "TG",
            sdk: "grammers MTProto",
            color: || theme::BLUE,
            pair: PairKind::None,
            fields: &[
                ChannelFieldDef { key: "api_id", label: "API ID", placeholder: "12345678", secret: false, required: true, default: None },
                ChannelFieldDef { key: "api_hash", label: "API Hash", placeholder: "...", secret: true, required: true, default: None },
                ChannelFieldDef { key: "phone", label: "Phone", placeholder: "+1234567890", secret: false, required: true, default: None },
            ],
        },
        ChannelDef {
            id: "discord",
            name: "Discord",
            glyph: "DC",
            sdk: "Serenity gateway",
            color: || theme::PURPLE,
            pair: PairKind::None,
            fields: &[
                ChannelFieldDef { key: "token", label: "Bot Token", placeholder: "MTAxxxx...", secret: true, required: true, default: None },
                ChannelFieldDef { key: "server_id", label: "Server (Guild) ID", placeholder: "9876543210", secret: false, required: false, default: None },
                ChannelFieldDef { key: "channel_id", label: "Default Channel ID", placeholder: "1234567890", secret: false, required: false, default: None },
            ],
        },
        ChannelDef {
            id: "slack",
            name: "Slack",
            glyph: "SL",
            sdk: "Socket Mode + Web API",
            color: || theme::GREEN,
            pair: PairKind::None,
            fields: &[
                ChannelFieldDef { key: "bot_token", label: "Bot Token", placeholder: "xoxb-...", secret: true, required: true, default: None },
                ChannelFieldDef { key: "app_token", label: "App Token", placeholder: "xapp-...", secret: true, required: true, default: None },
            ],
        },
        ChannelDef {
            id: "email",
            name: "Email",
            glyph: "EM",
            sdk: "lettre SMTP + IMAP",
            color: || theme::AMBER,
            pair: PairKind::None,
            fields: &[
                ChannelFieldDef { key: "smtp_host", label: "SMTP Host", placeholder: "smtp.gmail.com", secret: false, required: true, default: None },
                ChannelFieldDef { key: "smtp_port", label: "SMTP Port", placeholder: "587", secret: false, required: true, default: Some("587") },
                ChannelFieldDef { key: "username", label: "Username", placeholder: "you@gmail.com", secret: false, required: true, default: None },
                ChannelFieldDef { key: "password", label: "App Password", placeholder: "...", secret: true, required: true, default: None },
            ],
        },
        ChannelDef {
            id: "imessage",
            name: "iMessage",
            glyph: "iM",
            sdk: "AppleScript bridge",
            color: || theme::CYAN,
            pair: PairKind::AppleScript,
            fields: &[],
        },
        ChannelDef {
            id: "whatsapp",
            name: "WhatsApp",
            glyph: "WA",
            sdk: "Cloud API",
            color: || theme::GREEN,
            pair: PairKind::Qr,
            fields: &[
                ChannelFieldDef { key: "phone_id", label: "Phone Number ID", placeholder: "...", secret: false, required: true, default: None },
                ChannelFieldDef { key: "access_token", label: "Access Token", placeholder: "...", secret: true, required: true, default: None },
            ],
        },
        ChannelDef {
            id: "signal",
            name: "Signal",
            glyph: "SG",
            sdk: "signal-cli JSON-RPC",
            color: || theme::BLUE,
            pair: PairKind::Qr,
            fields: &[],
        },
        ChannelDef {
            id: "matrix",
            name: "Matrix",
            glyph: "MX",
            sdk: "matrix-rust-sdk",
            color: || theme::FIRE_ORANGE,
            pair: PairKind::None,
            fields: &[
                ChannelFieldDef { key: "homeserver", label: "Homeserver URL", placeholder: "https://matrix.org", secret: false, required: true, default: None },
                ChannelFieldDef { key: "username", label: "Username", placeholder: "@user:matrix.org", secret: false, required: true, default: None },
                ChannelFieldDef { key: "password", label: "Password", placeholder: "...", secret: true, required: true, default: None },
            ],
        },
        ChannelDef {
            id: "irc",
            name: "IRC",
            glyph: "IR",
            sdk: "Tokio IRC client",
            color: || theme::YELLOW,
            pair: PairKind::None,
            fields: &[
                ChannelFieldDef { key: "server", label: "Server", placeholder: "irc.libera.chat", secret: false, required: true, default: None },
                ChannelFieldDef { key: "port", label: "Port", placeholder: "6697", secret: false, required: true, default: Some("6697") },
                ChannelFieldDef { key: "nick", label: "Nick", placeholder: "zeusbot", secret: false, required: true, default: None },
                ChannelFieldDef { key: "channels", label: "Channels", placeholder: "#zeus,#ops", secret: false, required: true, default: None },
                ChannelFieldDef { key: "password", label: "Server Password", placeholder: "...", secret: true, required: false, default: None },
            ],
        },
        ChannelDef {
            id: "x_twitter",
            name: "X / Twitter",
            glyph: "X",
            sdk: "v2 API + OAuth 1.0a",
            color: || theme::WHITE,
            pair: PairKind::None,
            fields: &[
                ChannelFieldDef { key: "api_key", label: "API Key", placeholder: "...", secret: true, required: true, default: None },
                ChannelFieldDef { key: "api_secret", label: "API Secret", placeholder: "...", secret: true, required: true, default: None },
                ChannelFieldDef { key: "access_token", label: "Access Token", placeholder: "...", secret: true, required: true, default: None },
                ChannelFieldDef { key: "access_secret", label: "Access Secret", placeholder: "...", secret: true, required: true, default: None },
            ],
        },
    ]
}

/// Test status for a channel's "send test" action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestStatus {
    Idle,
    Testing,
    Success,
    Failed,
}

/// ChanConfig onboarding screen (08/19) — 1:1 with the prototype ChanConfigStep
/// component (JSX 1007–1126).
///
/// Per-channel credential fields, conditional on which channels were toggled
/// in the Channels step. Each channel shows its fields + a "SEND TEST" button.
pub struct ChanConfigScreen {
    /// Which channels are toggled on (from the Channels step).
    pub toggled: Vec<String>,
    /// Config values keyed by "channel_id.field_key" (e.g. "discord.token").
    pub config_values: std::collections::HashMap<String, String>,
    /// Currently focused field key (e.g. "discord.token"), or a test button key ("test:discord").
    pub focused_field: String,
    /// Test status per channel.
    pub test_statuses: std::collections::HashMap<String, TestStatus>,
    /// Cursor position within the flat field list for ↑↓ navigation.
    pub field_cursor: usize,
    /// Per-channel bot-message policy ("on"/"mentions"/"off"), keyed by channel
    /// id. Only bot-capable channels (`is_bot_capable`) carry an entry; the
    /// value is read by `bot_policy` (defaulting to "mentions") and persisted by
    /// `app::collect_and_persist` as `allow_bots = "<choice>"`.
    pub bot_policies: std::collections::HashMap<String, String>,
}

/// The three valid `allow_bots` policy values, in cycle order. Default is the
/// first entry (`mentions`).
pub const BOT_POLICIES: [&str; 3] = ["mentions", "on", "off"];

/// Channels that gate bot-authored messages via `allow_bots` (discord,
/// telegram, irc, matrix, slack). Mirrors the channels whose `*ChannelConfig`
/// struct carries an `allow_bots: Option<String>` field AND that onboarding
/// persists. Non-bot channels (email, imessage, …) get no selector.
pub fn is_bot_capable(channel_id: &str) -> bool {
    matches!(channel_id, "discord" | "telegram" | "irc" | "matrix" | "slack")
}

impl Default for ChanConfigScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl ChanConfigScreen {
    pub fn new() -> Self {
        Self {
            toggled: Vec::new(),
            config_values: std::collections::HashMap::new(),
            focused_field: String::new(),
            test_statuses: std::collections::HashMap::new(),
            field_cursor: 0,
            bot_policies: std::collections::HashMap::new(),
        }
    }

    /// Current bot-message policy for a channel, defaulting to `mentions` when
    /// the user has not cycled the selector. Verbatim one of `on`/`mentions`/`off`.
    pub fn bot_policy(&self, channel_id: &str) -> &str {
        self.bot_policies
            .get(channel_id)
            .map(String::as_str)
            .unwrap_or(BOT_POLICIES[0])
    }

    /// Advance the bot-message policy for a channel to the next value in the
    /// `mentions → on → off → mentions` cycle. No-op for non-bot channels.
    pub fn cycle_bot_policy(&mut self, channel_id: &str) {
        if !is_bot_capable(channel_id) {
            return;
        }
        let cur = self.bot_policy(channel_id);
        let idx = BOT_POLICIES.iter().position(|p| *p == cur).unwrap_or(0);
        let next = BOT_POLICIES[(idx + 1) % BOT_POLICIES.len()];
        self.bot_policies
            .insert(channel_id.to_string(), next.to_string());
    }

    /// Build the flat list of focusable items for the currently toggled channels.
    /// Each item is a string key: "channel_id.field_key" or "test:channel_id".
    fn focusable_keys(&self) -> Vec<String> {
        let mut keys = Vec::new();
        for ch in channels() {
            if !self.toggled.contains(&ch.id.to_string()) {
                continue;
            }
            for f in ch.fields {
                keys.push(format!("{}.{}", ch.id, f.key));
            }
            // Bot-capable channels expose an allow_bots policy selector,
            // focusable in the same flat nav list as the credential fields.
            if is_bot_capable(ch.id) {
                keys.push(format!("allowbots:{}", ch.id));
            }
            if ch.has_test() {
                keys.push(format!("test:{}", ch.id));
            }
        }
        keys
    }

    /// Move focus up.
    pub fn focus_prev(&mut self) {
        let keys = self.focusable_keys();
        if keys.is_empty() {
            return;
        }
        self.field_cursor = self.field_cursor.saturating_add(keys.len() - 1) % keys.len();
        self.focused_field = keys[self.field_cursor].clone();
    }

    /// Move focus down.
    pub fn focus_next(&mut self) {
        let keys = self.focusable_keys();
        if keys.is_empty() {
            return;
        }
        self.field_cursor = (self.field_cursor + 1) % keys.len();
        self.focused_field = keys[self.field_cursor].clone();
    }

    /// Trigger the test action for the channel of the focused item.
    pub fn trigger_test(&mut self) {
        let ch_id = if let Some(rest) = self.focused_field.strip_prefix("test:") {
            rest.to_string()
        } else if let Some((ch, _)) = self.focused_field.split_once('.') {
            ch.to_string()
        } else {
            return;
        };
        // Only channels with a test button (have fields) can be tested.
        let Some(def) = channels().iter().find(|c| c.id == ch_id && c.has_test()) else {
            return;
        };
        // Real validation: every `required` field must be non-empty before the
        // test can pass. Previously this blanket-inserted Success with zero
        // checks — so a test "passed" with no credentials entered (merakizzz's
        // "✓ delivered to Discord" with empty fields). Now: missing a required
        // field → Failed; all required present → Success.
        let all_required_present = def.fields.iter().filter(|f| f.required).all(|f| {
            let full_key = format!("{}.{}", ch_id, f.key);
            self.config_values
                .get(&full_key)
                .is_some_and(|v| !v.trim().is_empty())
        });
        let status = if all_required_present {
            TestStatus::Success
        } else {
            TestStatus::Failed
        };
        self.test_statuses.insert(ch_id, status);
    }

    /// Handle a printable character input for the currently focused field.
    pub fn input_char(&mut self, c: char) {
        if self.focused_field.starts_with("test:") || self.focused_field.is_empty() {
            return; // not an input field
        }
        if let Some(val) = self.config_values.get_mut(&self.focused_field) {
            val.push(c);
        } else {
            self.config_values.insert(self.focused_field.clone(), c.to_string());
        }
    }

    /// Handle backspace on the currently focused field.
    pub fn input_backspace(&mut self) {
        if self.focused_field.starts_with("test:") {
            return;
        }
        if let Some(val) = self.config_values.get_mut(&self.focused_field) {
            val.pop();
        }
    }

    /// Get the display value for a field (masked `***{last4}` if secret).
    ///
    /// Char-safe: uses `chars().rev().take(4)` — NEVER byte-slices (multibyte safe),
    /// consistent with the Auth screen's masking.
    fn display_value(&self, full_key: &str, secret: bool) -> String {
        let raw = self.config_values.get(full_key).map(|s| s.as_str()).unwrap_or("");
        if secret && !raw.is_empty() {
            let last4: String = raw.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
            format!("***{}", last4)
        } else {
            raw.to_string()
        }
    }
}

/// Truncate `text` to at most `max_w` display columns, appending an ellipsis
/// when clipped so the cut is honest rather than a raw mid-word chop. A width
/// budget of 0 yields the empty string. Used for the subtitle, field values and
/// test-result messages so a long string can never overflow `inner.width` into
/// the card chrome (the same unclamped-`set_string` class fixed in model.rs).
fn clamp_ellipsis(text: &str, max_w: usize) -> String {
    let len = text.chars().count();
    if len <= max_w {
        return text.to_string();
    }
    if max_w == 0 {
        return String::new();
    }
    if max_w == 1 {
        return "…".to_string();
    }
    let mut out: String = text.chars().take(max_w - 1).collect();
    out.push('…');
    out
}

impl Widget for &ChanConfigScreen {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Plain Widget path: no blink phase available → caret off.
        self.render_with_cursor(area, buf, false);
    }
}

impl ChanConfigScreen {
    /// Render with an explicit blink phase (`cursor_on` from `App.cursor_visible()`).
    /// The focused field paints the canonical `▏` caret only when `cursor_on`.
    pub fn render_with_cursor(&self, area: Rect, buf: &mut Buffer, cursor_on: bool) {
        Clear.render(area, buf);
        // Opaque background
        let bg = Block::default().style(Style::default().bg(theme::BG));
        bg.render(area, buf);

        if area.width < 4 || area.height < 4 {
            return;
        }

        let inner = Rect {
            x: area.x + 2,
            y: area.y + 1,
            width: area.width.saturating_sub(4),
            height: area.height.saturating_sub(2),
        };

        let mut cy = inner.y;
        let bottom = inner.y + inner.height;

        // ── Header: "Configure {N} channel(s)" ──
        let n = channels().iter().filter(|c| self.toggled.contains(&c.id.to_string())).count();
        if cy < bottom {
            let plural = if n != 1 { "s" } else { "" };
            buf.set_string(
                inner.x,
                cy,
                format!("Configure {} channel{}", n, plural),
                Style::default().fg(theme::TEXT).add_modifier(ratatui::style::Modifier::BOLD),
            );
            cy += 1;
        }
        if cy < bottom {
            buf.set_string(
                inner.x,
                cy,
                clamp_ellipsis(
                    "All channels visible — fill in any order. Test buttons send a \"Zeus connected ✅\" message to verify.",
                    inner.width as usize,
                ),
                Style::default().fg(theme::DIM),
            );
            cy += 2;
        }

        // ── Empty state ──
        if n == 0 {
            if cy < bottom {
                let msg = "No channels selected. Zeus will run console-only.";
                let box_w = (msg.chars().count() as u16 + 4).min(inner.width);
                let box_rect = Rect { x: inner.x, y: cy, width: box_w, height: 3 };
                let dashed = Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme::MUTED));
                dashed.render(box_rect, buf);
                buf.set_string(box_rect.x + 2, box_rect.y + 1, msg, Style::default().fg(theme::MUTED));
            }
            return;
        }

        // ── One config card per SELECTED channel (vertical stack) ──
        for ch in channels() {
            if !self.toggled.contains(&ch.id.to_string()) {
                continue;
            }
            if cy >= bottom {
                break;
            }

            let ch_color = (ch.color)();
            let status = self.test_statuses.get(ch.id).copied().unwrap_or(TestStatus::Idle);

            // ── Header row: glyph badge + name + sdk italic + spacer + state badge ──
            let mut header_spans = vec![
                Span::styled(
                    format!(" {} ", ch.glyph),
                    Style::default().fg(theme::BG).bg(ch_color).add_modifier(ratatui::style::Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(ch.name, Style::default().fg(theme::WHITE).add_modifier(ratatui::style::Modifier::BOLD)),
                Span::raw("  "),
                Span::styled(ch.sdk, Style::default().fg(theme::DIM).add_modifier(ratatui::style::Modifier::ITALIC)),
            ];
            // State badge (right side). Precedence: tested > pair-kind.
            let badge: Option<(&str, ratatui::style::Color)> = if status == TestStatus::Success {
                Some(("✓ TESTED", theme::GREEN))
            } else {
                match ch.pair {
                    PairKind::Qr => Some(("QR PAIRING", theme::AMBER)),
                    PairKind::AppleScript => Some(("APPLESCRIPT", theme::CYAN)),
                    PairKind::None => None,
                }
            };
            // Compute used width to right-align the badge.
            let used: usize = header_spans.iter().map(|s| s.content.chars().count()).sum();
            if let Some((label, color)) = badge {
                let badge_w = label.chars().count();
                let pad = (inner.width as usize).saturating_sub(used + badge_w);
                if pad > 0 {
                    header_spans.push(Span::raw(" ".repeat(pad)));
                }
                header_spans.push(Span::styled(label, Style::default().fg(color).add_modifier(ratatui::style::Modifier::BOLD)));
            }
            buf.set_line(inner.x, cy, &Line::from(header_spans), inner.width);
            cy += 1;

            // ── Credential-less info lines (imessage / signal) ──
            if ch.fields.is_empty() {
                if cy < bottom {
                    let (mark, mark_color, text) = match ch.pair {
                        PairKind::AppleScript => (
                            "●",
                            theme::CYAN,
                            "Uses native macOS bridge. No credentials needed. Will request Messages permission on first use.",
                        ),
                        PairKind::Qr => (
                            "⚠",
                            theme::AMBER,
                            "Requires phone-side QR scan. Pairing screen will display after this step.",
                        ),
                        PairKind::None => ("", theme::DIM, ""),
                    };
                    // Mark prefix is "{mark} " (mark + space). Budget the info text
                    // to the remaining width so it ellipsis-truncates honestly rather
                    // than mid-word chopping at the `set_line` clamp edge.
                    let text_budget =
                        (inner.width as usize).saturating_sub(2).saturating_sub(2);
                    let text = clamp_ellipsis(text, text_budget);
                    let info = Line::from(vec![
                        Span::styled(format!("{} ", mark), Style::default().fg(mark_color)),
                        Span::styled(text, Style::default().fg(theme::TEXT)),
                    ]);
                    buf.set_line(inner.x + 2, cy, &info, inner.width.saturating_sub(2));
                    cy += 1;
                }
                cy += 1; // gap between cards
                continue;
            }

            // ── Body fields ──
            for f in ch.fields {
                if cy >= bottom {
                    break;
                }
                let full_key = format!("{}.{}", ch.id, f.key);
                let is_focused = self.focused_field == full_key;

                // Label (left gutter) + required marker
                let label_style = if is_focused {
                    Style::default().fg(theme::FIRE_ORANGE)
                } else {
                    Style::default().fg(theme::DIM)
                };
                let required_marker = if f.required { " *" } else { "" };
                let label_text = format!("{}{}", f.label, required_marker);
                buf.set_string(inner.x + 4, cy, &label_text, label_style);

                // Value / placeholder, offset into a fixed gutter for alignment.
                let value = self.display_value(&full_key, f.secret);
                let val_x = inner.x + 4 + 24; // ≈label gutter
                let (value_text, value_style) = if value.is_empty() {
                    // Focused-but-empty: blink-gated caret on the placeholder
                    // line (auth.rs canonical `▏`, gated on `cursor_on`).
                    if is_focused && cursor_on {
                        (format!("{}\u{258f}", f.placeholder), Style::default().fg(theme::MUTED))
                    } else {
                        (f.placeholder.to_string(), Style::default().fg(theme::MUTED))
                    }
                } else if is_focused && cursor_on {
                    // Blink-gated cursor glyph trailing the focused value.
                    (format!("{value}\u{258f}"), Style::default().fg(theme::TEXT))
                } else if is_focused {
                    (value, Style::default().fg(theme::TEXT))
                } else {
                    (value, Style::default().fg(theme::TEXT))
                };
                if val_x < inner.x + inner.width {
                    // Budget the value zone so a long secret/URL can never overflow
                    // `inner.width` into the card chrome (unclamped-`set_string` class).
                    let val_budget = (inner.x + inner.width).saturating_sub(val_x) as usize;
                    let clamped = clamp_ellipsis(&value_text, val_budget);
                    buf.set_string(val_x, cy, &clamped, value_style);
                }
                cy += 1;
            }

            // ── Bot-message policy selector (allow_bots) ──
            if is_bot_capable(ch.id) && cy < bottom {
                let ab_key = format!("allowbots:{}", ch.id);
                let is_ab_focused = self.focused_field == ab_key;
                let policy = self.bot_policy(ch.id);

                let label_style = if is_ab_focused {
                    Style::default().fg(theme::FIRE_ORANGE)
                } else {
                    Style::default().fg(theme::DIM)
                };
                buf.set_string(inner.x + 4, cy, "Bot messages", label_style);

                // Render the three choices inline; highlight the active one.
                // Focused row shows a hint that ⏎ cycles the value.
                let mut spans: Vec<Span> = Vec::new();
                for (i, p) in BOT_POLICIES.iter().enumerate() {
                    if i > 0 {
                        spans.push(Span::styled(" / ", Style::default().fg(theme::MUTED)));
                    }
                    let style = if *p == policy {
                        Style::default()
                            .fg(theme::TEXT)
                            .add_modifier(ratatui::style::Modifier::BOLD)
                    } else {
                        Style::default().fg(theme::MUTED)
                    };
                    spans.push(Span::styled(*p, style));
                }
                if is_ab_focused {
                    spans.push(Span::styled(
                        "  (⏎ cycle)",
                        Style::default().fg(theme::FIRE_ORANGE),
                    ));
                }
                let val_x = inner.x + 4 + 24;
                if val_x < inner.x + inner.width {
                    let line = Line::from(spans);
                    buf.set_line(val_x, cy, &line, (inner.x + inner.width).saturating_sub(val_x));
                }
                cy += 1;
            }

            // ── SEND TEST button ──
            if ch.has_test() && cy + 2 < bottom {
                let test_key = format!("test:{}", ch.id);
                let is_test_focused = self.focused_field == test_key;

                let (btn_label, btn_color) = match status {
                    TestStatus::Testing => ("▸ SENDING...", theme::FIRE_ORANGE),
                    TestStatus::Success => ("✓ DELIVERED", theme::GREEN),
                    TestStatus::Failed => ("✗ FAILED", theme::RED),
                    TestStatus::Idle => {
                        if is_test_focused {
                            ("▸ SEND TEST", theme::FIRE_ORANGE)
                        } else {
                            ("▸ SEND TEST", theme::DIM)
                        }
                    }
                };
                let btn_border = if is_test_focused || status == TestStatus::Testing || status == TestStatus::Success {
                    btn_color
                } else {
                    theme::MUTED
                };

                let btn_w = btn_label.chars().count() as u16 + 4;
                let btn_rect = Rect { x: inner.x + 4, y: cy, width: btn_w.min(inner.width), height: 3 };
                let btn_block = Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(btn_border));
                btn_block.render(btn_rect, buf);
                buf.set_string(
                    btn_rect.x + 2,
                    btn_rect.y + 1,
                    btn_label,
                    Style::default().fg(btn_color).add_modifier(ratatui::style::Modifier::BOLD),
                );

                // Result line — Success confirms delivery; Failed tells the
                // user a required credential is missing (so the test is no
                // longer a silent rubber-stamp).
                let result_msg = match status {
                    TestStatus::Success => {
                        Some((format!("● Test message delivered to {}", ch.name), theme::GREEN))
                    }
                    TestStatus::Failed => {
                        Some(("✗ Missing required credentials".to_string(), theme::RED))
                    }
                    _ => None,
                };
                if let Some((msg, color)) = result_msg {
                    let mx = btn_rect.x + btn_rect.width + 2;
                    if mx < inner.x + inner.width {
                        let msg_budget = (inner.x + inner.width).saturating_sub(mx) as usize;
                        let clamped = clamp_ellipsis(&msg, msg_budget);
                        buf.set_string(mx, btn_rect.y + 1, &clamped, Style::default().fg(color));
                    }
                }
                cy += 3;
            }

            cy += 1; // gap between cards
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn discord_toggled() -> ChanConfigScreen {
        let mut s = ChanConfigScreen::new();
        s.toggled = vec!["discord".to_string()];
        s
    }

    #[test]
    fn discord_has_server_id_field() {
        // merakizzz: Server/Guild ID couldn't be entered — because the field
        // didn't exist. It must now be present on the Discord def.
        let discord = channels().iter().find(|c| c.id == "discord").unwrap();
        assert!(
            discord.fields.iter().any(|f| f.key == "server_id"),
            "Discord must expose a server_id field"
        );
    }

    #[test]
    fn server_id_is_focusable() {
        let s = discord_toggled();
        let keys = s.focusable_keys();
        assert!(
            keys.contains(&"discord.server_id".to_string()),
            "server_id must be reachable via field navigation"
        );
    }

    #[test]
    fn trigger_test_fails_with_no_credentials() {
        // The core bug: test "passed" with zero credentials entered.
        let mut s = discord_toggled();
        s.focused_field = "test:discord".to_string();
        s.trigger_test();
        assert_eq!(
            s.test_statuses.get("discord").copied(),
            Some(TestStatus::Failed),
            "test must FAIL when required token is empty (was a silent rubber-stamp)"
        );
    }

    #[test]
    fn trigger_test_passes_with_required_present() {
        let mut s = discord_toggled();
        // token is the only required Discord field; server_id/channel_id optional.
        s.config_values
            .insert("discord.token".to_string(), "MTAxxxx.real.token".to_string());
        s.focused_field = "test:discord".to_string();
        s.trigger_test();
        assert_eq!(
            s.test_statuses.get("discord").copied(),
            Some(TestStatus::Success),
            "test must PASS once all required fields are present"
        );
    }

    #[test]
    fn trigger_test_blank_token_fails() {
        let mut s = discord_toggled();
        // whitespace-only required field must NOT count as present.
        s.config_values
            .insert("discord.token".to_string(), "   ".to_string());
        s.focused_field = "test:discord".to_string();
        s.trigger_test();
        assert_eq!(
            s.test_statuses.get("discord").copied(),
            Some(TestStatus::Failed),
            "whitespace-only credential must not pass validation"
        );
    }

    // ── Blink-gated cursor render (Option A: canonical `▏`, drop static `▌`) ──

    use ratatui::{backend::TestBackend, Terminal};

    fn render_focused(cursor_on: bool) -> String {
        let mut s = discord_toggled();
        // Focus a real Discord field and give it a value so the caret trails it.
        s.focused_field = "discord.token".to_string();
        s.config_values
            .insert("discord.token".to_string(), "MTAxxxx".to_string());
        let mut term = Terminal::new(TestBackend::new(120, 40)).unwrap();
        term.draw(|f| {
            s.render_with_cursor(f.area(), f.buffer_mut(), cursor_on);
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

    fn dump_at(width: u16) -> String {
        let mut s = discord_toggled();
        // Toggle a QR-pairing (signal) + AppleScript (imessage) channel to exercise
        // badges + info lines at narrow width, plus a non-secret long value (matrix homeserver).
        s.toggled.push("signal".to_string());
        s.toggled.push("imessage".to_string());
        s.toggled.push("matrix".to_string());
        s.test_statuses.insert("discord".to_string(), TestStatus::Success);
        s.config_values.insert(
            "matrix.homeserver".to_string(),
            "https://matrix.veryverylonghomeservername.example.org:8448".to_string(),
        );
        s.config_values.insert(
            "discord.token".to_string(),
            "MTExNzU1NzE2NTc4NDEwNTM3Njg.GabcDe.veryLongDiscordBotTokenValueHere".to_string(),
        );
        let mut term = Terminal::new(TestBackend::new(width, 30)).unwrap();
        term.draw(|f| s.render_with_cursor(f.area(), f.buffer_mut(), false)).unwrap();
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

    /// #271 visual parity: at narrow width a long field value / subtitle / info
    /// line must (a) never overflow `inner.width` into the card chrome and
    /// (b) truncate with an ellipsis rather than a raw mid-word chop.
    /// A TestBackend at width 64 reproduces the clip; the assertions pin the
    /// clamp_ellipsis fix so it can't silently regress to unclamped set_string.
    #[test]
    fn narrow_width_values_clamp_with_ellipsis() {
        let out = dump_at(64);
        // The long homeserver URL must be ellipsis-truncated, not raw-clipped.
        assert!(
            out.contains("https://matrix.veryverylonghome…"),
            "long homeserver value must clamp with an ellipsis at narrow width.\n{out}"
        );
        // The full untruncated URL must NOT appear (proof it was clamped).
        assert!(
            !out.contains("matrix.veryverylonghomeservername.example.org:8448"),
            "untruncated homeserver must not render at width 64 (overflow guard).\n{out}"
        );
        // Info lines (imessage / signal) ellipsis-truncate, no mid-word chop bleed.
        assert!(
            out.contains("Will r…") || out.contains("Will re…"),
            "AppleScript info line must clamp with an ellipsis.\n{out}"
        );
        assert!(
            out.contains("displa…") || out.contains("display…"),
            "QR-pairing info line must clamp with an ellipsis.\n{out}"
        );
        // No glyph may be painted in the rightmost column (overflow into chrome).
        for line in out.lines() {
            let chars: Vec<char> = line.chars().collect();
            if chars.len() >= 64 {
                let last = chars[63];
                assert!(
                    last == ' ' || last == '…',
                    "rightmost column must be blank/ellipsis (no overflow), got {last:?} in: {line}"
                );
            }
        }
    }

    #[test]
    fn normal_width_renders_value_in_full() {
        // At width 100 the 58-char homeserver fits → renders complete, no premature clamp.
        let out = dump_at(100);
        assert!(
            out.contains("https://matrix.veryverylonghomeservername.example.org:8448"),
            "homeserver must render in full when it fits the width.\n{out}"
        );
    }

    #[test]
    fn clamp_ellipsis_budget_semantics() {
        assert_eq!(clamp_ellipsis("hello", 10), "hello"); // fits → unchanged
        assert_eq!(clamp_ellipsis("hello", 5), "hello"); // exact → unchanged
        assert_eq!(clamp_ellipsis("hello", 4), "hel…"); // clip → ellipsis
        assert_eq!(clamp_ellipsis("hello", 1), "…");
        assert_eq!(clamp_ellipsis("hello", 0), "");
    }

    #[test]
    fn caret_painted_on_blink_phase() {
        // cursor_on=true + focused field → canonical insertion glyph `▏` painted.
        let out = render_focused(true);
        assert!(
            out.contains('\u{258f}'),
            "expected canonical caret `▏` when cursor_on && field focused"
        );
    }

    #[test]
    fn caret_hidden_on_blink_off() {
        // cursor_on=false (blink "off" half-cycle) → no caret glyph.
        let out = render_focused(false);
        assert!(
            !out.contains('\u{258f}'),
            "caret must vanish on the blink-off phase"
        );
    }

    #[test]
    fn static_block_caret_is_gone() {
        // Option A unification: the old always-on `▌` must never be painted,
        // in either blink phase.
        assert!(
            !render_focused(true).contains('\u{258c}'),
            "static `▌` must be replaced by the blink-gated `▏`"
        );
        assert!(
            !render_focused(false).contains('\u{258c}'),
            "static `▌` must not appear on blink-off either"
        );
    }
}
