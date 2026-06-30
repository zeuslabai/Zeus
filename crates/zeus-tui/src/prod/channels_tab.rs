use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Clear, Widget};

use crate::api::ChannelResponse;
use crate::prod::draw::BufferClampExt;
use crate::theme;

const UNKNOWN: &str = "—";

/// Channel adapter status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelStatus {
    Connected,
    Reconnecting,
    Disconnected,
}

impl ChannelStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Connected => "connected",
            Self::Reconnecting => "reconnecting",
            Self::Disconnected => "disconnected",
        }
    }

    pub fn color(self) -> Color {
        match self {
            Self::Connected => theme::GREEN,
            Self::Reconnecting => theme::AMBER,
            Self::Disconnected => theme::DIM,
        }
    }

    pub fn dot(self) -> &'static str {
        "●"
    }

    /// Map a gateway channel `status` string to a display status.
    pub fn from_gateway(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "connected" | "active" | "online" | "ok" | "ready" => Self::Connected,
            "reconnecting" | "connecting" | "pending" | "starting" => Self::Reconnecting,
            _ => Self::Disconnected,
        }
    }
}

#[derive(Debug, Clone)]
struct ChannelCard {
    id: String,
    name: String,
    glyph: String,
    color: Color,
    status: ChannelStatus,
    binding: String,
    recent: String,
    msgs_24h: String,
    sdk: String,
    enabled: String,
}

impl ChannelCard {
    fn from_response(ch: &ChannelResponse) -> Self {
        let channel_key = first_non_empty(&[&ch.channel_type, &ch.id, &ch.name]);
        let name = first_non_empty(&[&ch.name, &title_case(&ch.channel_type), &ch.id]);
        let status = match ch.enabled {
            Some(false) => ChannelStatus::Disconnected,
            _ => ChannelStatus::from_gateway(&ch.status),
        };

        Self {
            id: display_or_dash(&ch.id),
            name: display_or_dash(&name),
            glyph: glyph_for(&channel_key),
            color: color_for(&channel_key),
            status,
            // /v1/channels does not expose account/workspace binding yet.
            binding: UNKNOWN.to_string(),
            recent: ch
                .last_message_at
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(UNKNOWN)
                .to_string(),
            // /v1/channels does not expose a 24h message counter yet.
            msgs_24h: UNKNOWN.to_string(),
            // /v1/channels does not expose adapter SDK/runtime metadata yet.
            sdk: UNKNOWN.to_string(),
            enabled: ch
                .enabled
                .map(|enabled| if enabled { "true" } else { "false" })
                .unwrap_or(UNKNOWN)
                .to_string(),
        }
    }
}

/// Production Channels tab: JSX-faithful messaging-adapter summary + cards,
/// backed only by live `/v1/channels` data. Missing API fields render `—`.
#[derive(Debug, Clone, Default)]
pub struct ChannelsTab {
    pub selected: usize,
    pub scroll: usize,
    pub live_channels: Option<Vec<ChannelResponse>>,
}

impl ChannelsTab {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_live(live: Option<&[ChannelResponse]>) -> Self {
        Self {
            live_channels: live.map(|rows| rows.to_vec()),
            ..Self::default()
        }
    }

    fn cards(&self) -> Vec<ChannelCard> {
        self.live_channels
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(ChannelCard::from_response)
            .collect()
    }
}

impl Widget for ChannelsTab {
    fn render(self, area: Rect, buf: &mut Buffer) {
        Clear.render(area, buf);
        if area.width < 20 || area.height < 6 {
            return;
        }

        fill_rect(buf, area, theme::BG);

        let cards = self.cards();
        let connected = cards
            .iter()
            .filter(|ch| ch.status == ChannelStatus::Connected)
            .count();
        let reconnecting = cards
            .iter()
            .filter(|ch| ch.status == ChannelStatus::Reconnecting)
            .count();
        let disconnected = cards
            .iter()
            .filter(|ch| ch.status == ChannelStatus::Disconnected)
            .count();

        let title = Style::default()
            .fg(theme::TEXT)
            .add_modifier(Modifier::BOLD);
        buf.set_string_clamped(area.x + 2, area.y, "Messaging adapters", title);
        buf.set_string_clamped(
            area.x + 2,
            area.y + 1,
            format!(
                "{} channels — all running in single zeus-channels process",
                cards.len()
            ),
            Style::default().fg(theme::DIM),
        );

        let chips = format!(
            "● {connected} CONNECTED   ● {reconnecting} RECONNECTING   ● {disconnected} DISCONNECTED"
        );
        let chip_width = chips.chars().count() as u16;
        let chip_x = area
            .right()
            .saturating_sub(chip_width.saturating_add(2))
            .max(area.x + 2);
        draw_status_chips(buf, chip_x, area.y, connected, reconnecting, disconnected);

        let list_y = area.y + 3;
        if cards.is_empty() {
            draw_empty_state(buf, area, list_y, self.live_channels.is_some());
            return;
        }

        let row_stride = 4;
        for (visible_idx, (source_idx, ch)) in
            cards.iter().enumerate().skip(self.scroll).enumerate()
        {
            let card_y = list_y + (visible_idx as u16 * row_stride);
            if card_y.saturating_add(2) >= area.bottom() {
                break;
            }
            draw_channel_card(buf, area, card_y, ch, source_idx == self.selected);
        }
    }
}

fn draw_status_chips(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    connected: usize,
    reconnecting: usize,
    disconnected: usize,
) {
    let mut cx = x;
    for (status, count, label) in [
        (ChannelStatus::Connected, connected, "CONNECTED"),
        (ChannelStatus::Reconnecting, reconnecting, "RECONNECTING"),
        (ChannelStatus::Disconnected, disconnected, "DISCONNECTED"),
    ] {
        let text = format!("{} {count} {label}", status.dot());
        buf.set_string_clamped(cx, y, text.as_str(), Style::default().fg(status.color()));
        cx = cx.saturating_add(text.chars().count() as u16 + 3);
    }
}

fn draw_empty_state(buf: &mut Buffer, area: Rect, y: u16, fetched: bool) {
    let width = area.width.saturating_sub(4).min(88);
    if y + 3 >= area.bottom() || width < 16 {
        return;
    }

    let x = area.x + 2;
    paint_span(buf, x, x + width, y, theme::BG_PANEL);
    paint_span(buf, x, x + width, y + 1, theme::BG_PANEL);
    paint_span(buf, x, x + width, y + 2, theme::BG_PANEL);
    let msg = if fetched {
        "No channel adapters returned by /v1/channels"
    } else {
        "Waiting for /v1/channels…"
    };
    buf.set_string_clamped(x + 2, y + 1, msg, Style::default().fg(theme::DIM));
}

fn draw_channel_card(buf: &mut Buffer, area: Rect, y: u16, ch: &ChannelCard, selected: bool) {
    let x = area.x + 2;
    let width = area.width.saturating_sub(4).min(120);
    if width < 28 {
        return;
    }

    let bg = if selected {
        theme::BG_HIGHLIGHT
    } else {
        theme::BG_PANEL
    };
    let border = if selected { ch.color } else { theme::MUTED };

    for row in 0..3 {
        paint_span(buf, x, x + width, y + row, bg);
    }

    buf.set_string_clamped(x, y, "▌", Style::default().fg(border).bg(bg));
    let glyph = format!("[{}]", ch.glyph);
    buf.set_string_clamped(x + 2, y, glyph, Style::default().fg(ch.color).bg(bg));
    buf.set_string_clamped(
        x + 9,
        y,
        ch.name.as_str(),
        Style::default()
            .fg(theme::TEXT)
            .bg(bg)
            .add_modifier(Modifier::BOLD),
    );
    buf.set_string_clamped(
        x + 9,
        y + 1,
        format!("id {}", ch.id),
        Style::default().fg(theme::DIM).bg(bg),
    );

    let status_text = format!("{} {}", ch.status.dot(), ch.status.label());
    let status_x = x
        .saturating_add(width)
        .saturating_sub(status_text.chars().count() as u16 + 2);
    buf.set_string_clamped(
        status_x,
        y,
        status_text,
        Style::default().fg(ch.status.color()).bg(bg),
    );

    let binding_x = x + 30;
    if binding_x < x + width.saturating_sub(12) {
        buf.set_string_clamped(
            binding_x,
            y,
            format!("binding {}", ch.binding),
            Style::default().fg(theme::TEXT).bg(bg),
        );
        buf.set_string_clamped(
            binding_x,
            y + 1,
            format!("recent {}", ch.recent),
            Style::default().fg(theme::DIM).bg(bg),
        );
    }

    let stats_x = x.saturating_add(width).saturating_sub(36);
    if stats_x > x + 45 {
        buf.set_string_clamped(
            stats_x,
            y + 1,
            format!("enabled {}", ch.enabled),
            Style::default().fg(theme::DIM).bg(bg),
        );
        buf.set_string_clamped(
            stats_x,
            y + 2,
            format!("{} MSGS / 24H", ch.msgs_24h),
            Style::default().fg(theme::FIRE_ORANGE).bg(bg),
        );
    }

    buf.set_string_clamped(
        x + 9,
        y + 2,
        format!("sdk {}", ch.sdk),
        Style::default().fg(theme::DIM).bg(bg),
    );

    let buttons = if ch.status == ChannelStatus::Connected {
        "[ TEST ] [ EDIT ] [ PAUSE ]"
    } else {
        "[ TEST ] [ EDIT ]"
    };
    let button_x = x
        .saturating_add(width)
        .saturating_sub(buttons.chars().count() as u16 + 2);
    if button_x > x + 10 {
        buf.set_string_clamped(
            button_x,
            y + 2,
            buttons,
            Style::default().fg(theme::DIM).bg(bg),
        );
    }

    if y + 3 < area.bottom() {
        let rule_width = width.saturating_sub(4) as usize;
        let rule = "─".repeat(rule_width.min(116));
        buf.set_string_clamped(x + 2, y + 3, rule, Style::default().fg(theme::MUTED));
    }
}

fn first_non_empty(values: &[&str]) -> String {
    values
        .iter()
        .map(|value| value.trim())
        .find(|value| !value.is_empty())
        .unwrap_or(UNKNOWN)
        .to_string()
}

fn display_or_dash(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        UNKNOWN.to_string()
    } else {
        trimmed.to_string()
    }
}

fn glyph_for(channel: &str) -> String {
    match channel.trim().to_ascii_lowercase().as_str() {
        "discord" => "DC".to_string(),
        "telegram" => "TG".to_string(),
        "slack" => "SL".to_string(),
        "email" => "EM".to_string(),
        "imessage" | "iMessage" => "iM".to_string(),
        "matrix" => "MX".to_string(),
        "whatsapp" => "WA".to_string(),
        "signal" => "SG".to_string(),
        "webhook" => "WH".to_string(),
        other => other
            .chars()
            .filter(|ch| ch.is_ascii_alphanumeric())
            .take(2)
            .collect::<String>()
            .to_ascii_uppercase(),
    }
}

fn color_for(channel: &str) -> Color {
    match channel.trim().to_ascii_lowercase().as_str() {
        "discord" => theme::PURPLE,
        "telegram" => theme::BLUE,
        "slack" => theme::GREEN,
        "email" => theme::AMBER,
        "imessage" | "whatsapp" => theme::GREEN,
        "matrix" => theme::CYAN,
        "signal" => theme::BLUE,
        "webhook" => theme::FIRE_ORANGE,
        _ => theme::DIM,
    }
}

fn title_case(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let mut chars = trimmed.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    format!(
        "{}{}",
        first.to_uppercase().collect::<String>(),
        chars.as_str().to_ascii_lowercase()
    )
}

fn fill_rect(buf: &mut Buffer, area: Rect, color: Color) {
    let style = Style::default().bg(color);
    for y in area.top()..area.bottom() {
        paint_span(buf, area.left(), area.right(), y, color);
        for x in area.left()..area.right() {
            if x < buf.area.right() && y < buf.area.bottom() {
                buf[(x, y)].set_style(style);
            }
        }
    }
}

fn paint_span(buf: &mut Buffer, left: u16, right: u16, y: u16, color: Color) {
    if y >= buf.area.bottom() {
        return;
    }
    let style = Style::default().bg(color);
    for x in left..right.min(buf.area.right()) {
        buf[(x, y)].set_symbol(" ").set_style(style);
    }
}
