//! Onboarding renderer — pixel-perfect JSX implementation.

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, Paragraph, Gauge},
};

use super::{OnboardingState, OnboardingStep, Provider, PROVIDERS, CHANNELS, SECURITY_LEVELS};

// Subsystem definitions for the Features step
static FEATURES: &[(&str, &str, &str)] = &[
    ("nous",       "Cognitive",     "Reasoning, planning, and self-reflection loop"),
    ("mnemosyne",  "Memory",        "Long-term memory with SQLite FTS5 + vector embeddings"),
    ("aegis",      "Security",      "Sandboxing, path filtering, and command restrictions"),
    ("athena",     "Docs",          "Automatic documentation generation and indexing"),
    ("hermes",     "Notifications", "Cross-channel alerting and escalation routing"),
    ("prometheus", "Orchestration", "Multi-agent heartbeat and task coordination"),
    // Abilities — external integrations (off by default, opt-in)
    ("browser",    "Browser",       "Chrome CDP automation — navigate, click, screenshot"),
    ("talos",      "Talos",         "Native macOS/cross-platform automation tools"),
    ("mcp",        "MCP",           "Model Context Protocol — external tool servers"),
];

// Zeus TUI color palette (from JSX)
mod colors {
    #![allow(dead_code)]
    use ratatui::style::{Color, Style};

    pub const BG: Color         = Color::Rgb(10, 10, 15);
    pub const FG: Color         = Color::Rgb(212, 207, 200);
    pub const DIM: Color        = Color::Rgb(90, 86, 80);
    pub const ACCENT: Color     = Color::Rgb(255, 60, 20);
    pub const ACCENT_BRIGHT: Color = Color::Rgb(255, 104, 66);
    pub const ACCENT_DIM: Color = Color::Rgb(160, 48, 26);
    pub const GREEN: Color      = Color::Rgb(34, 197, 94);
    pub const YELLOW: Color     = Color::Rgb(234, 179, 8);
    pub const RED: Color        = Color::Rgb(239, 68, 68);
    pub const PURPLE: Color     = Color::Rgb(168, 85, 247);
    pub const TEXT: Color       = Color::Rgb(212, 207, 200);
    pub const TEXT_BRIGHT: Color= Color::Rgb(240, 236, 230);
    pub const BORDER: Color     = Color::Rgb(46, 34, 24);
    pub const BORDER_BRIGHT: Color = Color::Rgb(90, 56, 32);
    pub const BG_PANEL: Color   = Color::Rgb(16, 14, 20);

    pub fn border() -> Style {
        Style::default().fg(BORDER)
    }
    pub fn border_active() -> Style {
        Style::default().fg(BORDER_BRIGHT)
    }
}

/// Render the full onboarding screen for the current step.
/// Truncate a block title to fit within available width, adding "..." if needed.
fn truncate_title(title: &str, max_width: u16) -> String {
    let max = max_width.saturating_sub(4) as usize; // 2 for borders + 2 padding
    if title.len() <= max { title.to_string() } else if max > 3 { format!("{}...", &title[..max - 3]) } else { title[..max].to_string() }
}

pub fn render_onboarding(f: &mut Frame, state: &OnboardingState) {
    let area = f.area();

    // Background
    f.render_widget(
        Block::default().style(Style::default().bg(colors::BG)),
        area,
    );

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // top bar
            Constraint::Length(3), // step progress gauge
            Constraint::Length(1), // step name bar
            Constraint::Min(0),    // main content
            Constraint::Length(1), // bottom help
        ])
        .split(area);

    render_top_bar(f, chunks[0]);
    render_progress(f, state, chunks[1]);
    render_step_name(f, state, chunks[2]);
    render_step_content(f, state, chunks[3]);
    render_bottom_help(f, state, chunks[4]);
}

fn render_top_bar(f: &mut Frame, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(12)])
        .split(area);

    let left = Paragraph::new(Line::from(vec![
        Span::styled(" ZEUS ", Style::default().fg(colors::RED).add_modifier(Modifier::BOLD)),
        Span::styled(" onboard", Style::default().fg(colors::DIM)),
    ]));
    f.render_widget(left, chunks[0]);

    let right = Paragraph::new(Line::from(
        Span::styled(concat!(env!("CARGO_PKG_VERSION"), " "), Style::default().fg(colors::DIM))
    )).alignment(Alignment::Right);
    f.render_widget(right, chunks[1]);
}

fn render_progress(f: &mut Frame, state: &OnboardingState, area: Rect) {
    let idx = state.step.index();
    let total = OnboardingStep::total();

    // Split: left for progress bar + label, right for step dots
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length((total + 2) as u16)])
        .split(area);

    // Left: progress gauge with label
    let ratio = if total <= 1 { 0.0 } else { (idx as f64) / ((total - 1) as f64) };
    let label = format!(
        " STEP {}/{}  {}",
        idx + 1, total,
        state.step.title()
    );
    let gauge = Gauge::default()
        .block(Block::default().borders(Borders::BOTTOM).border_style(colors::border()))
        .gauge_style(Style::default().fg(colors::ACCENT).bg(colors::BG_PANEL))
        .ratio(ratio)
        .label(label);
    f.render_widget(gauge, chunks[0]);

    // Right: colored dots (green=done, red=current, dim=future)
    let mut dots: Vec<Span> = vec![Span::raw(" ")];
    for i in 0..total {
        let color = if i < idx {
            colors::GREEN
        } else if i == idx {
            colors::ACCENT
        } else {
            colors::DIM
        };
        dots.push(Span::styled("●", Style::default().fg(color)));
    }
    dots.push(Span::raw(" "));
    let dots_p = Paragraph::new(Line::from(dots))
        .alignment(Alignment::Right)
        .block(Block::default().borders(Borders::BOTTOM).border_style(colors::border()));
    f.render_widget(dots_p, chunks[1]);
}

fn render_step_name(f: &mut Frame, state: &OnboardingState, area: Rect) {
    // Build breadcrumb showing only 5 steps centered around current position
    let steps = [
        OnboardingStep::Welcome, OnboardingStep::SetupMode, OnboardingStep::QuickStart,
        OnboardingStep::Provider, OnboardingStep::Auth, OnboardingStep::Model,
        OnboardingStep::Channels, OnboardingStep::ChanConfig, OnboardingStep::SignalPair, OnboardingStep::WhatsAppPair, OnboardingStep::Gateway,
        OnboardingStep::Agent, OnboardingStep::Workspace, OnboardingStep::Security,
        OnboardingStep::Voice, OnboardingStep::Images, OnboardingStep::Orchestration,
        OnboardingStep::Memory, OnboardingStep::Skills, OnboardingStep::Complete,
    ];
    
    let current_idx = state.step.index();
    let total = steps.len();
    
    // Calculate window: show 2 before, current, 2 after when possible
    let window_size = 5;
    let half_window = window_size / 2;
    
    let start = if current_idx <= half_window {
        0
    } else if current_idx + half_window >= total {
        total.saturating_sub(window_size)
    } else {
        current_idx - half_window
    };
    
    let end = (start + window_size).min(total);
    
    let mut spans = vec![Span::raw(" ")];
    
    // Show "..." if not starting from beginning
    if start > 0 {
        spans.push(Span::styled("... ", Style::default().fg(colors::DIM)));
    }
    
    // Show the window of steps with colored pipe separators
    for i in start..end {
        let step = &steps[i];
        let is_current = step == &state.step;
        let style = if is_current {
            Style::default().fg(colors::RED).add_modifier(Modifier::BOLD)
        } else if step.index() < state.step.index() {
            Style::default().fg(colors::GREEN)
        } else {
            Style::default().fg(colors::DIM)
        };

        spans.push(Span::styled(step.short(), style));

        // Add colored pipe separator — red between done/current, dim for future
        if i < end - 1 {
            let pipe_color = if i < current_idx {
                colors::GREEN
            } else if i == current_idx {
                colors::ACCENT
            } else {
                colors::BORDER
            };
            spans.push(Span::styled(" ┃ ", Style::default().fg(pipe_color)));
        }
    }
    
    // Show "..." if not ending at the last step
    if end < total {
        spans.push(Span::styled(" ...", Style::default().fg(colors::DIM)));
    }
    
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_step_content(f: &mut Frame, state: &OnboardingState, area: Rect) {
    match state.step {
        OnboardingStep::Welcome       => render_welcome(f, state, area),
        OnboardingStep::SetupMode     => render_setup_mode(f, state, area),
        OnboardingStep::QuickStart    => render_quickstart(f, state, area),
        OnboardingStep::Provider      => render_provider(f, state, area),
        OnboardingStep::Auth          => render_auth(f, state, area),
        OnboardingStep::Model         => render_model(f, state, area),
        OnboardingStep::Fallback      => render_fallback(f, state, area),
        OnboardingStep::Channels      => render_channels(f, state, area),
        OnboardingStep::ChanConfig    => render_chan_config(f, state, area),
        OnboardingStep::SignalPair    => render_signal_pair(f, state, area),
        OnboardingStep::WhatsAppPair  => render_whatsapp_pair(f, state, area),
        OnboardingStep::Gateway       => render_gateway(f, state, area),
        OnboardingStep::Agent         => render_persona(f, state, area),
        OnboardingStep::Workspace     => render_workspace(f, state, area),
        OnboardingStep::Security      => render_security(f, state, area),
        OnboardingStep::Features      => render_features(f, state, area),
        OnboardingStep::Voice         => render_voice(f, state, area),
        OnboardingStep::Images        => render_images(f, state, area),
        OnboardingStep::Orchestration => render_orchestration(f, state, area),
        OnboardingStep::Memory        => render_memory(f, state, area),
        OnboardingStep::Skills        => render_skills(f, state, area),
        OnboardingStep::Complete      => render_launch(f, state, area),
    }
}

fn render_bottom_help(f: &mut Frame, state: &OnboardingState, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(40)])
        .split(area);

    // Left: key count info
    let keys_entered = if !state.api_key.is_empty() { 1 } else { 0 };
    let left = Paragraph::new(Line::from(vec![
        Span::styled(format!(" Keys entered: {} ", keys_entered), Style::default().fg(colors::GREEN)),
        Span::styled("│ ", Style::default().fg(colors::BORDER)),
        Span::styled(format!("Step {} of {}", state.step.index() + 1, OnboardingStep::total()), Style::default().fg(colors::DIM)),
    ]));
    f.render_widget(left, chunks[0]);

    // Right: navigation hints
    let right = Paragraph::new(Line::from(vec![
        Span::styled("Esc ", Style::default().fg(colors::ACCENT)),
        Span::styled("Back ", Style::default().fg(colors::DIM)),
        Span::styled("│ ", Style::default().fg(colors::BORDER)),
        Span::styled("Enter ", Style::default().fg(colors::ACCENT)),
        Span::styled("Continue ", Style::default().fg(colors::DIM)),
        Span::styled("│ ", Style::default().fg(colors::BORDER)),
        Span::styled("Q ", Style::default().fg(colors::ACCENT)),
        Span::styled("Quit", Style::default().fg(colors::DIM)),
    ])).alignment(Alignment::Right);
    f.render_widget(right, chunks[1]);
}

// ── Welcome ────────────────────────────────────────────────────────────────────

fn render_welcome(f: &mut Frame, state: &OnboardingState, area: Rect) {
    let _tick = state.tick; // used for animation timing

    // Full pixel-art logo from JSX prototype (6 lines, box-drawing chars)
    let logo = [
        ("██████╗ ███████╗██╗   ██╗███████╗", colors::ACCENT),
        ("╚════██╗██╔════╝██║   ██║██╔════╝", colors::ACCENT),
        ("  ███╔═╝█████╗  ██║   ██║███████╗", colors::ACCENT_BRIGHT),
        (" ██╔══╝ ██╔══╝  ██║   ██║╚════██║", colors::ACCENT_BRIGHT),
        ("███████╗███████╗╚██████╔╝███████║", colors::ACCENT_DIM),
        ("╚══════╝╚══════╝ ╚═════╝ ╚══════╝", colors::ACCENT_DIM),
    ];

    let mut lines = vec![Line::from(""), Line::from("")];

    for (text, color) in &logo {
        lines.push(Line::from(Span::styled(
            format!("  {}", text),
            Style::default().fg(*color).add_modifier(Modifier::BOLD),
        )));
    }

    lines.extend(vec![
        Line::from(""),
        Line::from(Span::styled(
            "  AUTONOMOUS COGNITIVE PLATFORM",
            Style::default().fg(colors::DIM),
        )),
        Line::from(vec![
            Span::styled(concat!("  ", env!("CARGO_PKG_VERSION")), Style::default().fg(colors::ACCENT_DIM).add_modifier(Modifier::BOLD)),
            Span::styled(" │ ", Style::default().fg(colors::BORDER)),
            Span::styled(format!("{} LLM providers", PROVIDERS.len()), Style::default().fg(colors::DIM)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  Welcome to Zeus. This wizard will configure your",
            Style::default().fg(colors::TEXT_BRIGHT),
        )),
        Line::from(Span::styled(
            "  cognitive agent platform — models, channels, memory,",
            Style::default().fg(colors::TEXT),
        )),
        Line::from(Span::styled(
            "  security, and workspace. Takes about 2 minutes.",
            Style::default().fg(colors::TEXT),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("  [Y] ", Style::default().fg(colors::ACCENT).add_modifier(Modifier::BOLD)),
            Span::styled("Continue    ", Style::default().fg(colors::TEXT)),
            Span::styled("[N] ", Style::default().fg(colors::DIM)),
            Span::styled("Exit", Style::default().fg(colors::DIM)),
        ]),
    ]);

    let p = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded)
            .border_style(colors::border_active())
            .title(" ⬡ Welcome "));
    f.render_widget(p, area);
}

// ── Provider ───────────────────────────────────────────────────────────────────

fn render_provider(f: &mut Frame, state: &OnboardingState, area: Rect) {
    // Provider tag colors, indexed by position in PROVIDERS. Keep this
    // aligned with the PROVIDERS slice in `onboarding/mod.rs` — if a
    // provider is inserted or reordered, update both in lockstep.
    let tag_colors: &[ratatui::style::Color] = &[
        colors::ACCENT,                           // 0 Anthropic  — red
        colors::GREEN,                            // 1 OpenAI     — green
        ratatui::style::Color::Rgb(59, 130, 246), // 2 Google     — blue
        ratatui::style::Color::Rgb(6, 182, 212),  // 3 Ollama     — cyan
        ratatui::style::Color::Rgb(59, 130, 246), // 4 Gemini CLI — blue
        colors::PURPLE,                           // 5 Kimi       — purple
        colors::YELLOW,                           // 6 GLM        — yellow
        ratatui::style::Color::Rgb(255, 104, 66), // 7 Qwen       — orange
    ];

    // Layout: title area + grid area + help
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3), // title + subtitle
            Constraint::Min(0),   // grid
            Constraint::Length(1), // nav hint
        ])
        .split(area);

    // Title
    let title_lines = vec![
        Line::from(Span::styled("  SELECT LLM PROVIDER", Style::default().fg(colors::ACCENT).add_modifier(Modifier::BOLD))),
        Line::from(Span::styled("  Choose your primary inference provider", Style::default().fg(colors::DIM))),
    ];
    f.render_widget(Paragraph::new(title_lines), chunks[0]);

    // Card geometry — a single card on-screen is:
    //
    //   [gap][marker ][border][<card_inner chars>][border]
    //     2      2         1           N             1     = card_total
    //
    // `marker` is either "  " (unselected) or "▸ " (selected) so both
    // states consume the same column budget. `card_inner` is the text
    // width between the two `│` borders AND the width of the `─` run
    // between the `╭`/`╮` top corners — previously these were
    // desynchronised (14 vs 13) which left a one-char right overhang on
    // every card.
    let card_inner: usize = 14;
    let card_total: usize = 2 /* gap */ + 2 /* marker */ + 1 /* border */
                          + card_inner + 1 /* border */;

    // Derive the column count from the available width so the grid
    // doesn't overflow narrow terminals. Clamp to [1, 6] — below 1 is
    // meaningless, above 6 just wastes horizontal space with the
    // current 8-provider roster.
    let usable_width = chunks[1].width as usize;
    let cols = usable_width
        .checked_div(card_total)
        .unwrap_or(1)
        .clamp(1, 6);
    let rows = PROVIDERS.len().div_ceil(cols);

    // Build grid lines manually — one row of providers renders as five
    // consecutive `Line`s: top border, name, tag, detection status,
    // bottom border. Rows are separated by a single blank line.
    let mut grid_lines: Vec<Line> = vec![Line::from("")];

    for row in 0..rows {
        let row_start = row * cols;
        let row_end = (row_start + cols).min(PROVIDERS.len());

        // ── Top borders ────────────────────────────────────────────
        let mut top_spans = vec![Span::raw("  ")];
        for i in row_start..row_end {
            let selected = i == state.selected_provider;
            let border_color = if selected { colors::BORDER_BRIGHT } else { colors::BORDER };
            let marker = if selected { "▸ " } else { "  " };
            top_spans.push(Span::styled(marker, Style::default().fg(colors::ACCENT)));
            top_spans.push(Span::styled("╭", Style::default().fg(border_color)));
            top_spans.push(Span::styled("─".repeat(card_inner), Style::default().fg(border_color)));
            top_spans.push(Span::styled("╮", Style::default().fg(border_color)));
        }
        grid_lines.push(Line::from(top_spans));

        // ── Name row ───────────────────────────────────────────────
        // Body cells pad to `card_inner` chars so the right `│` lines
        // up with the `╮` / `╯` above and below. Previously this was
        // `card_inner - 1`, which left a 1-column overhang.
        let mut name_spans = vec![Span::raw("  ")];
        for i in row_start..row_end {
            let p = &PROVIDERS[i];
            let selected = i == state.selected_provider;
            let border_color = if selected { colors::BORDER_BRIGHT } else { colors::BORDER };
            let name_style = if selected {
                Style::default().fg(colors::TEXT_BRIGHT).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(colors::TEXT)
            };
            name_spans.push(Span::raw("  "));
            name_spans.push(Span::styled("│", Style::default().fg(border_color)));
            name_spans.push(Span::styled(format!(" {:<width$}", p.name, width = card_inner - 1), name_style));
            name_spans.push(Span::styled("│", Style::default().fg(border_color)));
        }
        grid_lines.push(Line::from(name_spans));

        // ── Tag row (coloured) ─────────────────────────────────────
        let mut tag_spans = vec![Span::raw("  ")];
        for i in row_start..row_end {
            let p = &PROVIDERS[i];
            let border_color = if i == state.selected_provider { colors::BORDER_BRIGHT } else { colors::BORDER };
            let tc = tag_colors.get(i).copied().unwrap_or(colors::DIM);
            tag_spans.push(Span::raw("  "));
            tag_spans.push(Span::styled("│", Style::default().fg(border_color)));
            tag_spans.push(Span::styled(format!(" {:<width$}", p.tag, width = card_inner - 1), Style::default().fg(tc)));
            tag_spans.push(Span::styled("│", Style::default().fg(border_color)));
        }
        grid_lines.push(Line::from(tag_spans));

        // ── Detection status row ───────────────────────────────────
        let mut detect_spans = vec![Span::raw("  ")];
        for i in row_start..row_end {
            let detected = state.providers_with_detection.get(i).copied().unwrap_or(false);
            let border_color = if i == state.selected_provider { colors::BORDER_BRIGHT } else { colors::BORDER };
            let (dot, dot_color) = if detected {
                ("● ready", colors::GREEN)
            } else {
                ("○ no key", colors::DIM)
            };
            detect_spans.push(Span::raw("  "));
            detect_spans.push(Span::styled("│", Style::default().fg(border_color)));
            detect_spans.push(Span::styled(format!(" {:<width$}", dot, width = card_inner - 1), Style::default().fg(dot_color)));
            detect_spans.push(Span::styled("│", Style::default().fg(border_color)));
        }
        grid_lines.push(Line::from(detect_spans));

        // ── Bottom borders ─────────────────────────────────────────
        let mut bot_spans = vec![Span::raw("  ")];
        for i in row_start..row_end {
            let border_color = if i == state.selected_provider { colors::BORDER_BRIGHT } else { colors::BORDER };
            bot_spans.push(Span::raw("  "));
            bot_spans.push(Span::styled("╰", Style::default().fg(border_color)));
            bot_spans.push(Span::styled("─".repeat(card_inner), Style::default().fg(border_color)));
            bot_spans.push(Span::styled("╯", Style::default().fg(border_color)));
        }
        grid_lines.push(Line::from(bot_spans));
        grid_lines.push(Line::from(""));
    }

    f.render_widget(Paragraph::new(grid_lines), chunks[1]);

    // Nav hint
    let nav = Paragraph::new(Line::from(vec![
        Span::styled("  ←/→ ", Style::default().fg(colors::ACCENT).add_modifier(Modifier::BOLD)),
        Span::styled("Browse  ", Style::default().fg(colors::DIM)),
        Span::styled("Enter ", Style::default().fg(colors::ACCENT).add_modifier(Modifier::BOLD)),
        Span::styled("Select", Style::default().fg(colors::DIM)),
    ]));
    f.render_widget(nav, chunks[2]);
}

// ── Model ──────────────────────────────────────────────────────────────────────

fn render_model(f: &mut Frame, state: &OnboardingState, area: Rect) {
    if state.models_fetching {
        let lines = vec![
            Line::from(""),
            Line::from(Span::styled("  Fetching models from provider...", Style::default().fg(colors::ACCENT))),
            Line::from(""),
        ];
        let p = Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded)
                .border_style(colors::border_active())
                .title(" ⬡ Select Model "));
        f.render_widget(p, area);
        return;
    }

    let models = state.current_models();
    if let Some(ref err) = state.models_fetch_error {
        let lines = vec![
            Line::from(""),
            Line::from(Span::styled(format!("  Failed to fetch: {}", err), Style::default().fg(colors::RED))),
            Line::from(Span::styled("  Using default model list", Style::default().fg(colors::DIM))),
            Line::from(""),
        ];
        let p = Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded)
                .border_style(colors::border_active())
                .title(" ⬡ Select Model "));
        f.render_widget(p, area);
        // Fall through to show defaults below
    }

    let items: Vec<ListItem> = models.iter().enumerate().map(|(i, m)| {
        let selected = i == state.selected_model;
        ListItem::new(Line::from(vec![
            Span::styled(if selected { "▶ " } else { "  " }, Style::default().fg(colors::RED)),
            Span::styled(
                m.to_string(),
                if selected {
                    Style::default().fg(colors::TEXT_BRIGHT).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(colors::TEXT)
                },
            ),
        ]))
    }).collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded)
            .border_style(colors::border_active())
            .title(" ⬡ Select Model  ↑↓=Navigate  Enter=Confirm "));
    f.render_widget(list, area);
}

// ── Persona ────────────────────────────────────────────────────────────────────

fn render_persona(f: &mut Frame, state: &OnboardingState, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3), // agent name
            Constraint::Length(3), // user name
            Constraint::Length(3), // user role
            Constraint::Length(3), // user org
            Constraint::Length(6), // personality style
            Constraint::Min(0),    // persona list
        ])
        .split(area);
    // chunks[0..5] are used below — chunks[5] is the persona list

    // Agent name input (sel==0)
    let agent_cursor = if state.sel == 0 { "█" } else { "" };
    let name_display = format!("{}{}", state.agent_name, agent_cursor);
    let name_p = Paragraph::new(name_display)
        .style(Style::default().fg(colors::TEXT_BRIGHT))
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded)
            .border_style(if state.sel == 0 { Style::default().fg(colors::RED) } else { colors::border() })
            .title(" Agent Name "));
    f.render_widget(name_p, chunks[0]);

    // User name input (sel==1) — "Who am I serving?"
    let user_cursor = if state.sel == 1 { "█" } else { "" };
    let user_display = format!("{}{}", state.user_name, user_cursor);
    let user_p = Paragraph::new(user_display)
        .style(Style::default().fg(colors::TEXT_BRIGHT))
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded)
            .border_style(if state.sel == 1 { Style::default().fg(colors::RED) } else { colors::border() })
            .title(" Who am I serving? (Your name) "));
    f.render_widget(user_p, chunks[1]);

    // User role input (sel==2)
    let role_cursor = if state.sel == 2 { "█" } else { "" };
    let role_display = format!("{}{}", state.user_role, role_cursor);
    let role_p = Paragraph::new(role_display)
        .style(Style::default().fg(colors::TEXT_BRIGHT))
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded)
            .border_style(if state.sel == 2 { Style::default().fg(colors::RED) } else { colors::border() })
            .title(" Your Role (e.g. Founder, Engineer) "));
    f.render_widget(role_p, chunks[2]);

    // User org input (sel==3)
    let org_cursor = if state.sel == 3 { "█" } else { "" };
    let org_display = format!("{}{}", state.user_org, org_cursor);
    let org_p = Paragraph::new(org_display)
        .style(Style::default().fg(colors::TEXT_BRIGHT))
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded)
            .border_style(if state.sel == 3 { Style::default().fg(colors::RED) } else { colors::border() })
            .title(" Your Organization "));
    f.render_widget(org_p, chunks[3]);

    // Personality style selector (sel==4)
    let style_names = ["Professional", "Collaborative", "Minimal", "Autonomous"];
    let style_descs = ["Formal, precise, business-ready", "Warm, team-oriented, adaptive", "Terse, no filler, signal only", "Self-directed, proactive, low-interrupt"];
    let style_items: Vec<ListItem> = style_names.iter().enumerate().map(|(i, name)| {
        let selected = i == state.personality_style;
        ListItem::new(Line::from(vec![
            Span::styled(if selected { "  ▶ " } else { "    " }, Style::default().fg(colors::RED)),
            Span::styled(
                format!("{:<16}", name),
                if selected { Style::default().fg(colors::TEXT_BRIGHT).add_modifier(Modifier::BOLD) } else { Style::default().fg(colors::TEXT) },
            ),
            Span::styled(style_descs[i], Style::default().fg(colors::DIM)),
        ]))
    }).collect();
    let style_list = List::new(style_items)
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded)
            .border_style(if state.sel == 4 { Style::default().fg(colors::RED) } else { colors::border() })
            .title(" Communication Style  ↑↓=Select "));
    f.render_widget(style_list, chunks[4]);

    // Persona categories list (sel==5)
    let mut items: Vec<ListItem> = vec![];
    for (cat_idx, cat) in state.personas.iter().enumerate() {
        items.push(ListItem::new(Line::from(
            Span::styled(format!("  {}", cat.cat), Style::default().fg(colors::YELLOW).add_modifier(Modifier::BOLD))
        )));
        for (item_idx, item) in cat.items.iter().enumerate() {
            let selected = cat_idx == state.persona_cat && item_idx == state.persona_item;
            items.push(ListItem::new(Line::from(vec![
                Span::styled(if selected { "  ▶ " } else { "    " }, Style::default().fg(colors::RED)),
                Span::styled(
                    item.to_string(),
                    if selected {
                        Style::default().fg(colors::TEXT_BRIGHT).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(colors::TEXT)
                    },
                ),
            ])));
        }
    }

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded)
            .border_style(if state.sel >= 5 { Style::default().fg(colors::RED) } else { colors::border() })
            .title(" Select Persona  ↑↓=Navigate  Tab=Switch Field "));
    f.render_widget(list, chunks[5]);
}

// ── Fallback LLMs ──────────────────────────────────────────────────────────────

fn render_fallback(f: &mut Frame, state: &OnboardingState, area: Rect) {
    let primary_idx = state.selected_provider;
    let primary = PROVIDERS.get(primary_idx);

    let mut lines: Vec<Line> = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Select a backup LLM for automatic failover (optional)",
            Style::default().fg(colors::DIM),
        )),
        Line::from(""),
    ];

    // Primary provider row — always shown at top, marked, not selectable
    if let Some(p) = primary {
        let primary_configured = state.providers_with_detection.get(primary_idx).copied().unwrap_or(false);
        let check_style = if primary_configured {
            Style::default().fg(colors::GREEN).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(colors::YELLOW)
        };
        lines.push(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled("[✓]", check_style),
            Span::raw(" "),
            Span::styled(
                format!("{:<14}", p.name),
                Style::default().fg(colors::GREEN).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" {}", p.tag),
                Style::default().fg(colors::DIM),
            ),
            Span::styled("  ← primary", Style::default().fg(colors::ACCENT_DIM)),
        ]));
    }

    lines.push(Line::from(Span::styled(
        "  ─────────────────────────────────────────",
        Style::default().fg(colors::BORDER_BRIGHT),
    )));

    // Non-primary providers — selectable
    let non_primary: Vec<(usize, &Provider)> = PROVIDERS
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != primary_idx)
        .collect();

    if non_primary.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No other providers available",
            Style::default().fg(colors::DIM),
        )));
    }

    for (list_idx, (prov_idx, p)) in non_primary.iter().enumerate() {
        let focused = state.fallback_focus == list_idx;
        let configured = state.providers_with_detection.get(*prov_idx).copied().unwrap_or(false);
        let toggled = state.fallback_models.iter().any(|m| m.starts_with(&format!("{}/", p.provider_id)));

        let cursor = if focused { "▶ " } else { "  " };
        let cursor_style = Style::default().fg(colors::RED);

        let (check, check_style) = if toggled {
            ("[✓]", Style::default().fg(colors::GREEN).add_modifier(Modifier::BOLD))
        } else {
            ("[ ]", Style::default().fg(colors::DIM))
        };

        let name_style = if focused {
            Style::default().fg(colors::TEXT_BRIGHT).add_modifier(Modifier::BOLD)
        } else if configured {
            Style::default().fg(colors::TEXT)
        } else {
            Style::default().fg(colors::DIM)
        };

        let mut row = vec![
            Span::styled(cursor, cursor_style),
            Span::styled(check, check_style),
            Span::raw(" "),
            Span::styled(format!("{:<14}", p.name), name_style),
            Span::styled(
                format!(" {}", p.tag),
                Style::default().fg(if focused { colors::DIM } else { colors::BORDER_BRIGHT }),
            ),
        ];

        if !configured && p.provider_id != "ollama" && p.provider_id != "google-gemini-cli" {
            row.push(Span::styled("  (no key)", Style::default().fg(colors::DIM)));
        }
        if focused && toggled {
            if !configured && p.provider_id != "ollama" {
                row.push(Span::styled("  → (configure key first)", Style::default().fg(colors::YELLOW)));
            } else if let Some(model_str) = state.fallback_models.iter().find(|m| m.starts_with(&format!("{}/", p.provider_id))) {
                row.push(Span::styled(format!("  → {}", model_str), Style::default().fg(colors::ACCENT_DIM)));
            }
        }

        lines.push(Line::from(row));
    }

    lines.push(Line::from(""));
    if state.fallback_models.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No backup selected — Enter to skip",
            Style::default().fg(colors::DIM),
        )));
    } else {
        let chain: Vec<&str> = state.fallback_models.iter().map(String::as_str).collect();
        lines.push(Line::from(vec![
            Span::styled("  Failover chain: ", Style::default().fg(colors::ACCENT_DIM)),
            Span::styled(chain.join(" → "), Style::default().fg(colors::TEXT)),
        ]));
    }

    let p = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded)
            .border_style(colors::border_active())
            .title(" ⬡ Backup LLM  ↑↓=Navigate  Space=Toggle  Enter=Continue "));
    f.render_widget(p, area);
}

// ── Channels ───────────────────────────────────────────────────────────────────

fn render_channels(f: &mut Frame, state: &OnboardingState, area: Rect) {
    let items: Vec<ListItem> = CHANNELS.iter().enumerate().map(|(i, ch)| {
        let toggled = state.channel_toggled.contains(&i);
        let selected = i == state.sel;
        let coming_soon = ch.coming_soon;
        // Coming-soon channels: greyed-out check, no toggle (non-selectable visual).
        let (check, check_style) = if coming_soon {
            ("[ ]", Style::default().fg(colors::DIM))
        } else if toggled {
            ("[✓]", Style::default().fg(colors::GREEN))
        } else {
            ("[ ]", Style::default().fg(colors::DIM))
        };
        let name_style = if coming_soon {
            Style::default().fg(colors::DIM)
        } else if selected {
            Style::default().fg(colors::TEXT_BRIGHT).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(colors::TEXT)
        };
        let desc_text = if coming_soon {
            format!("{} (coming soon)", ch.desc)
        } else {
            ch.desc.to_string()
        };
        ListItem::new(Line::from(vec![
            Span::styled(if selected { "▶ " } else { "  " }, Style::default().fg(colors::RED)),
            Span::styled(check, check_style),
            Span::raw(" "),
            Span::styled(format!("{:<12}", ch.name), name_style),
            Span::styled(desc_text, Style::default().fg(colors::DIM)),
        ]))
    }).collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded)
            .border_style(colors::border_active())
            .title(" ⬡ Channels  ↑↓=Navigate  Space=Toggle  Enter=Continue "));
    f.render_widget(list, area);
}

// ── Security ───────────────────────────────────────────────────────────────────

fn render_security(f: &mut Frame, state: &OnboardingState, area: Rect) {
    let items: Vec<ListItem> = SECURITY_LEVELS.iter().enumerate().map(|(i, lvl)| {
        let selected = i == state.security_level;
        let marker = if selected { "▶ " } else { "  " };
        
        // Create bordered card like JSX template
        let security_content = vec![
            Line::from(vec![
                Span::styled("╭─", Style::default().fg(colors::BORDER)),
                Span::styled("─".repeat(58), Style::default().fg(colors::BORDER)),
                Span::styled("─╮", Style::default().fg(colors::BORDER)),
            ]),
            Line::from(vec![
                Span::styled("│ ", Style::default().fg(colors::BORDER)),
                Span::styled(marker, Style::default().fg(colors::RED)),
                Span::styled(
                    lvl.name,
                    if selected {
                        Style::default().fg(colors::TEXT_BRIGHT).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(colors::TEXT)
                    },
                ),
                Span::styled(" │", Style::default().fg(colors::BORDER)),
            ]),
            Line::from(vec![
                Span::styled("│   ", Style::default().fg(colors::BORDER)),
                Span::styled(lvl.desc, Style::default().fg(colors::DIM)),
                Span::styled(" │", Style::default().fg(colors::BORDER)),
            ]),
            Line::from(vec![
                Span::styled("╰─", Style::default().fg(colors::BORDER)),
                Span::styled("─".repeat(58), Style::default().fg(colors::BORDER)),
                Span::styled("─╯", Style::default().fg(colors::BORDER)),
            ]),
            Line::from(""),
        ];
        
        ListItem::new(security_content)
    }).collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded)
            .border_style(colors::border_active())
            .title(" ⬡ Security Level  ↑↓=Navigate  Enter=Confirm "));
    f.render_widget(list, area);
}

// ── Skills ─────────────────────────────────────────────────────────────────────

fn render_skills(f: &mut Frame, state: &OnboardingState, area: Rect) {
    let mut items: Vec<ListItem> = vec![];
    let mut flat_idx = 0usize;
    for (ci, cat) in state.skills.iter().enumerate() {
        items.push(ListItem::new(Line::from(
            Span::styled(format!("  {}", cat.cat), Style::default().fg(colors::YELLOW).add_modifier(Modifier::BOLD))
        )));
        for (si, sk) in cat.items.iter().enumerate() {
            let enabled = state.skill_selected.get(&(ci, si)).copied().unwrap_or(false);
            let selected = flat_idx == state.sel;
            let (check, check_style) = if enabled {
                ("[✓]", Style::default().fg(colors::GREEN))
            } else {
                ("[ ]", Style::default().fg(colors::DIM))
            };
            let marker = if selected { "▶ " } else { "  " };
            items.push(ListItem::new(Line::from(vec![
                Span::styled(marker, Style::default().fg(colors::ACCENT)),
                Span::styled(check, check_style),
                Span::raw(" "),
                Span::styled(
                    format!("{:<24}", sk.name),
                    if selected { Style::default().fg(colors::TEXT_BRIGHT).add_modifier(Modifier::BOLD) } else { Style::default().fg(colors::TEXT_BRIGHT) },
                ),
                Span::styled(sk.desc.as_str(), Style::default().fg(colors::DIM)),
            ])));
            flat_idx += 1;
        }
    }

    // Use a scrollable paragraph instead of List for long skill lists
    let mut skill_lines: Vec<Line> = vec![];
    let max_desc = (area.width as usize).saturating_sub(35); // truncate descriptions
    let mut line_idx = 0usize;
    let mut sel_line = 0usize;
    let mut flat = 0usize;
    for (ci, cat) in state.skills.iter().enumerate() {
        skill_lines.push(Line::from(
            Span::styled(format!("  {}", cat.cat), Style::default().fg(colors::YELLOW).add_modifier(Modifier::BOLD))
        ));
        line_idx += 1;
        for (si, sk) in cat.items.iter().enumerate() {
            let enabled = state.skill_selected.get(&(ci, si)).copied().unwrap_or(false);
            let selected = flat == state.sel;
            if selected { sel_line = line_idx; }
            let (check, check_style) = if enabled {
                ("[✓]", Style::default().fg(colors::GREEN))
            } else {
                ("[ ]", Style::default().fg(colors::DIM))
            };
            let marker = if selected { "▶ " } else { "  " };
            let desc_truncated: String = sk.desc.chars().take(max_desc).collect();
            skill_lines.push(Line::from(vec![
                Span::styled(marker, Style::default().fg(colors::ACCENT)),
                Span::styled(check, check_style),
                Span::raw(" "),
                Span::styled(
                    format!("{:<22}", sk.name),
                    if selected { Style::default().fg(colors::TEXT_BRIGHT).add_modifier(Modifier::BOLD) } else { Style::default().fg(colors::TEXT_BRIGHT) },
                ),
                Span::styled(desc_truncated, Style::default().fg(colors::DIM)),
            ]));
            flat += 1;
            line_idx += 1;
        }
    }

    let scroll = if sel_line > 5 { (sel_line - 5) as u16 } else { 0 };
    let p = Paragraph::new(skill_lines)
        .scroll((scroll, 0))
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded)
            .border_style(colors::border_active())
            .title(" ⬡ Skills  ↑↓=Navigate  Space=Toggle  Enter=Install "));
    f.render_widget(p, area);
}

// ── Generic ConfigStep renderer ────────────────────────────────────────────────

/// Renders a config-step form with title, subtitle, and labeled fields.
/// Each field is a tuple of (label, value, is_active).
/// Active fields show a cursor block and accent border.
fn render_config_step(
    f: &mut Frame,
    title: &str,
    subtitle: &str,
    fields: &[(&str, &str, bool)],
    area: Rect,
) {
    let max_label = fields.iter().map(|(l, _, _)| l.len()).max().unwrap_or(0);

    let mut lines: Vec<Line> = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  {}", title),
            Style::default().fg(colors::ACCENT).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            format!("  {}", subtitle),
            Style::default().fg(colors::DIM),
        )),
        Line::from(""),
    ];

    for (label, value, active) in fields {
        let marker = if *active { "\u{25b8} " } else { "  " };
        let marker_style = if *active {
            Style::default().fg(colors::ACCENT)
        } else {
            Style::default().fg(colors::DIM)
        };
        let label_style = if *active {
            Style::default().fg(colors::TEXT_BRIGHT).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(colors::TEXT)
        };
        let val_style = if *active {
            Style::default().fg(colors::TEXT_BRIGHT)
        } else {
            Style::default().fg(colors::DIM)
        };
        let cursor = if *active { "\u{2588}" } else { "" };

        lines.push(Line::from(vec![
            Span::styled(format!("  {}", marker), marker_style),
            Span::styled(format!("{:<width$}", label, width = max_label), label_style),
            Span::styled(" \u{2502} ", Style::default().fg(colors::BORDER)),
            Span::styled(value.to_string(), val_style),
            Span::styled(cursor, Style::default().fg(colors::ACCENT)),
        ]));
    }

    lines.push(Line::from(Span::styled(
        "    Tab=Next field  Enter=Continue",
        Style::default().fg(colors::DIM),
    )));

    let p = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded)
            .border_style(colors::border_active())
            .title(format!(" \u{2b21} {} ", title)));
    f.render_widget(p, area);
}

// ── SetupMode (step 2) ────────────────────────────────────────────────────────

fn render_setup_mode(f: &mut Frame, state: &OnboardingState, area: Rect) {
    let options = [
        ("QuickStart", "Sensible defaults, get running in 30 seconds"),
        ("Manual",     "Step through every setting one by one"),
        ("Skip",       "Use existing config, skip onboarding"),
    ];

    let mut lines: Vec<Line> = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Setup Mode",
            Style::default().fg(colors::ACCENT).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "  Choose your configuration style",
            Style::default().fg(colors::DIM),
        )),
        Line::from(""),
    ];

    for (i, (name, desc)) in options.iter().enumerate() {
        let selected = i == state.setup_mode;
        let marker = if selected { "\u{25b8} " } else { "  " };
        lines.push(Line::from(vec![
            Span::styled(
                format!("    {}", marker),
                Style::default().fg(colors::ACCENT),
            ),
            Span::styled(
                format!("{:<12}", name),
                if selected {
                    Style::default().fg(colors::TEXT_BRIGHT).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(colors::TEXT)
                },
            ),
            Span::styled(
                format!("  {}", desc),
                Style::default().fg(colors::DIM),
            ),
        ]));
        lines.push(Line::from(""));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "    \u{2191}/\u{2193}=Navigate  Enter=Select",
        Style::default().fg(colors::DIM),
    )));

    let p = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded)
            .border_style(colors::border_active())
            .title(" \u{2b21} Setup Mode "));
    f.render_widget(p, area);
}

// ── QuickStart (step 3) ────────────────────────────────────────────────────────

fn render_quickstart(f: &mut Frame, state: &OnboardingState, area: Rect) {
    let labels = ["Gateway Port", "Gateway Host", "Workspace", "Sessions", "Max Iterations"];
    let fields: Vec<(&str, &str, bool)> = labels
        .iter()
        .enumerate()
        .map(|(i, label)| {
            let val = state.quickstart_fields.get(i).map(|s| s.as_str()).unwrap_or("");
            (*label, val, i == state.quickstart_focus)
        })
        .collect();

    render_config_step(f, "Quick Start", "Fast configuration — edit defaults or press Enter to accept", &fields, area);
}

// ── Auth (step 5) ──────────────────────────────────────────────────────────────

fn render_auth(f: &mut Frame, state: &OnboardingState, area: Rect) {
    let provider = PROVIDERS.get(state.selected_provider).unwrap_or(&PROVIDERS[0]);
    let detected = state.providers_with_detection.get(state.selected_provider).copied().unwrap_or(false);

    let mut lines: Vec<Line> = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Authentication",
            Style::default().fg(colors::ACCENT).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            format!("  Configure credentials for {}", provider.name),
            Style::default().fg(colors::DIM),
        )),
        Line::from(""),
    ];

    // Detection status
    if detected {
        lines.push(Line::from(vec![
            Span::styled("    \u{2713} ", Style::default().fg(colors::GREEN)),
            Span::styled(
                format!("{} detected in environment", provider.env_var),
                Style::default().fg(colors::GREEN),
            ),
        ]));
        lines.push(Line::from(""));
    }

    // CLI credential Y/N prompt — show instead of fields when detected
    if state.cli_cred_prompt {
        if let Some(ref cred) = state.cli_cred {
            lines.push(Line::from(vec![
                Span::styled("  Found existing ", Style::default().fg(colors::TEXT)),
                Span::styled(cred.provider_name.clone(), Style::default().fg(colors::ACCENT).add_modifier(Modifier::BOLD)),
                Span::styled(" credentials from ", Style::default().fg(colors::TEXT)),
                Span::styled(cred.source.clone(), Style::default().fg(colors::ACCENT)),
                Span::styled(". Use these?", Style::default().fg(colors::TEXT)),
            ]));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled("  Token: ", Style::default().fg(colors::DIM)),
                Span::styled(cred.masked(), Style::default().fg(colors::TEXT_BRIGHT)),
            ]));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled("  [", Style::default().fg(colors::DIM)),
                Span::styled("Y", Style::default().fg(colors::GREEN).add_modifier(Modifier::BOLD)),
                Span::styled("] Yes, use these   [", Style::default().fg(colors::DIM)),
                Span::styled("N", Style::default().fg(colors::RED).add_modifier(Modifier::BOLD)),
                Span::styled("] No, enter manually", Style::default().fg(colors::DIM)),
            ]));
        }
        let p = Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded)
                .border_style(colors::border_active())
                .title(format!(" \u{2b21} {} Auth ", provider.name)));
        f.render_widget(p, area);
        return;
    }

    // Ollama: show URL field instead of API key/OAuth
    if provider.provider_id == "ollama" {
        // Show the editable URL field — uses api_key field to store the URL
        let url_display = if state.api_key.is_empty() {
            std::env::var("OLLAMA_HOST").unwrap_or_else(|_| "http://localhost:11434".to_string())
        } else {
            state.api_key.clone()
        };
        lines.push(Line::from(vec![
            Span::styled("  \u{25b8} ", Style::default().fg(colors::ACCENT)),
            Span::styled(
                format!("{:<16}", "Ollama URL"),
                Style::default().fg(colors::TEXT_BRIGHT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" \u{2502} ", Style::default().fg(colors::BORDER)),
            Span::styled(&url_display, Style::default().fg(colors::TEXT_BRIGHT)),
            Span::styled("\u{2588}", Style::default().fg(colors::ACCENT)),
        ]));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Ollama runs locally — no API key needed.",
            Style::default().fg(colors::DIM),
        )));
        lines.push(Line::from(Span::styled(
            "  Edit the URL if Ollama is on a different host/port.",
            Style::default().fg(colors::DIM),
        )));
        lines.push(Line::from(Span::styled(
            "  Models will be fetched automatically on next step.",
            Style::default().fg(colors::DIM),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Type to edit URL  |  Enter=Continue  Esc=Back",
            Style::default().fg(colors::DIM),
        )));
        f.render_widget(Paragraph::new(lines), area);
        return;
    }

    // Device code providers — show explicit menu (API Key | Device Code) like OpenAI/Anthropic
    let supports_device_code = matches!(provider.provider_id, "qwen" | "minimax");
    if supports_device_code {
        let api_active = state.auth_mode == 0;
        let dc_active = state.auth_mode == 3;

        // Option row: API Key
        lines.push(Line::from(vec![
            Span::styled(if api_active { "  \u{25b8} " } else { "    " },
                if api_active { Style::default().fg(colors::ACCENT) } else { Style::default().fg(colors::DIM) }),
            Span::styled(
                format!("{:<16}", "API Key"),
                if api_active { Style::default().fg(colors::TEXT_BRIGHT).add_modifier(Modifier::BOLD) } else { Style::default().fg(colors::TEXT) },
            ),
            Span::styled(" \u{2502} ", Style::default().fg(colors::BORDER)),
            if api_active {
                let masked = if state.api_key.is_empty() { String::new() }
                    else if state.api_key.len() <= 4 { state.api_key.clone() }
                    else { format!("{}{}", "\u{2022}".repeat(state.api_key.len() - 4), &state.api_key[state.api_key.len()-4..]) };
                Span::styled(masked, Style::default().fg(colors::TEXT_BRIGHT))
            } else {
                Span::styled("Paste your API key", Style::default().fg(colors::DIM))
            },
            Span::styled(if api_active { "\u{2588}" } else { "" }, Style::default().fg(colors::ACCENT)),
        ]));

        // Option row: Device Code
        lines.push(Line::from(vec![
            Span::styled(if dc_active { "  \u{25b8} " } else { "    " },
                if dc_active { Style::default().fg(colors::ACCENT) } else { Style::default().fg(colors::DIM) }),
            Span::styled(
                format!("{:<16}", "Device Code"),
                if dc_active { Style::default().fg(colors::TEXT_BRIGHT).add_modifier(Modifier::BOLD) } else { Style::default().fg(colors::TEXT) },
            ),
            Span::styled(" \u{2502} ", Style::default().fg(colors::BORDER)),
            Span::styled("Sign in via browser — no API key needed", Style::default().fg(colors::DIM)),
        ]));

        // When Device Code is active, show the code/URL details below
        if dc_active {
            lines.push(Line::from(""));
            if state.device_code_user_code.is_empty() {
                lines.push(Line::from(Span::styled(
                    "  Requesting device code...",
                    Style::default().fg(colors::DIM),
                )));
            } else {
                lines.push(Line::from(Span::styled(
                    format!("  Visit:  {}", state.device_code_verification_url),
                    Style::default().fg(colors::TEXT_BRIGHT),
                )));
                lines.push(Line::from(""));
                lines.push(Line::from(vec![
                    Span::styled("  Code:   ", Style::default().fg(colors::TEXT)),
                    Span::styled(
                        state.device_code_user_code.clone(),
                        Style::default().fg(colors::ACCENT).add_modifier(Modifier::BOLD),
                    ),
                ]));
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    "  Open the URL in a browser, enter the code, then press Enter.",
                    Style::default().fg(colors::DIM),
                )));
            }
        }
        if let Some(ref err) = state.error {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!("  {}", err),
                Style::default().fg(colors::DIM),
            )));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Tab = switch mode  |  Enter = continue",
            Style::default().fg(colors::DIM),
        )));
        let p = Paragraph::new(lines).wrap(ratatui::widgets::Wrap { trim: false });
        f.render_widget(p, area);
        return;
    }

    // Show OAuth option for providers that support it
    let supports_oauth = matches!(provider.provider_id, "anthropic" | "openai" | "google" | "google-gemini-cli");

    // API Key field — mask all but last 4 chars
    let masked_api = if state.api_key.is_empty() {
        String::new()
    } else if state.api_key.len() <= 4 {
        state.api_key.clone()
    } else {
        let visible = &state.api_key[state.api_key.len() - 4..];
        format!("{}{}", "\u{2022}".repeat(state.api_key.len() - 4), visible)
    };

    let masked_oauth = if state.oauth_token.is_empty() {
        String::new()
    } else if state.oauth_token.len() <= 4 {
        state.oauth_token.clone()
    } else {
        let visible = &state.oauth_token[state.oauth_token.len() - 4..];
        format!("{}{}", "\u{2022}".repeat(state.oauth_token.len() - 4), visible)
    };

    let api_active = state.auth_mode == 0;
    let oauth_active = state.auth_mode == 1;
    let browser_active = state.auth_mode == 2;
    let is_gemini_cli = provider.provider_id == "google-gemini-cli";

    // API Key row
    lines.push(Line::from(vec![
        Span::styled(if api_active { "  \u{25b8} " } else { "    " },
            if api_active { Style::default().fg(colors::ACCENT) } else { Style::default().fg(colors::DIM) }),
        Span::styled(
            format!("{:<16}", "API Key"),
            if api_active { Style::default().fg(colors::TEXT_BRIGHT).add_modifier(Modifier::BOLD) } else { Style::default().fg(colors::TEXT) },
        ),
        Span::styled(" \u{2502} ", Style::default().fg(colors::BORDER)),
        Span::styled(&masked_api, if api_active { Style::default().fg(colors::TEXT_BRIGHT) } else { Style::default().fg(colors::DIM) }),
        Span::styled(if api_active { "\u{2588}" } else { "" }, Style::default().fg(colors::ACCENT)),
    ]));

    // OAuth Token row — Anthropic + OpenAI
    if supports_oauth {
        lines.push(Line::from(vec![
            Span::styled(if oauth_active { "  \u{25b8} " } else { "    " },
                if oauth_active { Style::default().fg(colors::ACCENT) } else { Style::default().fg(colors::DIM) }),
            Span::styled(
                format!("{:<16}", "OAuth Token"),
                if oauth_active { Style::default().fg(colors::TEXT_BRIGHT).add_modifier(Modifier::BOLD) } else { Style::default().fg(colors::TEXT) },
            ),
            Span::styled(" \u{2502} ", Style::default().fg(colors::BORDER)),
            Span::styled(&masked_oauth, if oauth_active { Style::default().fg(colors::TEXT_BRIGHT) } else { Style::default().fg(colors::DIM) }),
            Span::styled(if oauth_active { "\u{2588}" } else { "" }, Style::default().fg(colors::ACCENT)),
        ]));
    }

    // Login with Browser row — Google providers only
    if supports_oauth {
        lines.push(Line::from(vec![
            Span::styled(if browser_active { "  \u{25b8} " } else { "    " },
                if browser_active { Style::default().fg(colors::ACCENT) } else { Style::default().fg(colors::DIM) }),
            Span::styled(
                format!("{:<16}", "Login with Browser"),
                if browser_active { Style::default().fg(colors::GREEN).add_modifier(Modifier::BOLD) } else { Style::default().fg(colors::TEXT) },
            ),
            Span::styled(" \u{2502} ", Style::default().fg(colors::BORDER)),
            Span::styled(
                if browser_active { "Press Enter to open browser" } else { "" },
                Style::default().fg(colors::DIM),
            ),
        ]));
    }

    lines.push(Line::from(""));
    if supports_oauth {
        let hint = if is_gemini_cli {
            "    ↑↓ / Tab = select  |  Enter = confirm  |  Login with Browser recommended for Gemini CLI"
        } else {
            "    ↑↓ / Tab = select mode  |  Enter = confirm"
        };
        lines.push(Line::from(Span::styled(hint, Style::default().fg(colors::DIM))));
    } else {
        lines.push(Line::from(Span::styled(
            "    Paste your API key",
            Style::default().fg(colors::DIM),
        )));
    }

    // Show error if present
    if let Some(ref err) = state.error {
        lines.push(Line::from(Span::styled(
            format!("    \u{2717} {}", err),
            Style::default().fg(colors::RED),
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "    Enter=Continue  Esc=Back",
        Style::default().fg(colors::DIM),
    )));

    let p = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded)
            .border_style(colors::border_active())
            .title(format!(" \u{2b21} {} Auth ", provider.name)));
    f.render_widget(p, area);
}

// ── ChanConfig (step 8) ────────────────────────────────────────────────────────

fn render_chan_config(f: &mut Frame, state: &OnboardingState, area: Rect) {
    // Build field list from toggled channels
    // Must match CHANNELS order in mod.rs: Discord, Telegram, IRC, Signal
    let channel_field_defs: &[(&str, &[&str])] = &[
        ("Discord",    &["Bot Token", "Channel ID", "Guild ID", "Role IDs (comma-sep, optional)"]),
        ("Telegram",   &["Bot Token", "Chat ID"]),
        ("IRC",        &["Server", "Port", "Channels", "Nick"]),
        ("Signal",     &["signal-cli Path", "Phone Number", "HTTP Port"]),
        ("X/Twitter",  &["Bearer Token", "API Key", "API Secret", "Access Token", "Access Token Secret", "Client ID (OAuth 2.0)", "Client Secret (OAuth 2.0)"]),
        ("Pantheon",   &["Server (host:port)", "Channel Key", "Nick"]),
        ("WhatsApp",   &["Bridge URL", "Phone Number"]),
        ("Matrix",     &["Homeserver URL", "User ID", "Access Token", "Default Room"]),
        ("Slack",      &["Bot Token (xoxb-)", "App Token (xapp-)", "Default Channel"]),
        ("Email",      &["SMTP Host", "SMTP Port", "IMAP Host", "IMAP Port"]),
        ("MQTT",       &["Broker URL", "Topic", "Client ID"]),
        ("Mattermost", &["Server URL", "Token", "Team ID"]),
    ];

    let mut lines: Vec<Line> = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Channel Configuration",
            Style::default().fg(colors::ACCENT).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "  Set up credentials for each enabled channel",
            Style::default().fg(colors::DIM),
        )),
        Line::from(""),
    ];

    // Field indices must match the flat chan_config_fields layout
    // (all channels pre-allocated, not just toggled ones)
    // Must match channel_field_defs order: Discord(4), Telegram(2), IRC(4), Signal(3), X/Twitter(5), Pantheon(3), WhatsApp(2), Matrix(4), Slack(3), Email(4), MQTT(3), Mattermost(3)
    let field_counts: &[usize] = &[4, 2, 4, 3, 7, 3, 2, 4, 3, 4, 3, 3];
    let mut base_idx = 0usize;
    let mut active_line: usize = 0; // track which line the focused field lands on
    for (chan_idx, (chan_name, chan_fields)) in channel_field_defs.iter().enumerate() {
        if !state.channel_toggled.contains(&chan_idx) {
            base_idx += field_counts.get(chan_idx).copied().unwrap_or(0);
            continue;
        }
        if chan_fields.is_empty() {
            lines.push(Line::from(Span::styled(
                format!("    {} — no configuration needed", chan_name),
                Style::default().fg(colors::DIM),
            )));
            lines.push(Line::from(""));
            continue;
        }
        // Channel header
        lines.push(Line::from(Span::styled(
            format!("  {}", chan_name),
            Style::default().fg(colors::YELLOW).add_modifier(Modifier::BOLD),
        )));

        for (fi, field_label) in chan_fields.iter().enumerate() {
            let abs_idx = base_idx + fi;
            let val = state.chan_config_fields.get(abs_idx).map(|s| s.as_str()).unwrap_or("");
            let active = !state.bot_policy_focused && abs_idx == state.chan_config_focus;
            if active { active_line = lines.len(); }
            let marker = if active { "\u{25b8} " } else { "  " };
            let marker_style = if active { Style::default().fg(colors::ACCENT) } else { Style::default().fg(colors::DIM) };
            let label_style = if active {
                Style::default().fg(colors::TEXT_BRIGHT).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(colors::TEXT)
            };
            let val_style = if active { Style::default().fg(colors::TEXT_BRIGHT) } else { Style::default().fg(colors::DIM) };
            let cursor = if active { "\u{2588}" } else { "" };

            lines.push(Line::from(vec![
                Span::styled(format!("    {}", marker), marker_style),
                Span::styled(format!("{:<16}", field_label), label_style),
                Span::styled(" \u{2502} ", Style::default().fg(colors::BORDER)),
                Span::styled(val.to_string(), val_style),
                Span::styled(cursor, Style::default().fg(colors::ACCENT)),
            ]));
        }
        // For Signal: show prereq hint
        if *chan_name == "Signal" {
            lines.push(Line::from(Span::styled(
                "      Hint: signal-cli must be installed and registered first.",
                Style::default().fg(colors::DIM),
            )));
            lines.push(Line::from(Span::styled(
                "      Leave path/port blank to use defaults (signal-cli, 8080).",
                Style::default().fg(colors::DIM),
            )));
        }
        // For WhatsApp: show bridge hint
        if *chan_name == "WhatsApp" {
            lines.push(Line::from(Span::styled(
                "      Uses Baileys bridge (QR code pairing). No Meta Business account needed.",
                Style::default().fg(colors::DIM),
            )));
            lines.push(Line::from(Span::styled(
                "      Leave bridge URL blank for default (ws://localhost:3000).",
                Style::default().fg(colors::DIM),
            )));
        }
        // For Matrix: show hint
        if *chan_name == "Matrix" {
            lines.push(Line::from(Span::styled(
                "      Uses Matrix Client-Server API. Get access token from Element or curl.",
                Style::default().fg(colors::DIM),
            )));
        }
        // For Slack: show hint
        if *chan_name == "Slack" {
            lines.push(Line::from(Span::styled(
                "      Bot Token (xoxb-) from Slack App config. App Token (xapp-) for Socket Mode.",
                Style::default().fg(colors::DIM),
            )));
        }
        // For Email: show hint
        if *chan_name == "Email" {
            lines.push(Line::from(Span::styled(
                "      SMTP for outbound, IMAP for inbound. Use app-specific passwords.",
                Style::default().fg(colors::DIM),
            )));
        }
        // For MQTT: show hint
        if *chan_name == "MQTT" {
            lines.push(Line::from(Span::styled(
                "      Publish/subscribe to MQTT topics. Supports mqtt:// and mqtts:// URLs.",
                Style::default().fg(colors::DIM),
            )));
        }
        // For Mattermost: show hint
        if *chan_name == "Mattermost" {
            lines.push(Line::from(Span::styled(
                "      Personal Access Token from Account Settings → Security.",
                Style::default().fg(colors::DIM),
            )));
        }
        base_idx += field_counts.get(chan_idx).copied().unwrap_or(0);
        lines.push(Line::from(""));
    }

    // Bot Message Policy — rendered at bottom, only if Discord is toggled
    if state.channel_toggled.contains(&0) {
        let policy_options = ["all messages", "@mentioned only", "off (ignore bots)"];
        let policy_focused = state.bot_policy_focused;
        if policy_focused { active_line = lines.len(); }
        lines.push(Line::from(Span::styled(
            if policy_focused { "  ▸ Bot Message Policy" } else { "    Bot Message Policy" },
            if policy_focused {
                Style::default().fg(colors::ACCENT).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(colors::TEXT).add_modifier(Modifier::BOLD)
            },
        )));
        for (pi, label) in policy_options.iter().enumerate() {
            let selected = match (pi, state.allow_bots_mode.as_str()) {
                (0, "on") | (1, "mentions") | (2, "off") => true,
                _ => false,
            };
            let radio = if selected { "◉" } else { "◯" };
            lines.push(Line::from(vec![
                Span::styled("      ", Style::default()),
                Span::styled(
                    format!("{} {}", radio, label),
                    if selected && policy_focused {
                        Style::default().fg(colors::ACCENT).add_modifier(Modifier::BOLD)
                    } else if selected {
                        Style::default().fg(colors::TEXT_BRIGHT).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(colors::DIM)
                    },
                ),
            ]));
        }
        lines.push(Line::from(Span::styled(
            "      B=Cycle policy  Tab=Next field",
            Style::default().fg(colors::DIM),
        )));
        lines.push(Line::from(""));
    }

    let any_toggled = state.channel_toggled.iter().any(|&i| {
        field_counts.get(i).copied().unwrap_or(0) > 0
    });
    if !any_toggled {
        lines.push(Line::from(Span::styled(
            "    No channels selected — press Esc to go back and toggle channels",
            Style::default().fg(colors::DIM),
        )));
        lines.push(Line::from(""));
    }

    lines.push(Line::from(Span::styled(
        "    Tab=Next field  Enter=Continue",
        Style::default().fg(colors::DIM),
    )));

    // Scroll to keep the active field visible — use actual line position,
    // not field index, because headers and separators inflate line count.
    let visible_height = area.height.saturating_sub(2) as usize; // minus border
    let scroll_offset = if active_line > visible_height.saturating_sub(3) {
        (active_line - visible_height.saturating_sub(3)) as u16
    } else {
        0
    };

    let p = Paragraph::new(lines)
        .scroll((scroll_offset, 0))
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded)
            .border_style(colors::border_active())
            .title(" \u{2b21} Channel Config "));
    f.render_widget(p, area);
}

// ── SignalPair (step 8b) ──────────────────────────────────────────────────────

fn render_signal_pair(f: &mut Frame, state: &OnboardingState, area: Rect) {
    let mut lines: Vec<Line> = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Link Signal Device",
            Style::default().fg(colors::ACCENT).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "  On your phone: Signal → Settings → Linked Devices → Add Device",
            Style::default().fg(colors::DIM),
        )),
        Line::from(""),
    ];

    if state.signal_qr_fetching {
        let dots = match (state.tick / 8) % 4 { 0 => ".", 1 => "..", 2 => "...", _ => "" };
        lines.push(Line::from(Span::styled(
            format!("  Starting signal-cli daemon{}", dots),
            Style::default().fg(colors::DIM),
        )));
    } else if let Some(ref err) = state.signal_qr_error {
        lines.push(Line::from(Span::styled(
            format!("  Error: {}", err),
            Style::default().fg(colors::RED).add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Make sure signal-cli is installed and the phone number is registered.",
            Style::default().fg(colors::DIM),
        )));
        lines.push(Line::from(Span::styled(
            "  Press Enter to skip pairing and continue.",
            Style::default().fg(colors::DIM),
        )));
    } else if let Some(ref uri) = state.signal_qr_uri {
        // Render the tsdevice:// URI as a terminal QR code
        for qr_line in render_qr_lines(uri) {
            lines.push(Line::from(Span::raw(format!("  {}", qr_line))));
        }
        lines.push(Line::from(""));
        if state.signal_linked {
            lines.push(Line::from(Span::styled(
                "  ✓ Device linked successfully! Press Enter to continue.",
                Style::default().fg(ratatui::style::Color::Green),
            )));
        } else {
            lines.push(Line::from(Span::styled(
                "  Scan with Signal → Settings → Linked Devices → Add Device",
                Style::default().fg(colors::ACCENT),
            )));
            lines.push(Line::from(Span::styled(
                "  Press Enter once linked (or Esc to skip).",
                Style::default().fg(colors::DIM),
            )));
        }
    }

    let paragraph = ratatui::widgets::Paragraph::new(lines)
        .wrap(ratatui::widgets::Wrap { trim: false });
    f.render_widget(paragraph, area);
}

/// Render a string as a terminal QR code using unicode half-block characters.
/// Returns one String per terminal row. Each module = 1 char wide.
/// Two QR rows are merged into one terminal row via ▀/▄/█/space half-blocks.
fn render_qr_lines(data: &str) -> Vec<String> {
    use qrcode::{QrCode, EcLevel};
    use qrcode::types::Color;

    let code = match QrCode::with_error_correction_level(data.as_bytes(), EcLevel::L) {
        Ok(c) => c,
        Err(_) => return vec!["[QR generation failed]".to_string()],
    };

    let width = code.width();
    // get_colors() returns a flat vec of Color (Dark/Light) in row-major order
    let modules: Vec<bool> = code.into_colors()
        .iter()
        .map(|c| matches!(c, Color::Dark))
        .collect();

    // Quiet zone: 2 modules padding on each side
    const QUIET: usize = 2;
    let padded_w = width + QUIET * 2;
    // Build padded rows (light = false)
    let total_rows = width + QUIET * 2;
    let get_module = |row: usize, col: usize| -> bool {
        if row < QUIET || row >= QUIET + width || col < QUIET || col >= QUIET + width {
            return false; // quiet zone = light
        }
        let r = row - QUIET;
        let c = col - QUIET;
        modules[r * width + c]
    };

    let mut out = Vec::new();
    // Process pairs of rows
    let mut row = 0usize;
    while row < total_rows {
        let mut line = String::new();
        for col in 0..padded_w {
            let top = get_module(row, col);
            let bot = if row + 1 < total_rows { get_module(row + 1, col) } else { false };
            line.push(match (top, bot) {
                (true,  true)  => '\u{2588}', // █ full block
                (true,  false) => '\u{2580}', // ▀ upper half
                (false, true)  => '\u{2584}', // ▄ lower half
                (false, false) => ' ',
            });
        }
        out.push(line);
        row += 2;
    }
    out
}

// ── WhatsAppPair (step 8c) ────────────────────────────────────────────────────

fn render_whatsapp_pair(f: &mut Frame, state: &OnboardingState, area: Rect) {
    let mut lines: Vec<Line> = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Link WhatsApp Device",
            Style::default().fg(colors::ACCENT).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "  On your phone: WhatsApp → Linked Devices → Link a Device",
            Style::default().fg(colors::DIM),
        )),
        Line::from(""),
    ];

    if state.whatsapp_qr_fetching {
        let dots = match (state.tick / 8) % 4 { 0 => ".", 1 => "..", 2 => "...", _ => "" };
        lines.push(Line::from(Span::styled(
            format!("  Connecting to Baileys bridge{}", dots),
            Style::default().fg(colors::DIM),
        )));
    } else if let Some(ref err) = state.whatsapp_qr_error {
        // "already linked" is informational, not fatal
        let is_linked = err.contains("already linked");
        lines.push(Line::from(Span::styled(
            format!("  {}", err),
            if is_linked {
                Style::default().fg(colors::ACCENT).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(colors::RED).add_modifier(Modifier::BOLD)
            },
        )));
        if !is_linked {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  Make sure the Baileys bridge is running (node bridge.js).",
                Style::default().fg(colors::DIM),
            )));
        }
        lines.push(Line::from(Span::styled(
            "  Press Enter to continue.",
            Style::default().fg(colors::DIM),
        )));
    } else if let Some(ref qr) = state.whatsapp_qr_data {
        for qr_line in render_qr_lines(qr) {
            lines.push(Line::from(Span::raw(format!("  {}", qr_line))));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Scan with WhatsApp → Linked Devices → Link a Device",
            Style::default().fg(colors::ACCENT),
        )));
        lines.push(Line::from(Span::styled(
            "  Press Enter once linked (or Esc to skip).",
            Style::default().fg(colors::DIM),
        )));
    }

    let paragraph = ratatui::widgets::Paragraph::new(lines)
        .wrap(ratatui::widgets::Wrap { trim: false });
    f.render_widget(paragraph, area);
}

// ── Gateway (step 9) ──────────────────────────────────────────────────────────

fn render_gateway(f: &mut Frame, state: &OnboardingState, area: Rect) {
    let labels = ["Gateway URL", "Enable Heartbeat", "Heartbeat Interval"];
    let defaults = ["http://localhost:8080", "on", "300"];
    let fields: Vec<(&str, &str, bool)> = labels
        .iter()
        .enumerate()
        .map(|(i, label)| {
            let val = state.gateway_fields.get(i).map(|s| s.as_str()).unwrap_or(defaults[i]);
            (*label, val, i == state.gateway_focus)
        })
        .collect();

    render_config_step(f, "Gateway", "Configure the Zeus API gateway", &fields, area);
}

// ── Workspace (step 11) ───────────────────────────────────────────────────────

fn render_workspace(f: &mut Frame, state: &OnboardingState, area: Rect) {
    let ws_path = state.workspace_path.display().to_string();
    let sessions_path = state.sessions_path.display().to_string();

    let mut lines: Vec<Line> = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Workspace",
            Style::default().fg(colors::ACCENT).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "  File-based workspace for memory, notes, and sessions",
            Style::default().fg(colors::DIM),
        )),
        Line::from(""),
    ];

    // Helper: render one editable path row with focus cursor
    let render_row = |label: &str, value: &str, idx: usize| -> Line<'static> {
        let focused = idx == state.workspace_focus;
        let editing = focused && state.workspace_editing;
        let marker = if focused { "▶ " } else { "  " };
        let value_style = if editing {
            Style::default().fg(colors::ACCENT_BRIGHT).add_modifier(Modifier::BOLD)
        } else if focused {
            Style::default().fg(colors::TEXT_BRIGHT).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(colors::TEXT)
        };
        let shown = if editing { format!("{}█", value) } else { value.to_string() };
        Line::from(vec![
            Span::styled(format!("  {}", marker), Style::default().fg(colors::ACCENT)),
            Span::styled(
                format!("{:<16}", label),
                Style::default().fg(if focused { colors::TEXT_BRIGHT } else { colors::TEXT }).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" │ ", Style::default().fg(colors::BORDER)),
            Span::styled(shown, value_style),
        ])
    };

    lines.push(render_row("Workspace Path", &ws_path, 0));
    lines.push(render_row("Sessions Path",  &sessions_path, 1));
    lines.push(Line::from(""));

    // Generation status
    if state.workspace_generated {
        lines.push(Line::from(vec![
            Span::styled("    \u{2713} ", Style::default().fg(colors::GREEN)),
            Span::styled("Workspace files generated", Style::default().fg(colors::GREEN)),
        ]));
        lines.push(Line::from(Span::styled(
            "      SOUL.md  AGENTS.md  IDENTITY.md  HEARTBEAT.md  MEMORY.md",
            Style::default().fg(colors::DIM),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            "    Workspace will be generated when you continue",
            Style::default().fg(colors::YELLOW),
        )));
    }

    lines.push(Line::from(""));
    let hint = if state.workspace_editing {
        "    Type to edit path  Enter=Save  Esc=Cancel"
    } else {
        "    ↑/↓=Focus  e=Edit path  Enter=Generate & Continue  Esc=Back"
    };
    lines.push(Line::from(Span::styled(
        hint,
        Style::default().fg(colors::DIM),
    )));

    let p = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded)
            .border_style(colors::border_active())
            .title(" \u{2b21} Workspace "));
    f.render_widget(p, area);
}

// ── Features (step 13) ────────────────────────────────────────────────────────

fn render_features(f: &mut Frame, state: &OnboardingState, area: Rect) {
    let mut lines: Vec<Line> = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  UNLOCK ABILITIES",
            Style::default().fg(colors::ACCENT).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "  Enable or disable Zeus subsystems",
            Style::default().fg(colors::DIM),
        )),
        Line::from(""),
    ];

    for (i, (key, label, desc)) in FEATURES.iter().enumerate() {
        let enabled = state.feature_toggles.get(key).copied().unwrap_or(false);
        let selected = i == state.sel;
        let (check, check_style) = if enabled {
            ("[✓]", Style::default().fg(colors::GREEN))
        } else {
            ("[ ]", Style::default().fg(colors::DIM))
        };
        let marker = if selected { "▶ " } else { "  " };
        lines.push(Line::from(vec![
            Span::styled(format!("  {}", marker), Style::default().fg(colors::ACCENT)),
            Span::styled(check, check_style),
            Span::raw(" "),
            Span::styled(
                format!("{:<14}", label),
                if selected {
                    Style::default().fg(colors::TEXT_BRIGHT).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(colors::TEXT)
                },
            ),
            Span::styled(*desc, Style::default().fg(colors::DIM)),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "    ↑/↓=Navigate  Space=Toggle  Enter=Continue",
        Style::default().fg(colors::DIM),
    )));

    let p = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded)
            .border_style(colors::border_active())
            .title(" ⬡ Features "));
    f.render_widget(p, area);
}

// ── Voice (step 14) ────────────────────────────────────────────────────────────

fn render_voice(f: &mut Frame, state: &OnboardingState, area: Rect) {
    let labels = ["STT URL", "Piper TTS URL", "ElevenLabs API Key"];
    let defaults = ["https://your-stt-endpoint", "https://your-tts-endpoint", "sk-..."];
    let fields: Vec<(&str, &str, bool)> = labels
        .iter()
        .enumerate()
        .map(|(i, label)| {
            let val = state.voice_fields.get(i).map(|s| s.as_str()).unwrap_or(defaults[i]);
            (*label, val, i == state.voice_focus)
        })
        .collect();

    render_config_step(f, "Voice", "Configure speech-to-text and text-to-speech providers", &fields, area);
}

// ── Images (step 14) ──────────────────────────────────────────────────────────

fn render_images(f: &mut Frame, state: &OnboardingState, area: Rect) {
    let labels = ["Image Provider", "Provider URL"];
    let defaults = ["gpt-image-1.5", "https://api.openai.com/v1/images"];
    let fields: Vec<(&str, &str, bool)> = labels
        .iter()
        .enumerate()
        .map(|(i, label)| {
            let val = state.image_fields.get(i).map(|s| s.as_str()).unwrap_or(defaults[i]);
            (*label, val, i == state.image_focus)
        })
        .collect();

    render_config_step(f, "Images", "Configure image generation provider (any OpenAI-compatible API works)", &fields, area);
}

// ── Orchestration (step 15) ────────────────────────────────────────────────────

fn render_orchestration(f: &mut Frame, state: &OnboardingState, area: Rect) {
    let labels = ["Enable Heartbeat", "Heartbeat Interval", "Enable Cognitive", "Max Iterations"];
    let defaults = ["enabled", "5m", "enabled", "10"];
    let fields: Vec<(&str, &str, bool)> = labels
        .iter()
        .enumerate()
        .map(|(i, label)| {
            let val = state.orch_fields.get(i).map(|s| s.as_str()).unwrap_or(defaults[i]);
            (*label, val, i == state.orch_focus)
        })
        .collect();

    render_config_step(f, "Orchestration", "Multi-agent coordination and autonomous task execution", &fields, area);
}

// ── Memory (step 16) ──────────────────────────────────────────────────────────

fn render_memory(f: &mut Frame, state: &OnboardingState, area: Rect) {
    let labels = ["Database Path", "Enable FTS", "Embedding Provider"];
    let defaults = ["~/.zeus/memory.db", "enabled", "OpenAI"];
    let fields: Vec<(&str, &str, bool)> = labels
        .iter()
        .enumerate()
        .map(|(i, label)| {
            let val = state.memory_fields.get(i).map(|s| s.as_str()).unwrap_or(defaults[i]);
            (*label, val, i == state.memory_focus)
        })
        .collect();

    render_config_step(f, "Memory", "Configure Mnemosyne long-term memory with SQLite FTS5 + vector embeddings", &fields, area);
}

// ── Launch ─────────────────────────────────────────────────────────────────────

fn render_launch(f: &mut Frame, state: &OnboardingState, area: Rect) {
    let provider = PROVIDERS.get(state.selected_provider)
        .map(|p| p.name).unwrap_or("Anthropic");
    let model = state.selected_model_string();
    let name = if state.agent_name.is_empty() { "zeus" } else { &state.agent_name };
    let gateway = state.gateway_fields.first().map(|s| s.as_str()).unwrap_or("http://localhost:8080");

    // Launch options matching JSX CompleteStep
    let launch_options = [
        ("Launch Gateway", "Start the Zeus API gateway server"),
        ("Launch Agent", "Launch a single Zeus agent instance"),
        ("Save & Exit", "Save configuration and exit to shell"),
    ];

    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  ✓ Setup complete. Your Zeus:",
            Style::default().fg(colors::GREEN).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("    Provider   ", Style::default().fg(colors::DIM)),
            Span::styled(provider, Style::default().fg(colors::TEXT_BRIGHT)),
        ]),
        Line::from(vec![
            Span::styled("    Model      ", Style::default().fg(colors::DIM)),
            Span::styled(model, Style::default().fg(colors::TEXT_BRIGHT)),
        ]),
        Line::from(vec![
            Span::styled("    Name       ", Style::default().fg(colors::DIM)),
            Span::styled(name.to_string(), Style::default().fg(colors::TEXT_BRIGHT)),
        ]),
        Line::from(vec![
            Span::styled("    Gateway    ", Style::default().fg(colors::DIM)),
            Span::styled(gateway.to_string(), Style::default().fg(colors::TEXT_BRIGHT)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  Launch Options:",
            Style::default().fg(colors::ACCENT).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];

    // Add radio button options
    for (i, (option, desc)) in launch_options.iter().enumerate() {
        let selected = i == state.complete_selection;
        let radio = if selected { "◉" } else { "◯" };
        let marker = if selected { "▶ " } else { "  " };
        
        lines.push(Line::from(vec![
            Span::styled(format!("  {}", marker), Style::default().fg(colors::RED)),
            Span::styled(format!("{} ", radio), 
                if selected { Style::default().fg(colors::ACCENT) } else { Style::default().fg(colors::DIM) }
            ),
            Span::styled(
                *option,
                if selected {
                    Style::default().fg(colors::TEXT_BRIGHT).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(colors::TEXT)
                },
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("      ", Style::default()),
            Span::styled(*desc, Style::default().fg(colors::DIM)),
        ]));
        lines.push(Line::from(""));
    }

    lines.push(Line::from(Span::styled(
        "    ↑↓=Select Option  Enter=Launch",
        Style::default().fg(colors::DIM),
    )));

    let p = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded)
            .border_style(colors::border_active())
            .title(" ⬡ Ready "));
    f.render_widget(p, area);
}
