use std::borrow::Cow;

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Block, Borders, Widget};

use crate::theme;

/// A model entry — matches JSX ModelStep models object (line ~793).
///
/// String fields are `Cow<'static, str>` so the const per-provider catalogs
/// stay zero-alloc (`Cow::Borrowed`, const-constructible) while live-fetched
/// entries own their names (`Cow::Owned`) with placeholder metadata.
struct ModelEntry {
    id: Cow<'static, str>,
    name: Cow<'static, str>,
    ctx: Cow<'static, str>,
    price: Cow<'static, str>,
    recommended: bool,
    sub: Cow<'static, str>,
}

// Per-provider model catalogs — keyed by the canonical provider ids in
// `crate::screens::providers`. Static catalogs for the API providers; the
// three live-fetch providers (ollama / zai / moonshot) carry a small seed
// catalog plus a runtime LIVE-FETCH notice (endpoint shown in render()).
const ANTHROPIC_MODELS: &[ModelEntry] = &[
    ModelEntry { id: Cow::Borrowed("claude-opus-4-8"), name: Cow::Borrowed("Claude Opus 4.8"), ctx: Cow::Borrowed("1M"), price: Cow::Borrowed("$15/$75 per Mtok"), recommended: true, sub: Cow::Borrowed("Most capable for reasoning + code") },
    ModelEntry { id: Cow::Borrowed("claude-sonnet-4-6"), name: Cow::Borrowed("Claude Sonnet 4.6"), ctx: Cow::Borrowed("200K"), price: Cow::Borrowed("$3/$15 per Mtok"), recommended: false, sub: Cow::Borrowed("Balanced cost / quality") },
    ModelEntry { id: Cow::Borrowed("claude-haiku-4-5"), name: Cow::Borrowed("Claude Haiku 4.5"), ctx: Cow::Borrowed("200K"), price: Cow::Borrowed("$0.80/$4 per Mtok"), recommended: false, sub: Cow::Borrowed("Fast + cheap") },
];

const OPENAI_MODELS: &[ModelEntry] = &[
    ModelEntry { id: Cow::Borrowed("gpt-4o"), name: Cow::Borrowed("GPT-4o"), ctx: Cow::Borrowed("128K"), price: Cow::Borrowed("$2.50/$10 per Mtok"), recommended: true, sub: Cow::Borrowed("Multimodal flagship") },
    ModelEntry { id: Cow::Borrowed("gpt-4o-mini"), name: Cow::Borrowed("GPT-4o-mini"), ctx: Cow::Borrowed("128K"), price: Cow::Borrowed("$0.15/$0.60 per Mtok"), recommended: false, sub: Cow::Borrowed("Cost-efficient") },
    ModelEntry { id: Cow::Borrowed("o1-pro"), name: Cow::Borrowed("o1-pro"), ctx: Cow::Borrowed("200K"), price: Cow::Borrowed("$15/$60 per Mtok"), recommended: false, sub: Cow::Borrowed("Deep reasoning") },
];

const GOOGLE_MODELS: &[ModelEntry] = &[
    ModelEntry { id: Cow::Borrowed("gemini-2.5-pro"), name: Cow::Borrowed("Gemini 2.5 Pro"), ctx: Cow::Borrowed("2M"), price: Cow::Borrowed("$1.25/$10 per Mtok"), recommended: true, sub: Cow::Borrowed("Flagship multimodal + long context") },
    ModelEntry { id: Cow::Borrowed("gemini-2.5-flash"), name: Cow::Borrowed("Gemini 2.5 Flash"), ctx: Cow::Borrowed("1M"), price: Cow::Borrowed("$0.30/$2.50 per Mtok"), recommended: false, sub: Cow::Borrowed("Fast + cheap") },
];

const GEMINI_CLI_MODELS: &[ModelEntry] = &[
    ModelEntry { id: Cow::Borrowed("gemini-2.5-pro"), name: Cow::Borrowed("Gemini 2.5 Pro"), ctx: Cow::Borrowed("2M"), price: Cow::Borrowed("OAuth (no key)"), recommended: true, sub: Cow::Borrowed("Via google-gemini/gemini-cli OAuth") },
    ModelEntry { id: Cow::Borrowed("gemini-2.5-flash"), name: Cow::Borrowed("Gemini 2.5 Flash"), ctx: Cow::Borrowed("1M"), price: Cow::Borrowed("OAuth (no key)"), recommended: false, sub: Cow::Borrowed("Fast + cheap") },
];

const KIMI_MODELS: &[ModelEntry] = &[
    ModelEntry { id: Cow::Borrowed("kimi-k2.7-code"), name: Cow::Borrowed("Kimi K2.7 Code"), ctx: Cow::Borrowed("256K"), price: Cow::Borrowed("$0.60/$2.50 per Mtok"), recommended: true, sub: Cow::Borrowed("Agentic coding flagship") },
    ModelEntry { id: Cow::Borrowed("kimi-k2.7-code-highspeed"), name: Cow::Borrowed("Kimi K2.7 Code (Highspeed)"), ctx: Cow::Borrowed("256K"), price: Cow::Borrowed("$1.00/$4 per Mtok"), recommended: false, sub: Cow::Borrowed("Lower-latency variant") },
];

const GLM_MODELS: &[ModelEntry] = &[
    ModelEntry { id: Cow::Borrowed("glm-5.2"), name: Cow::Borrowed("GLM-5.2"), ctx: Cow::Borrowed("200K"), price: Cow::Borrowed("$0.60/$2.20 per Mtok"), recommended: true, sub: Cow::Borrowed("Flagship code + reasoning") },
    ModelEntry { id: Cow::Borrowed("glm-4.6"), name: Cow::Borrowed("GLM-4.6"), ctx: Cow::Borrowed("200K"), price: Cow::Borrowed("$0.40/$1.60 per Mtok"), recommended: false, sub: Cow::Borrowed("Prior-gen, cheaper") },
];

const QWEN_MODELS: &[ModelEntry] = &[
    ModelEntry { id: Cow::Borrowed("qwen3-max"), name: Cow::Borrowed("Qwen3 Max"), ctx: Cow::Borrowed("256K"), price: Cow::Borrowed("$0.40/$1.20 per Mtok"), recommended: true, sub: Cow::Borrowed("Flagship agentic + multilingual") },
    ModelEntry { id: Cow::Borrowed("qwen3-coder"), name: Cow::Borrowed("Qwen3 Coder"), ctx: Cow::Borrowed("256K"), price: Cow::Borrowed("$0.30/$1.00 per Mtok"), recommended: false, sub: Cow::Borrowed("Code-specialized") },
];

const MINIMAX_MODELS: &[ModelEntry] = &[
    ModelEntry { id: Cow::Borrowed("MiniMax-M3"), name: Cow::Borrowed("MiniMax-M3"), ctx: Cow::Borrowed("1M"), price: Cow::Borrowed("$0.20/$0.80 per Mtok"), recommended: true, sub: Cow::Borrowed("Flagship throughput + agentic") },
    ModelEntry { id: Cow::Borrowed("MiniMax-M2"), name: Cow::Borrowed("MiniMax-M2"), ctx: Cow::Borrowed("245K"), price: Cow::Borrowed("$0.10/$0.40 per Mtok"), recommended: false, sub: Cow::Borrowed("Prior-gen, lighter") },
];

const MIMO_MODELS: &[ModelEntry] = &[
    ModelEntry { id: Cow::Borrowed("mimo-7b"), name: Cow::Borrowed("MiMo 7B"), ctx: Cow::Borrowed("32K"), price: Cow::Borrowed("$0.10/$0.40 per Mtok"), recommended: true, sub: Cow::Borrowed("Xiaomi compact, fast") },
];

const OPENROUTER_MODELS: &[ModelEntry] = &[
    ModelEntry { id: Cow::Borrowed("auto"), name: Cow::Borrowed("Auto (best for prompt)"), ctx: Cow::Borrowed("varies"), price: Cow::Borrowed("Varies by model"), recommended: true, sub: Cow::Borrowed("OpenRouter picks per request") },
    ModelEntry { id: Cow::Borrowed("anthropic/claude-opus-4-8"), name: Cow::Borrowed("Claude Opus 4.8"), ctx: Cow::Borrowed("1M"), price: Cow::Borrowed("$15/$75 per Mtok"), recommended: false, sub: Cow::Borrowed("Routed via OpenRouter") },
];

const XAI_MODELS: &[ModelEntry] = &[
    ModelEntry { id: Cow::Borrowed("grok-4"), name: Cow::Borrowed("Grok 4"), ctx: Cow::Borrowed("256K"), price: Cow::Borrowed("$3/$15 per Mtok"), recommended: true, sub: Cow::Borrowed("Real-time data, reasoning") },
    ModelEntry { id: Cow::Borrowed("grok-4-fast"), name: Cow::Borrowed("Grok 4 Fast"), ctx: Cow::Borrowed("256K"), price: Cow::Borrowed("$0.50/$2 per Mtok"), recommended: false, sub: Cow::Borrowed("Lower-latency variant") },
];

const SAKANA_MODELS: &[ModelEntry] = &[
    ModelEntry { id: Cow::Borrowed("fugu-ultra"), name: Cow::Borrowed("Fugu Ultra"), ctx: Cow::Borrowed("32K"), price: Cow::Borrowed("Low-cost"), recommended: true, sub: Cow::Borrowed("Sakana AI flagship") },
    ModelEntry { id: Cow::Borrowed("fugu"), name: Cow::Borrowed("Fugu"), ctx: Cow::Borrowed("32K"), price: Cow::Borrowed("Low-cost"), recommended: false, sub: Cow::Borrowed("Efficient, compact") },
];

// Seed catalogs for the live-fetch providers (shown until/if a live pull
// replaces them; the render() LIVE-FETCH notice names the endpoint).
const OLLAMA_MODELS: &[ModelEntry] = &[
    ModelEntry { id: Cow::Borrowed("(detected)"), name: Cow::Borrowed("Detected locally"), ctx: Cow::Borrowed("varies"), price: Cow::Borrowed("Free / self-hosted"), recommended: true, sub: Cow::Borrowed("Pulled from localhost:11434/api/tags") },
];

/// Model selection screen — step 5 of onboarding.
/// Matches JSX `ModelStep` (line 791) exactly.
pub struct ModelScreen {
    pub provider: String,
    pub selected: usize,
    /// Live-fetched catalog from the provider's `/v1/models` (P2 fetch worker).
    /// Empty = fall back to the static per-provider catalog. Non-empty = these
    /// entries (live names, placeholder metadata) drive the screen instead.
    live_models: Vec<ModelEntry>,
}

impl ModelScreen {
    pub fn new(provider: String) -> Self {
        Self {
            provider: provider.to_lowercase(),
            selected: 0,
            live_models: Vec::new(),
        }
    }

    /// Re-point the catalog at a newly-picked provider. Called when the Model
    /// step is (re)entered so the catalog tracks the Provider-screen choice
    /// instead of staying frozen at the construction-time provider. Resets the
    /// selection to the top of the new catalog (the old index may be out of
    /// range for a shorter catalog).
    pub fn set_provider(&mut self, provider: &str) {
        let lc = provider.to_lowercase();
        if self.provider != lc {
            self.provider = lc;
            self.selected = 0;
        }
    }

    /// Set the live-fetched model list (from the P2 `/v1/models` fetch worker).
    /// `names` are the model ids returned by the provider; they become owned
    /// `Cow::Owned` entries with placeholder metadata. The flagship (first
    /// entry, by provider convention) is marked `recommended`. An empty list
    /// clears live mode → the screen falls back to the static catalog.
    pub fn set_live_models(&mut self, names: Vec<String>) {
        self.live_models = names
            .into_iter()
            .enumerate()
            .map(|(i, n)| ModelEntry {
                id: Cow::Owned(n.clone()),
                name: Cow::Owned(n),
                ctx: Cow::Borrowed("—"),
                price: Cow::Borrowed("—"),
                recommended: i == 0,
                sub: Cow::Borrowed("Live from provider API"),
            })
            .collect();
        // A shorter live catalog may leave `selected` out of range.
        if self.selected >= self.live_models.len() && !self.live_models.is_empty() {
            self.selected = 0;
        }
    }

    /// Clear the live catalog → revert to the static per-provider list.
    pub fn clear_live_models(&mut self) {
        self.live_models.clear();
    }

    /// The active catalog: live-fetched entries when present, else the static
    /// per-provider catalog. The return lifetime narrows `&'static` → `&self`
    /// so a borrowed `&self.live_models` slice and the `'static` catalogs
    /// (coerced down via subtyping) share one signature.
    fn models(&self) -> &[ModelEntry] {
        if !self.live_models.is_empty() {
            return &self.live_models;
        }
        match self.provider.as_str() {
            "anthropic" => ANTHROPIC_MODELS,
            "openai" => OPENAI_MODELS,
            "google" => GOOGLE_MODELS,
            "gemini-cli" => GEMINI_CLI_MODELS,
            "kimi" => KIMI_MODELS,
            "glm" => GLM_MODELS,
            "qwen" => QWEN_MODELS,
            "minimax" => MINIMAX_MODELS,
            "mimo" => MIMO_MODELS,
            "openrouter" => OPENROUTER_MODELS,
            "xai" => XAI_MODELS,
            "sakana" => SAKANA_MODELS,
            "ollama" => OLLAMA_MODELS,
            // zai/moonshot are LIVE-FETCH only — no static seed; the notice in
            // render() names the endpoint and the catalog populates at runtime.
            _ => ANTHROPIC_MODELS,
        }
    }

    /// Runtime LIVE-FETCH endpoint for the three live providers, if any.
    /// Returns `(label, endpoint)` shown as the `● LIVE FETCH` notice.
    fn live_fetch_endpoint(&self) -> Option<(&'static str, &'static str)> {
        match self.provider.as_str() {
            "ollama" => Some(("Models pulled from", "localhost:11434/api/tags")),
            "glm" | "zai" => Some(("Live catalog from", "api.z.ai/api/paas/v4/models")),
            "kimi" | "moonshot" => Some(("Live catalog from", "api.moonshot.ai/v1/models")),
            _ => None,
        }
    }

    pub fn move_down(&mut self) {
        let len = self.models().len();
        if len > 0 {
            self.selected = (self.selected + 1) % len;
        }
    }

    pub fn move_up(&mut self) {
        let len = self.models().len();
        if len > 0 {
            self.selected = if self.selected == 0 { len - 1 } else { self.selected - 1 };
        }
    }

    pub fn selected_model_id(&self) -> &str {
        self.models().get(self.selected).map(|m| m.id.as_ref()).unwrap_or("")
    }

    /// Render the model screen into the given area.
    /// Matches JSX ModelStep after #271 parity:
    /// - Header: "Pick a model" + sub with provider name
    /// - Left list: slim bordered cards (`[GLY]` name + one-line desc)
    /// - Right detail panel: selected model id / context / pricing on full-width rows
    /// - Fetch-source notice stays driven by the real live-model list state
    pub fn render(&self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        Clear.render(area, buf);
        if area.width < 20 || area.height < 6 {
            return;
        }

        let models = self.models();
        let outer = Rect {
            x: area.x.saturating_add(1),
            y: area.y,
            width: area.width.saturating_sub(2),
            height: area.height,
        };
        let header_x = outer.x.saturating_add(1);
        let mut y = outer.y;

        // Header — "Pick a model" (jsx:814)
        buf.set_string(
            header_x,
            y,
            "Pick a model",
            Style::default()
                .fg(theme::WHITE)
                .add_modifier(Modifier::BOLD),
        );
        y += 1;

        // Sub — "From {provider}'s catalog. You can change anytime via zeus config set model …" (jsx:815)
        let sub_line = Line::from(vec![
            Span::styled("From ", Style::default().fg(theme::DIM)),
            Span::styled(&self.provider, Style::default().fg(theme::ACCENT_BRIGHT)),
            Span::styled(
                "'s catalog. You can change anytime via ",
                Style::default().fg(theme::DIM),
            ),
            Span::styled(
                "zeus config set model ...",
                Style::default().fg(theme::ACCENT_BRIGHT),
            ),
        ]);
        buf.set_line(header_x, y, &sub_line, outer.width.saturating_sub(2));
        y += 2;

        let content = Rect {
            x: outer.x,
            y,
            width: outer.width,
            height: outer.bottom().saturating_sub(y),
        };
        if content.width < 40 || content.height < 4 {
            return;
        }

        // Keep the list narrow like the JSX card rail; all dense data moves to
        // the detail panel so prices such as `$15/$75 per Mtok` cannot clip to
        // `$15/` in cramped card columns.
        let list_w = if content.width >= 96 {
            42
        } else if content.width >= 72 {
            36
        } else {
            (content.width / 2).max(28)
        }
        .min(content.width.saturating_sub(24));
        let gap = 2u16.min(content.width.saturating_sub(list_w));
        let detail_x = content.x.saturating_add(list_w).saturating_add(gap);
        let detail = Rect {
            x: detail_x,
            y: content.y,
            width: content.right().saturating_sub(detail_x),
            height: content.height,
        };
        let list = Rect {
            x: content.x,
            y: content.y,
            width: list_w,
            height: content.height,
        };

        render_model_list(list, buf, models, self.selected);
        if let Some(selected) = models.get(self.selected).or_else(|| models.first()) {
            render_model_detail(
                detail,
                buf,
                &self.provider,
                selected,
                self.live_fetch_endpoint(),
                !self.live_models.is_empty(),
            );
        }
    }
}

fn render_model_list(
    area: Rect,
    buf: &mut ratatui::buffer::Buffer,
    models: &[ModelEntry],
    selected: usize,
) {
    // Slim card (#271) = 2 content rows (badge+name / 1-line desc) + border +
    // 1-row gutter. Context/pricing/id live only in the detail panel.
    let item_height = 5u16;
    let visible_count = (area.height / item_height.max(1)) as usize;
    let scroll_start = if selected >= visible_count {
        selected.saturating_sub(visible_count / 2)
    } else {
        0
    }
    .min(models.len().saturating_sub(visible_count.max(1)));

    for (idx, model) in models
        .iter()
        .enumerate()
        .skip(scroll_start)
        .take(visible_count)
    {
        let row_y = area.y + ((idx - scroll_start) as u16) * item_height;
        if row_y + 4 > area.bottom() {
            break;
        }

        let is_selected = idx == selected;
        let card = Rect {
            x: area.x.saturating_add(1),
            y: row_y,
            width: area.width.saturating_sub(2),
            height: 4,
        };
        if card.width < 8 || card.height < 3 {
            continue;
        }

        let border_color = if is_selected {
            theme::ACCENT
        } else {
            theme::MUTED
        };
        let left_accent = if is_selected {
            theme::ACCENT
        } else {
            theme::DIM
        };
        let bg_style = if is_selected {
            Style::default().bg(theme::ACCENT_FAINT)
        } else {
            Style::default().bg(theme::BG_PANEL)
        };

        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .render(card, buf);

        for row in 0..card.height {
            let py = card.y + row;
            if py < area.bottom() {
                buf[(card.x, py)]
                    .set_symbol("│")
                    .set_style(Style::default().fg(left_accent));
            }
        }
        for row in 1..card.height.saturating_sub(1) {
            for col in 1..card.width.saturating_sub(1) {
                buf[(card.x + col, card.y + row)].set_style(bg_style);
            }
        }

        if is_selected && card.left() > area.left() {
            buf[(card.left() - 1, card.top() + 1)]
                .set_symbol("▸")
                .set_style(
                    Style::default()
                        .fg(theme::FIRE_ORANGE)
                        .add_modifier(Modifier::BOLD),
                );
        }

        let badge = provider_glyph(model.id.as_ref());
        let badge_style = if is_selected {
            Style::default()
                .fg(theme::BG)
                .bg(theme::ACCENT)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(theme::ACCENT_BRIGHT)
                .bg(theme::BG_PANEL)
                .add_modifier(Modifier::BOLD)
        };
        let badge_text = format!("[{badge}]");
        let name_x = card.x + 2 + badge_text.len() as u16 + 1;
        buf.set_string(card.x + 2, card.y + 1, badge_text, badge_style);
        let name_width = card.right().saturating_sub(name_x).saturating_sub(2) as usize;
        let name = truncate_for_width(model.name.as_ref(), name_width);
        let name_style = if is_selected {
            Style::default()
                .fg(theme::WHITE)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme::TEXT)
        };
        buf.set_string(name_x, card.y + 1, name, name_style);

        let sub_width = card.width.saturating_sub(4) as usize;
        let sub = truncate_for_width(model.sub.as_ref(), sub_width);
        buf.set_string(card.x + 2, card.y + 2, sub, Style::default().fg(theme::DIM));
    }
}

fn render_model_detail(
    area: Rect,
    buf: &mut ratatui::buffer::Buffer,
    provider: &str,
    model: &ModelEntry,
    live_fetch: Option<(&'static str, &'static str)>,
    live_models_loaded: bool,
) {
    if area.width < 20 || area.height < 4 {
        return;
    }

    // Clear/repaint the whole detail column before direct writes. This mirrors
    // the provider parity fix and prevents stale wider detail rows from
    // surviving after selection/provider changes.
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            buf[(x, y)]
                .set_symbol(" ")
                .set_style(Style::default().fg(theme::TEXT).bg(theme::BG));
        }
    }

    for y in area.top()..area.bottom() {
        buf[(area.left(), y)]
            .set_symbol("│")
            .set_style(Style::default().fg(theme::MUTED));
    }

    let inner = Rect {
        x: area.x.saturating_add(2),
        y: area.y,
        width: area.width.saturating_sub(2),
        height: area.height,
    };
    if inner.width < 8 {
        return;
    }

    let mut y = inner.y;
    let title = Line::from(vec![
        Span::styled(
            model.name.as_ref(),
            Style::default()
                .fg(theme::WHITE)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ", Style::default()),
        Span::styled(model.id.as_ref(), Style::default().fg(theme::MUTED)),
    ]);
    buf.set_line(inner.x, y, &title, inner.width);
    y += 1;
    buf.set_line(
        inner.x,
        y,
        &Line::from(Span::styled(
            model.sub.as_ref(),
            Style::default().fg(theme::DIM),
        )),
        inner.width,
    );
    y += 2;

    buf.set_string(
        inner.x,
        y,
        "MODEL DETAILS",
        Style::default()
            .fg(theme::ACCENT_DIM)
            .add_modifier(Modifier::BOLD),
    );
    y += 1;
    render_detail_row(
        buf,
        inner,
        y,
        "PROVIDER",
        provider,
        Style::default().fg(theme::ACCENT_BRIGHT),
    );
    y += 1;
    render_detail_row(
        buf,
        inner,
        y,
        "MODEL ID",
        model.id.as_ref(),
        Style::default().fg(theme::TEXT),
    );
    y += 1;
    render_detail_row(
        buf,
        inner,
        y,
        "CONTEXT",
        model.ctx.as_ref(),
        Style::default()
            .fg(theme::ACCENT)
            .add_modifier(Modifier::BOLD),
    );
    y += 1;
    render_detail_row(
        buf,
        inner,
        y,
        "PRICING",
        model.price.as_ref(),
        Style::default().fg(theme::TEXT),
    );
    y += 1;
    if model.recommended {
        render_detail_row(
            buf,
            inner,
            y,
            "BADGE",
            "★ RECOMMENDED",
            Style::default()
                .fg(theme::GREEN)
                .add_modifier(Modifier::BOLD),
        );
        y += 1;
    }

    y += 1;
    if y + 3 < inner.bottom() {
        let filter_area = Rect {
            x: inner.x,
            y,
            width: inner.width,
            height: 3,
        };
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::MUTED))
            .render(filter_area, buf);
        buf.set_string(
            filter_area.left() + 2,
            filter_area.top(),
            "SEARCH / FILTER",
            Style::default()
                .fg(theme::ACCENT_DIM)
                .add_modifier(Modifier::BOLD),
        );
        let filter = Line::from(vec![
            Span::styled("/", Style::default().fg(theme::ACCENT_BRIGHT)),
            Span::styled(" search models", Style::default().fg(theme::TEXT)),
            Span::styled("  ·  ", Style::default().fg(theme::MUTED)),
            Span::styled("recommended", Style::default().fg(theme::GREEN)),
            Span::styled(" / context / price", Style::default().fg(theme::DIM)),
        ]);
        buf.set_line(
            filter_area.left() + 2,
            filter_area.top() + 1,
            &filter,
            filter_area.width.saturating_sub(4),
        );
        y = filter_area.bottom() + 1;
    }

    if y + 3 < inner.bottom() {
        let box_area = Rect {
            x: inner.x,
            y,
            width: inner.width,
            height: 4,
        };
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::ACCENT_DIM))
            .render(box_area, buf);
        buf.set_string(
            box_area.left() + 2,
            box_area.top() + 1,
            "WILL WRITE TO ~/.zeus/config.toml",
            Style::default().fg(theme::DIM).add_modifier(Modifier::BOLD),
        );
        let cfg = Line::from(vec![
            Span::styled("model", Style::default().fg(theme::TEXT)),
            Span::styled(" = ", Style::default().fg(theme::MUTED)),
            Span::styled(
                format!("\"{provider}/{}\"", model.id),
                Style::default().fg(theme::ACCENT_BRIGHT),
            ),
        ]);
        buf.set_line(
            box_area.left() + 2,
            box_area.top() + 2,
            &cfg,
            box_area.width.saturating_sub(4),
        );
        y = box_area.bottom() + 1;
    }

    if let Some((label, endpoint)) = live_fetch {
        if y < inner.bottom() {
            let notice = if live_models_loaded {
                Line::from(vec![
                    Span::styled(
                        "● LIVE FETCH",
                        Style::default()
                            .fg(theme::CYAN)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(format!("  {label}"), Style::default().fg(theme::DIM)),
                ])
            } else {
                Line::from(vec![
                    Span::styled(
                        "○ FALLBACK",
                        Style::default().fg(theme::DIM).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        "  live fetch unavailable — showing seed catalog",
                        Style::default().fg(theme::DIM),
                    ),
                ])
            };
            buf.set_line(inner.x, y, &notice, inner.width);
        }
        if y + 1 < inner.bottom() {
            // Endpoint on its own full-width row: the 80×30 TestBackend gate
            // must see the complete source, not a clipped tail after prose.
            buf.set_string(
                inner.x,
                y + 1,
                endpoint,
                Style::default().fg(if live_models_loaded {
                    theme::ACCENT_BRIGHT
                } else {
                    theme::DIM
                }),
            );
        }
    }
}

fn render_detail_row(
    buf: &mut ratatui::buffer::Buffer,
    inner: Rect,
    y: u16,
    label: &str,
    value: &str,
    value_style: Style,
) {
    if y >= inner.bottom() {
        return;
    }
    let label_w = 12u16.min(inner.width.saturating_sub(1));
    buf.set_string(
        inner.x,
        y,
        label,
        Style::default()
            .fg(theme::MUTED)
            .add_modifier(Modifier::BOLD),
    );
    let value_x = inner.x.saturating_add(label_w);
    let value_w = inner.right().saturating_sub(value_x);
    if value_w > 0 {
        let value = truncate_for_width(value, value_w as usize);
        buf.set_string(value_x, y, value, value_style);
    }
}

fn provider_glyph(id: &str) -> &'static str {
    match id.split(['-', '/', ':']).next().unwrap_or(id) {
        "claude" => "ANT",
        "gpt" | "o1" | "o3" | "o4" => "OAI",
        "gemini" => "GCP",
        "kimi" | "moonshot" => "KIM",
        "glm" => "GLM",
        "qwen" => "QWN",
        "abab" | "minimax" => "MNX",
        "mimo" => "MIM",
        "grok" => "XAI",
        "sakana" => "SAK",
        "llama" | "mistral" | "codellama" => "OLM",
        _ => "LLM",
    }
}

fn truncate_for_width(value: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let mut chars: Vec<char> = value.chars().collect();
    if chars.len() <= width {
        return value.to_string();
    }
    if width == 1 {
        return "…".to_string();
    }
    chars.truncate(width - 1);
    chars.push('…');
    chars.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Default screen with no live fetch → the static per-provider catalog
    /// drives `models()` (anthropic seed here). Pins the fallback path.
    #[test]
    fn empty_live_uses_static_catalog() {
        let s = ModelScreen::new("anthropic".to_string());
        assert!(s.live_models.is_empty(), "fresh screen has no live models");
        assert!(!s.models().is_empty(), "static catalog drives the screen");
        assert_eq!(s.models()[0].name, "Claude Opus 4.8",
            "models() returns the static anthropic catalog when live is empty");
    }

    /// A non-empty live list (from the P2 `/v1/models` fetch) overrides the
    /// static catalog: the screen shows exactly the fetched names, the first
    /// entry is flagged `recommended` (flagship convention), and metadata is
    /// the sparse placeholder. Pins the #239 live-Model-page contract.
    #[test]
    fn live_models_override_static_and_flag_flagship() {
        let mut s = ModelScreen::new("openai".to_string());
        s.set_live_models(vec!["gpt-5".into(), "gpt-5-mini".into(), "o3".into()]);

        let m = s.models();
        assert_eq!(m.len(), 3, "live list drives the catalog, not the static one");
        assert_eq!(m[0].name, "gpt-5");
        assert_eq!(m[0].id, "gpt-5");
        assert!(m[0].recommended, "first live entry is the flagship");
        assert!(!m[1].recommended && !m[2].recommended, "only the flagship is recommended");
        assert_eq!(m[1].ctx, "—", "live entries carry placeholder metadata");
        assert_eq!(s.selected_model_id(), "gpt-5", "selected reads from the live list");
    }

    /// Clearing the live list reverts to the static per-provider catalog —
    /// the proceed-after-failure / standalone fallback path.
    #[test]
    fn clear_live_reverts_to_static() {
        let mut s = ModelScreen::new("anthropic".to_string());
        s.set_live_models(vec!["x".into(), "y".into()]);
        assert_eq!(s.models().len(), 2, "live active");
        s.clear_live_models();
        assert_eq!(s.models()[0].name, "Claude Opus 4.8",
            "cleared -> static anthropic catalog again");
    }

    // ---- #251 render-fidelity: the LIVE FETCH badge is honest ----

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;

    fn render_buffer(s: &ModelScreen, width: u16, height: u16) -> Buffer {
        let mut term = Terminal::new(TestBackend::new(width, height)).unwrap();
        term.draw(|f| {
            let area = f.area();
            s.render(area, f.buffer_mut());
        })
        .unwrap();
        term.backend().buffer().clone()
    }

    /// Render a ModelScreen and return the flattened buffer text.
    fn render_text(s: &ModelScreen) -> String {
        let buf = render_buffer(s, 120, 40);
        buf.content().iter().map(|c| c.symbol()).collect::<String>()
    }

    /// Dump a rectangular region as rows of terminal glyphs. This is a real
    /// TestBackend buffer check, not token/palette matching, so clipped prices
    /// show up exactly as the terminal would render them.
    fn region_text(buf: &Buffer, x: u16, y: u16, width: u16, height: u16) -> String {
        let area = buf.area();
        let right = x.saturating_add(width).min(area.right());
        let bottom = y.saturating_add(height).min(area.bottom());
        let mut out = String::new();
        for row in y.min(area.bottom())..bottom {
            for col in x.min(area.right())..right {
                out.push_str(buf[(col, row)].symbol());
            }
            out.push('\n');
        }
        out
    }

    /// A glm provider with NO live models landed shows the honest `○ FALLBACK`
    /// notice — NOT the `● LIVE FETCH` badge (the fake-badge bug #251 fixed).
    #[test]
    fn glm_without_live_models_renders_fallback_not_live_badge() {
        let s = ModelScreen::new("glm".to_string());
        assert!(s.live_models.is_empty(), "no fetch landed");
        let text = render_text(&s);
        assert!(text.contains("○ FALLBACK"),
            "empty live list must render the honest FALLBACK notice");
        assert!(!text.contains("● LIVE FETCH"),
            "no fake LIVE FETCH badge when the static seed catalog is showing");
    }

    /// The same glm provider WITH a live list shows the real `● LIVE FETCH`
    /// badge — both states render distinctly off the same surface.
    #[test]
    fn glm_with_live_models_renders_live_badge_not_fallback() {
        let mut s = ModelScreen::new("glm".to_string());
        s.set_live_models(vec!["glm-5.2".into(), "glm-4.6".into()]);
        let text = render_text(&s);
        assert!(text.contains("● LIVE FETCH"),
            "a real fetch must render the LIVE FETCH badge");
        assert!(!text.contains("○ FALLBACK"),
            "live list present -> no fallback notice");
    }
    /// #271 visual-parity gate: slim list cards must not cram context/pricing
    /// into the narrow column, while the detail panel must render the full
    /// selected-model price. This uses a TestBackend region dump so a real
    /// `$15/$75 -> $15/` terminal clip fails loudly.
    #[test]
    fn slim_cards_move_full_pricing_to_detail_panel() {
        let s = ModelScreen::new("anthropic".to_string());
        let buf = render_buffer(&s, 100, 30);

        // For a 100-col backend render() lays out: content x=1, list width=42,
        // gap=2, detail x=45. Keep the regions explicit so this test catches
        // accidental dense fields returning to the list column.
        let list_dump = region_text(&buf, 0, 3, 44, 20);
        let detail_dump = region_text(&buf, 45, 3, 55, 20);

        assert!(
            list_dump.contains("[ANT] Claude Opus 4.8"),
            "slim card should keep glyph+name in the list column:
{list_dump}"
        );
        assert!(
            list_dump.contains("Most capable for reasoning + code"),
            "slim card should keep the 1-line description in the list column:
{list_dump}"
        );
        assert!(
            !list_dump.contains("$15/$75") && !list_dump.contains("PRICING"),
            "pricing must live in the full-width detail panel, not the cramped list:
{list_dump}"
        );
        assert!(
            !list_dump.contains("CONTEXT"),
            "context metadata must live in the full-width detail panel:
{list_dump}"
        );
        assert!(
            detail_dump.contains("PRICING") && detail_dump.contains("$15/$75 per Mtok"),
            "detail panel must render the full price without clipping:
{detail_dump}"
        );
        assert!(
            detail_dump.contains("CONTEXT") && detail_dump.contains("1M"),
            "detail panel must render context on a full-width row:
{detail_dump}"
        );
    }
}
