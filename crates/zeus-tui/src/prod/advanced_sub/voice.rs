//! Voice — Calls, STT/TTS config, recordings.
//!
//! Advanced subview (id: `voice`). Matches JSX `AdvancedSubview` `voice`
//! branch (docs/zeus-tui-production.jsx ~line 1489): a 2-column grid of four
//! config cards — TTS PROVIDER, STT PROVIDER, TWILIO, RECORDINGS. Each card:
//! accent-dim label (letter-spaced), white bold value, dim detail line.
//! Theme tokens, geometric glyphs, no emoji, opaque-bg inherited.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Clear, Widget};

use crate::api::{TtsProviderResponse, TtsVoiceResponse};
use crate::prod::draw::cell_mut_clamped;
use crate::theme;

/// A voice config card. `value`/`detail` are owned so the TTS PROVIDER card can
/// carry live data overlaid from `/v1/tts/providers` + `/v1/tts/voices`; the
/// other three cards (STT/Twilio/Recordings have no backend) stay const.
struct Card {
    label: &'static str,
    value: String,
    detail: String,
}

/// Const fallback text — used verbatim until the TTS poll-worker lands, and for
/// the three cards with no backend (STT/Twilio/Recordings → honest stub).
const TTS_VALUE: &str = "ElevenLabs";
const TTS_DETAIL: &str = "voice · Aria · 11labs_v2";

/// Build the four config cards, overlaying live TTS data onto the first card
/// when fetched. `None` → const fallback (pre-fetch, JSX-verbatim text).
fn build_cards(
    tts_providers: Option<&[TtsProviderResponse]>,
    tts_voices: Option<&[TtsVoiceResponse]>,
) -> [Card; 4] {
    // TTS PROVIDER card — live overlay when a provider is fetched.
    let (tts_value, tts_detail) = match tts_providers.and_then(|p| p.first()) {
        Some(p) => {
            let value = p.name.clone();
            // Detail: voice count · first voice name · status (honest, from
            // what the endpoints actually expose — provider has status, voices
            // have name; no model-id field, so we report what's real).
            let voice_n = tts_voices.map(|v| v.len()).unwrap_or(0);
            let first_voice = tts_voices
                .and_then(|v| v.first())
                .map(|v| v.name.as_str())
                .unwrap_or("—");
            let status = p.status.as_deref().unwrap_or("ready");
            (value, format!("{voice_n} voices · {first_voice} · {status}"))
        }
        None => (TTS_VALUE.to_string(), TTS_DETAIL.to_string()),
    };

    [
        Card { label: "TTS PROVIDER", value: tts_value, detail: tts_detail },
        // STT provider is a config default (no live telemetry endpoint) — show
        // the configured engine, drop the fabricated latency metric.
        Card { label: "STT PROVIDER", value: "Whisper · Groq".into(), detail: "whisper-large-v3".into() },
        // TWILIO + RECORDINGS have no backend (#260/#266): no fabricated phone
        // number, call counts, session counts, or sizes. Honest "not configured"
        // / dashed path until a real endpoint lands.
        Card { label: "TWILIO", value: "—".into(), detail: "not configured".into() },
        Card { label: "RECORDINGS", value: "—".into(), detail: "~/.zeus/voice/".into() },
    ]
}

/// Render the `voice` subview body into `area`.
///
/// Overlays live TTS provider/voice data onto the TTS PROVIDER card when the
/// poll-worker has fetched it; STT/Twilio/Recordings have no backend and stay
/// const (honest in-panel stub). `tts_*` are `None` until the first fetch.
pub fn render(
    area: Rect,
    buf: &mut Buffer,
    tts_providers: Option<&[TtsProviderResponse]>,
    tts_voices: Option<&[TtsVoiceResponse]>,
) {
    Clear.render(area, buf);
    let cards = build_cards(tts_providers, tts_voices);
    if area.width < 4 || area.height < 1 {
        return;
    }
    let right = area.right().min(buf.area.right());

    // 2-column grid. Gutter of 1 col between cards (JSX gap: 10).
    let inner_left = area.x + 1;
    let inner_right = right.saturating_sub(1);
    let total_w = inner_right.saturating_sub(inner_left);
    if total_w < 6 {
        return;
    }
    let gutter = 1u16;
    let col_w = (total_w.saturating_sub(gutter)) / 2;
    if col_w < 3 {
        return;
    }
    let col_x = [inner_left, inner_left + col_w + gutter];

    // Card height: label + value + detail + top pad = 4 rows; +1 gutter row.
    let card_h = 4u16;
    let row_gap = 1u16;

    for (i, card) in cards.iter().enumerate() {
        let cx = col_x[i % 2];
        let row = (i / 2) as u16;
        let cy = area.y + 1 + row * (card_h + row_gap);
        if cy >= area.bottom() {
            break;
        }
        let card_right = (cx + col_w).min(right);
        draw_card(cx, cy, card_right, card_h, card, buf);
    }
}

/// Draw one config card: bg panel + muted top border, then label/value/detail.
fn draw_card(x: u16, y: u16, right: u16, h: u16, card: &Card, buf: &mut Buffer) {
    // Panel background.
    for ry in y..(y + h).min(buf.area.bottom()) {
        for cx in x..right.min(buf.area.right()) {
            if let Some(c) = cell_mut_clamped(buf, cx, ry) { c.set_bg(theme::BG_PANEL); }
        }
    }
    let tx = x + 1;
    // Label (accent-dim, bold, letter-spaced to echo the JSX tracking).
    let label = spaced(card.label);
    let _ = set_str(tx, y, &label, Style::default().fg(theme::ACCENT_DIM).add_modifier(Modifier::BOLD), right, buf);
    // Value (white bold).
    if y + 1 < buf.area.bottom() {
        let _ = set_str(tx, y + 1, &card.value, Style::default().fg(theme::WHITE).add_modifier(Modifier::BOLD), right, buf);
    }
    // Detail (dim).
    if y + 2 < buf.area.bottom() {
        let _ = set_str(tx, y + 2, &card.detail, Style::default().fg(theme::DIM), right, buf);
    }
}

/// Insert a hair space between chars to echo JSX `letterSpacing` on labels.
fn spaced(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for (i, ch) in s.chars().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        out.push(ch);
    }
    out
}

/// Write `s` at (x,y), clipped to `max_x`. Returns x after the last cell.
fn set_str(x: u16, y: u16, s: &str, style: Style, max_x: u16, buf: &mut Buffer) -> u16 {
    let mut cx = x;
    for ch in s.chars() {
        if cx >= max_x || cx >= buf.area.right() {
            break;
        }
        if let Some(c) = cell_mut_clamped(buf, cx, y) { c.set_char(ch).set_style(style); }
        cx += 1;
    }
    cx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voice_match_jsx() {
        // None → const fallback, JSX-verbatim text.
        let cards = build_cards(None, None);
        assert_eq!(cards.len(), 4);
        assert_eq!(cards[0].label, "TTS PROVIDER");
        assert_eq!(cards[0].value, "ElevenLabs");
        assert_eq!(cards[1].value, "Whisper · Groq");
        // TWILIO + RECORDINGS have no backend → honest dash, not fabricated
        // phone number / session counts (#260/#266).
        assert_eq!(cards[2].label, "TWILIO");
        assert_eq!(cards[2].value, "—");
        assert_eq!(cards[2].detail, "not configured");
        assert_eq!(cards[3].label, "RECORDINGS");
        assert_eq!(cards[3].value, "—");
    }

    #[test]
    fn live_tts_overlays_first_card() {
        // Populated TTS provider + voices → TTS PROVIDER card shows live data;
        // the other three cards stay const (no backend).
        let providers = vec![TtsProviderResponse {
            name: "OpenAI".into(),
            status: Some("ready".into()),
            description: None,
        }];
        let voices = vec![
            TtsVoiceResponse { provider: "OpenAI".into(), voice_id: "alloy".into(), name: "Alloy".into(), gender: None },
            TtsVoiceResponse { provider: "OpenAI".into(), voice_id: "echo".into(), name: "Echo".into(), gender: None },
        ];
        let cards = build_cards(Some(&providers), Some(&voices));
        assert_eq!(cards[0].value, "OpenAI");
        assert_eq!(cards[0].detail, "2 voices · Alloy · ready");
        // Untouched cards stay honest (no fabricated numbers).
        assert_eq!(cards[1].value, "Whisper · Groq");
        assert_eq!(cards[3].value, "—");
    }

    #[test]
    fn render_no_panic() {
        let area = Rect::new(0, 0, 100, 30);
        let mut buf = Buffer::empty(area);
        render(area, &mut buf, None, None);
        render(Rect::new(0, 0, 3, 1), &mut buf, None, None);
        render(Rect::new(0, 0, 8, 2), &mut buf, None, None);
    }
}
