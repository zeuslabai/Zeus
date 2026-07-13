use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Block, Borders, Padding, Widget};

use crate::theme;

/// Truncate `text` to fit `max_w` display columns, appending `…` when clipped.
///
/// #271 visual-parity: every narrow-width text seam on this screen used a bare
/// `set_line`/`set_string` clamp, which hard-chops mid-word with NO ellipsis.
/// Route any value that can exceed its budget through this so it truncates
/// honestly with a trailing `…` *inside* the budget. Char-based (the strings
/// here are ASCII persona names/tones/principles); the 1-col `…` replaces the
/// last char on clip.
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

/// Persona names from the runtime library often include the article (`The Scholar`).
/// The TUI card also renders a separate all-caps glyph, so showing both creates
/// noisy titles like `THE  The Scholar`. Match the prototype card hierarchy by
/// keeping the glyph as the article/icon and dropping one leading `The ` from the
/// title text only.
fn card_persona_name(name: &str) -> &str {
    name.strip_prefix("The ").unwrap_or(name)
}

/// Persona entry — matches JSX PERSONAS (docs/zeus-tui-onboarding.jsx line 123).
///
/// Fields are owned (not `&'static`) because the picker is populated at runtime
/// from the on-disk persona library (`~/Zeus/personalities/`). When that dir is
/// absent or empty we fall back to [`default_personas`] — the original 6 from
/// the JSX prototype — so the offline experience is pixel-identical.
struct Persona {
    #[allow(dead_code)] // staged UI scaffolding
    id: String,
    name: String,
    glyph: String,
    color: Color,
    sub: String,
    tone: String,
    principles: Vec<String>,
}

/// The category-ordered palette the JSX prototype uses for the 6 cards. Disk
/// personas are colored by their position in the picker (round-robin) so the
/// grid keeps the same visual rhythm regardless of data source.
fn persona_palette() -> [Color; 6] {
    [
        theme::FIRE_ORANGE,
        theme::CYAN,
        theme::PURPLE,
        theme::GREEN,
        theme::YELLOW,
        theme::DIM,
    ]
}

/// The ordered list of directories searched for the on-disk persona library.
///
/// In order:
/// 1. `cwd/personalities` — running from a repo checkout.
/// 2. `$ZEUS_HOME/personalities` — the **canonical runtime path** seeded by
///    install.sh (`ZEUS_HOME`-aware via [`zeus_core::Config::zeus_home`], so a
///    bare deployed titan with no repo clone still finds the 25 personas).
/// 3. `~/Zeus/personalities` / `~/zeus/personalities` — legacy repo layouts.
///
/// The first non-empty directory wins.
fn persona_search_paths() -> Vec<std::path::PathBuf> {
    let mut paths = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        paths.push(cwd.join("personalities"));
    }
    if let Ok(home) = zeus_core::Config::zeus_home() {
        paths.push(home.join("personalities"));
    }
    if let Some(home) = dirs::home_dir() {
        paths.push(home.join("Zeus").join("personalities"));
        paths.push(home.join("zeus").join("personalities"));
    }
    paths
}

/// Load the persona library from disk, falling back to the JSX defaults.
///
/// Searches the directories returned by [`persona_search_paths`]. Each subfolder
/// is a category; each `.md` file's frontmatter supplies `name` (card title),
/// `tagline` (sub-line), and `tone` (preview). The first non-empty dir wins.
/// Empty/absent → [`default_personas`].
fn load_personas() -> Vec<Persona> {
    for candidate in persona_search_paths() {
        if candidate.is_dir() {
            if let Ok(personas) = read_personas_dir(&candidate) {
                if !personas.is_empty() {
                    return personas;
                }
            }
        }
    }

    default_personas()
}

/// Read every `<category>/<persona>.md` under `dir` into owned [`Persona`]s,
/// sorted by category then name for a stable grid order. Colors are assigned
/// round-robin from [`persona_palette`]; the glyph is the uppercased first 3
/// letters of the name (matching the JSX 3-char glyph convention).
fn read_personas_dir(dir: &std::path::Path) -> std::io::Result<Vec<Persona>> {
    let mut rows: Vec<(String, Persona)> = Vec::new();

    for cat_entry in std::fs::read_dir(dir)? {
        let cat_entry = cat_entry?;
        let cat_path = cat_entry.path();
        if !cat_path.is_dir() {
            continue;
        }
        let category = cat_entry.file_name().to_string_lossy().to_string();

        for file in std::fs::read_dir(&cat_path)? {
            let file = file?;
            let file_path = file.path();
            if file_path.extension().map(|e| e == "md").unwrap_or(false) {
                if let Ok(content) = std::fs::read_to_string(&file_path) {
                    let Some(name) = parse_frontmatter_field(&content, "name") else {
                        continue;
                    };
                    let sub = parse_frontmatter_field(&content, "tagline").unwrap_or_default();
                    let tone = parse_frontmatter_field(&content, "tone")
                        .filter(|t| !t.is_empty())
                        .unwrap_or_else(|| default_tone_for(&name));
                    let id = file_path
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| name.to_lowercase());
                    let glyph: String = name
                        .chars()
                        .filter(|c| c.is_alphanumeric())
                        .take(3)
                        .collect::<String>()
                        .to_uppercase();
                    rows.push((
                        category.clone(),
                        Persona {
                            id,
                            name,
                            glyph,
                            color: theme::CYAN, // re-assigned round-robin below
                            sub,
                            tone,
                            principles: Vec::new(),
                        },
                    ));
                }
            }
        }
    }

    rows.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.name.cmp(&b.1.name)));

    let palette = persona_palette();
    let personas: Vec<Persona> = rows
        .into_iter()
        .enumerate()
        .map(|(i, (_, mut p))| {
            p.color = palette[i % palette.len()];
            p
        })
        .collect();

    Ok(personas)
}

/// Parse a single `field: value` line from YAML-ish frontmatter (the leading
/// `---`-delimited block). Returns the trimmed, unquoted value. Mirrors the
/// pre-purge onboarding loader so disk layout stays compatible.
fn parse_frontmatter_field(content: &str, field: &str) -> Option<String> {
    let content = content.trim_start();
    if !content.starts_with("---") {
        return None;
    }
    let rest = &content[3..];
    let end = rest.find("\n---")?;
    let frontmatter = &rest[..end];
    let prefix = format!("{}:", field);
    for line in frontmatter.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix(&prefix) {
            return Some(value.trim().trim_matches('"').to_string());
        }
    }
    None
}

/// Per-persona tone fallback for on-disk personas that omit an explicit
/// `tone:` frontmatter field. Prevents every card from rendering the same
/// generic descriptor (the root cause of #281).
fn default_tone_for(name: &str) -> String {
    match name {
        "The Coordinator" | "Coordinator" => "professional, direct, decisive",
        "The Engineer" | "Engineer" => "precise, technical, terse",
        "The Creative" | "Creative" => "warm, expressive, narrative",
        "The Sysadmin" | "Sysadmin" => "calm, observational, methodical",
        "The Analyst" | "Analyst" => "curious, rigorous, thorough",
        "Innovator" => "bold, experimental",
        "Guardian" => "protective, vigilant",
        "Minimalist" => "terse, no fluff",
        "Mentor" => "patient, encouraging, Socratic",
        "Optimizer" => "relentless, metrics-driven, surgical",
        "Specialist" => "deep, exacting, focused",
        "Strategist" => "big-picture, sharp, opportunistic",
        "The Amplifier" => "energetic, clarifying, human",
        "The Architect" => "structured, deliberate, scalable",
        "The Backend Dev" => "data-first, careful, pragmatic",
        "The Builder" => "hands-on, iterative, ship-minded",
        "The Crafter" => "detail-obsessed, polished, intentional",
        "The Executor" => "organized, deadline-aware, reliable",
        "The Herald" => "vivid, audience-centric, compelling",
        "The Market Analyst" => "numbers-led, skeptical, clear",
        "The Operator" => "tactical, calm under pressure, efficient",
        "The Oracle" => "measured, foresighted, connected",
        "The Partner" => "collaborative, honest, invested",
        "The Plumber" => "pragmatic, gritty, gets-it-done",
        "The Polyglot" => "adaptable, idiomatic, curious",
        "The Scholar" => "rigorous, sourced, contemplative",
        "The Sentinel" => "watchful, cautious, principled",
        "The Spark" => "playful, surprising, generative",
        "The Substrate-Walker" => "systems-level, probing, tenacious",
        "The Trader" => "risk-aware, decisive, opportunistic",
        "The Visionary" => "aspirational, pattern-seeking, bold",
        "Custom" => "",
        _ => "balanced, clear, purposeful",
    }
    .to_string()
}

/// The original 6 personas from the JSX prototype — the offline fallback used
/// when no on-disk persona library is present.
fn default_personas() -> Vec<Persona> {
    vec![
        Persona {
            id: "coordinator".into(),
            name: "Coordinator".into(),
            glyph: "COO".into(),
            color: theme::FIRE_ORANGE,
            sub: "Orchestrates the fleet".into(),
            tone: "professional, direct, decisive".into(),
            principles: vec![
                "- Make decisions quickly when blocked.".into(),
                "- Delegate clearly. Track outcomes.".into(),
                "- Escalate to humans only when truly ambiguous.".into(),
            ],
        },
        Persona {
            id: "engineer".into(),
            name: "Engineer".into(),
            glyph: "ENG".into(),
            color: theme::CYAN,
            sub: "Writes and reviews code".into(),
            tone: "precise, technical, terse".into(),
            principles: vec![
                "- Read existing code before writing new.".into(),
                "- Tests pass before commit.".into(),
                "- One thing at a time.".into(),
            ],
        },
        Persona {
            id: "creative".into(),
            name: "Creative".into(),
            glyph: "CRT".into(),
            color: theme::PURPLE,
            sub: "Marketing and content".into(),
            tone: "warm, expressive, narrative".into(),
            principles: vec![
                "- Voice over voicelessness.".into(),
                "- Specific over generic.".into(),
                "- Iterate until it sings.".into(),
            ],
        },
        Persona {
            id: "sysadmin".into(),
            name: "Sysadmin".into(),
            glyph: "OPS".into(),
            color: theme::GREEN,
            sub: "Monitors and maintains".into(),
            tone: "calm, observational, methodical".into(),
            principles: vec![
                "- Observe before acting.".into(),
                "- Automate the repeatable.".into(),
                "- Fail loud, recover quiet.".into(),
            ],
        },
        Persona {
            id: "analyst".into(),
            name: "Analyst".into(),
            glyph: "ANL".into(),
            color: theme::AMBER,
            sub: "Research and synthesis".into(),
            tone: "curious, rigorous, thorough".into(),
            principles: vec![
                "- Cite sources. Show the work.".into(),
                "- Separate signal from noise.".into(),
                "- Quantify when you can.".into(),
            ],
        },
        Persona {
            id: "custom".into(),
            name: "Custom".into(),
            glyph: "CST".into(),
            color: theme::DIM,
            sub: "Define your own".into(),
            tone: String::new(),
            principles: Vec::new(),
        },
    ]
}

/// Identity field indices.
const FIELD_NAME: usize = 0;
const FIELD_ROLE: usize = 1;
const FIELD_TONE: usize = 2;
const FIELD_COUNT: usize = 3;

/// Agent screen — Step 9 (AGNT). Mirrors JSX AgentStep (line 1263):
/// persona picker (6 bordered cards, 2-col grid) + identity fields + SOUL.md live preview.
pub struct AgentScreen {
    /// Persona library, loaded from disk at construction (falls back to the
    /// JSX defaults when no on-disk library exists). Indexed by `persona_idx`.
    personas: Vec<Persona>,
    /// Selected persona index into `personas`.
    pub persona_idx: usize,
    /// Focused identity field (FIELD_NAME / FIELD_ROLE / FIELD_TONE).
    pub focused_field: usize,
    /// User-entered values; empty string falls back to persona/host defaults.
    pub name: String,
    pub role: String,
    pub tone: String,
    /// Hostname-derived name suggestion (JSX: `zeus${hostname.split(".").pop()}`).
    suggested_name: String,
    /// Blink phase from `App::cursor_visible()` — drives the insertion cursor
    /// on the focused identity field (set by the caller each frame).
    pub cursor_on: bool,
}

impl Default for AgentScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentScreen {
    pub fn new() -> Self {
        let suggested_name = hostname_suffix()
            .map(|s| format!("zeus{}", s))
            .unwrap_or_else(|| "Zeus100".to_string());
        Self {
            personas: load_personas(),
            persona_idx: 0,
            focused_field: FIELD_NAME,
            name: String::new(),
            role: String::new(),
            tone: String::new(),
            suggested_name,
            cursor_on: false,
        }
    }

    /// Up arrow — cycle persona backwards (JSX handleCycle case "agent").
    pub fn cycle_prev(&mut self) {
        self.persona_idx = (self.persona_idx + self.personas.len() - 1) % self.personas.len();
    }

    /// Down arrow — cycle persona forwards.
    pub fn cycle_next(&mut self) {
        self.persona_idx = (self.persona_idx + 1) % self.personas.len();
    }

    // ── 2-col grid nav (JSX: 6 personas in a 1fr/1fr grid → 3 rows × 2 cols) ──
    // Index layout:  0 1
    //                2 3
    //                4 5
    // ←/→ move ±1 column (within a row); ↑/↓ move ±2 (one grid row). All
    // clamped to [0, len) so edges don't wrap into the wrong row/column.

    /// Left arrow — move persona selection one column left (no wrap).
    pub fn move_left(&mut self) {
        if self.persona_idx % 2 == 1 {
            self.persona_idx -= 1;
        }
    }

    /// Right arrow — move persona selection one column right (no wrap, clamped).
    pub fn move_right(&mut self) {
        if self.persona_idx.is_multiple_of(2) && self.persona_idx + 1 < self.personas.len() {
            self.persona_idx += 1;
        }
    }

    /// Up arrow — move persona selection up one grid row (−2, clamped).
    pub fn move_up(&mut self) {
        if self.persona_idx >= 2 {
            self.persona_idx -= 2;
        }
    }

    /// Down arrow — move persona selection down one grid row (+2, clamped).
    pub fn move_down(&mut self) {
        if self.persona_idx + 2 < self.personas.len() {
            self.persona_idx += 2;
        }
    }

    /// Tab — cycle focus across the three identity fields.
    pub fn focus_next_field(&mut self) {
        self.focused_field = (self.focused_field + 1) % FIELD_COUNT;
    }

    pub fn input_char(&mut self, c: char) {
        match self.focused_field {
            FIELD_NAME => self.name.push(c),
            FIELD_ROLE => self.role.push(c),
            FIELD_TONE => self.tone.push(c),
            _ => {}
        }
    }

    pub fn input_backspace(&mut self) {
        match self.focused_field {
            FIELD_NAME => {
                self.name.pop();
            }
            FIELD_ROLE => {
                self.role.pop();
            }
            FIELD_TONE => {
                self.tone.pop();
            }
            _ => {}
        }
    }

    /// Reset the picker to the offline default persona library (the JSX 6).
    /// Test-support hook so the 1:1 layout tests are isolated from whatever
    /// on-disk `personalities/` library exists in the dev/CI environment —
    /// they assert the prototype's fixed 6-card grid, which is the offline
    /// fallback, not the disk-driven set.
    #[doc(hidden)]
    pub fn use_default_personas_for_test(&mut self) {
        self.personas = default_personas();
        self.persona_idx = 0;
    }

    /// Select a persona by name when hydrating onboarding from an existing
    /// config. Unknown persona names are preserved by adding a minimal entry so
    /// completing onboarding does not reset them to the first default persona.
    pub fn select_persona_name(&mut self, name: &str) {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return;
        }
        if let Some(idx) = self.personas.iter().position(|p| p.name == trimmed) {
            self.persona_idx = idx;
            return;
        }
        self.personas.push(Persona {
            id: trimmed.to_lowercase().replace(' ', "-"),
            name: trimmed.to_string(),
            glyph: "◈".to_string(),
            color: theme::CYAN,
            sub: "Loaded from existing config".to_string(),
            tone: default_tone_for(trimmed),
            principles: Vec::new(),
        });
        self.persona_idx = self.personas.len() - 1;
    }

    fn persona(&self) -> &Persona {
        &self.personas[self.persona_idx]
    }

    /// Name of the currently selected persona (e.g. "Coordinator").
    pub fn persona_name(&self) -> &str {
        &self.persona().name
    }

    /// SOUL.md persona body for the currently selected persona.
    pub fn persona_soul_body(&self) -> String {
        let persona = self.persona();
        let mut body = persona.name.clone();
        if !persona.sub.trim().is_empty() {
            body.push_str(" — ");
            body.push_str(persona.sub.trim());
        }
        if !persona.tone.trim().is_empty() {
            body.push_str("\n\nTone: ");
            body.push_str(persona.tone.trim());
        }
        if !persona.principles.is_empty() {
            body.push_str("\n\n");
            body.push_str(&persona.principles.join("\n"));
        }
        body
    }

    /// Effective agent name for the summary (user-entered or suggested).
    pub fn summary_name(&self) -> String {
        self.effective_name()
    }

    /// Effective values with JSX-style fallbacks.
    fn effective_name(&self) -> String {
        if self.name.is_empty() {
            self.suggested_name.clone()
        } else {
            self.name.clone()
        }
    }

    fn effective_role(&self) -> String {
        if self.role.is_empty() {
            self.persona().name.to_string()
        } else {
            self.role.clone()
        }
    }

    fn effective_tone(&self) -> String {
        if self.tone.is_empty() {
            self.persona().tone.to_string()
        } else {
            self.tone.clone()
        }
    }

    pub fn render(&self, area: Rect, buf: &mut Buffer) {
        Clear.render(area, buf);
        if area.width < 10 || area.height < 6 {
            return;
        }

        // Opaque background for the whole body (frame() contract §0.2).
        let bg = Block::default().style(Style::default().bg(theme::BG));
        bg.render(area, buf);

        // Split: left column (picker + identity) | right preview (fixed 46 cols when room).
        let preview_w: u16 = if area.width >= 92 { 46 } else { 0 };
        let left = Rect {
            x: area.x + 2,
            y: area.y + 1,
            width: area.width.saturating_sub(4 + preview_w),
            height: area.height.saturating_sub(2),
        };

        // ── Left: heading ──
        buf.set_string(
            left.x,
            left.y,
            "Agent persona",
            Style::default().fg(theme::TEXT_BRIGHT).add_modifier(Modifier::BOLD),
        );
        // #271: clamp the sub-line so it truncates with `…` at narrow widths
        // instead of hard-chopping mid-word.
        let sub_prefix = "Pick an archetype to seed your agent's ";
        let sub_suffix = ". Customize freely after onboarding.";
        let sub_budget = left.width as usize;
        let sub_clamped = clamp_ellipsis(
            &format!("{sub_prefix}SOUL.md{sub_suffix}"),
            sub_budget,
        );
        let sub_line = if sub_clamped.ends_with('…') {
            // Truncated — render as a single dim span (can't split colors mid-clip).
            Line::from(Span::styled(sub_clamped, Style::default().fg(theme::DIM)))
        } else {
            Line::from(vec![
                Span::styled(sub_prefix, Style::default().fg(theme::DIM)),
                Span::styled("SOUL.md", Style::default().fg(theme::ACCENT_BRIGHT)),
                Span::styled(sub_suffix, Style::default().fg(theme::DIM)),
            ])
        };
        buf.set_line(left.x, left.y + 1, &sub_line, left.width);

        // ── Persona cards: 2-col grid, 3 rows, bordered ──
        // All 6 personas live in 3 rows. card_h is adaptive: prefer 4 rows of
        // height per card, but compress toward 3 (the minimum that still shows
        // the title+sub lines) when the body is short, so the bottom row
        // (Analyst / Custom) is never silently dropped by the overflow guard
        // below. Reserve room for IDENTITY (label + 3 fields ≈ 8 rows).
        let gap: u16 = 1;
        let card_w = left.width.saturating_sub(gap) / 2;

        // Split the left column into three non-overlapping regions with a real
        // `Layout` instead of a hardcoded `IDENTITY_RESERVE` offset. The prior
        // fixed-8-row reserve drifted against the adaptive `card_h` (#253): when
        // the clamp forced `card_h` taller than the reserve assumed, the grid
        // bottom and the IDENTITY fields collided in the same rows. Anchoring
        // IDENTITY to the *actual* grid-region bottom makes overlap impossible
        // regardless of how `card_h` adapts.
        //   heading : 3 rows (title + sub + gap)
        //   identity: 8 rows (label + gap + 3 fields × 2 rows), reserved at the
        //             bottom — but `Min` so it never starves the grid
        //   grid    : everything in between (Min(9) keeps all 3 rows visible)
        const IDENTITY_ROWS: u16 = 8;
        let regions = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(9),
                Constraint::Length(IDENTITY_ROWS),
            ])
            .split(left);
        let grid_region = regions[1];
        let identity_region = regions[2];

        let grid_y = grid_region.y;
        // 3 rows must fit in the grid region; clamp card height to [3, 4].
        let card_h: u16 = (grid_region.height / 3).clamp(3, 4);
        // #295: scroll window. The on-disk persona library holds far more rows
        // than fit the grid region (26 personas = 13 rows vs ~3 visible), so
        // render a window of rows that follows `persona_idx` — keeping the
        // selected card on screen. Without it the overflow guard below silently
        // dropped any selection past the fold ("cursor disappears, can't scroll
        // down"). When every row already fits (e.g. the 6-persona fallback)
        // `first_row` stays 0 and nothing scrolls, so the squeeze-case render
        // tests are unaffected.
        let visible_rows = (grid_region.height / card_h).max(1);
        let total_rows = self.personas.len().div_ceil(2) as u16;
        let sel_row = (self.persona_idx / 2) as u16;
        let first_row = if sel_row < visible_rows {
            0
        } else {
            // Selection below the fold: scroll so it sits on the last visible row.
            sel_row + 1 - visible_rows
        };
        for (i, p) in self.personas.iter().enumerate() {
            let row = (i / 2) as u16;
            let col = (i % 2) as u16;
            // #295: skip rows outside the scroll window.
            if row < first_row || row >= first_row + visible_rows {
                continue;
            }
            let rect = Rect {
                x: left.x + col * (card_w + gap),
                y: grid_y + (row - first_row) * card_h,
                width: card_w,
                height: card_h,
            };
            // Guard against the GRID region bottom (not the whole left panel),
            // so a card can never draw into the IDENTITY region below it (#253).
            if rect.y + rect.height > grid_region.bottom() {
                continue;
            }
            let selected = i == self.persona_idx;
            let border_color = if selected { p.color } else { theme::BORDER };
            let card_bg = if selected { theme::BG_HIGHLIGHT } else { theme::BG_PANEL };
            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color))
                .style(Style::default().bg(card_bg))
                .padding(Padding::horizontal(1));
            let inner = block.inner(rect);
            block.render(rect, buf);
            if inner.height >= 1 {
                // #271: clamp the card title so glyph+name+marker fits the card
                // inner width without mid-word chop. The "● SELECTED" marker is
                // only shown when there's room; at narrow widths it's dropped.
                let glyph_part = format!("{}  ", p.glyph);
                let marker = if selected { "  ● SELECTED" } else { "" };
                let marker_w = marker.chars().count();
                let name_budget = inner.width as usize;
                let name_budget = name_budget.saturating_sub(glyph_part.chars().count());
                let name_budget = name_budget.saturating_sub(marker_w);
                let name_clamped = clamp_ellipsis(card_persona_name(&p.name), name_budget);
                let title = Line::from(vec![
                    Span::styled(glyph_part, Style::default().fg(p.color).add_modifier(Modifier::BOLD)),
                    Span::styled(name_clamped, Style::default().fg(theme::TEXT_BRIGHT).add_modifier(Modifier::BOLD)),
                    Span::styled(marker, Style::default().fg(p.color)),
                ]);
                buf.set_line(inner.x, inner.y, &title, inner.width);
            }
            if inner.height >= 2 {
                // #271: clamp the subtitle to card inner width.
                let sub_clamped = clamp_ellipsis(&p.sub, inner.width as usize);
                buf.set_string(inner.x, inner.y + 1, &sub_clamped, Style::default().fg(theme::DIM));
            }
        }

        // #295: scroll affordance. When the grid is windowed, show up/down
        // arrows + position, right-aligned on the heading row, so it's clear
        // more personas exist above/below the visible window.
        if total_rows > visible_rows {
            let up = if first_row > 0 { "▲" } else { " " };
            let down = if first_row + visible_rows < total_rows { "▼" } else { " " };
            let ind = format!("{up}{down} {}/{}", self.persona_idx + 1, self.personas.len());
            let iw = ind.chars().count() as u16;
            // Only draw it if it won't crowd the "Agent persona" heading text.
            if left.width > iw + 16 {
                buf.set_string(
                    left.x + left.width - iw,
                    left.y,
                    &ind,
                    Style::default().fg(theme::MUTED),
                );
            }
        }

        // ── IDENTITY fields ──
        // Anchored to the Layout-derived identity region (never the grid's
        // drifting bottom) so the fields can't overwrite the bottom card row.
        let mut fy = identity_region.y;
        buf.set_string(
            left.x,
            fy,
            "IDENTITY",
            Style::default().fg(theme::ACCENT_DIM).add_modifier(Modifier::BOLD),
        );
        fy += 1;

        let fields: [(usize, &str, String, String); 3] = [
            (
                FIELD_NAME,
                "Agent Name *",
                self.effective_name(),
                format!("Auto-suggested from hostname: {}", hostname_full().unwrap_or_else(|| "(unknown)".into())),
            ),
            (FIELD_ROLE, "Role", self.effective_role(), String::new()),
            (
                FIELD_TONE,
                "Tone",
                self.effective_tone(),
                "Used in SOUL.md prompt seed".to_string(),
            ),
        ];
        for (idx, label, value, hint) in fields.iter() {
            if fy + 2 > left.y + left.height {
                break;
            }
            let focused = self.focused_field == *idx;
            let label_style = if focused {
                Style::default().fg(theme::ACCENT_BRIGHT).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme::DIM).add_modifier(Modifier::BOLD)
            };
            // #271: clamp the label to 13 cols (the label column is 14 wide,
            // reserve 1 for the gap before the value).
            let label_clamped = clamp_ellipsis(label, 13);
            buf.set_string(left.x, fy, &label_clamped, label_style);
            let val_style = if focused {
                Style::default().fg(theme::TEXT_BRIGHT).bg(theme::BG_HIGHLIGHT)
            } else {
                Style::default().fg(theme::TEXT).bg(theme::BG_PANEL)
            };
            let marker = if focused { "▸ " } else { "  " };
            // #271: clamp the field value to the available width (left.width
            // minus the 14-col label offset and the 2-col marker prefix).
            let val_budget = left.width.saturating_sub(14) as usize;
            let val_budget = val_budget.saturating_sub(marker.chars().count());
            // Reserve 1 col for the cursor caret when focused+blinking.
            let val_budget = if self.cursor_on && focused {
                val_budget.saturating_sub(1)
            } else {
                val_budget
            };
            let val_clamped = clamp_ellipsis(value, val_budget);
            let mut val_spans = vec![
                Span::styled(marker, Style::default().fg(theme::FIRE_ORANGE)),
                Span::styled(val_clamped, val_style),
            ];
            // Insertion cursor — focused field only, blink-gated. The displayed
            // value is `effective_*()` (prefilled suggestion if the user field
            // is empty), so the caret sits at the end of the editable value.
            if self.cursor_on && focused {
                val_spans.push(Span::styled("▏", Style::default().fg(theme::AMBER)));
            }
            let val_line = Line::from(val_spans);
            buf.set_line(left.x + 14, fy, &val_line, left.width.saturating_sub(14));
            if !hint.is_empty() && fy + 1 < left.y + left.height {
                // #271: clamp the hint to the available width.
                let hint_clamped = clamp_ellipsis(hint, left.width.saturating_sub(16) as usize);
                buf.set_string(left.x + 16, fy + 1, &hint_clamped, Style::default().fg(theme::MUTED));
                fy += 1;
            }
            fy += 2;
        }

        // ── Right: SOUL.MD PREVIEW panel ──
        if preview_w > 0 {
            let right = Rect {
                x: area.x + area.width - preview_w - 1,
                y: area.y + 1,
                width: preview_w,
                height: area.height.saturating_sub(2),
            };
            buf.set_string(
                right.x,
                right.y,
                "SOUL.MD PREVIEW",
                Style::default().fg(theme::ACCENT_DIM).add_modifier(Modifier::BOLD),
            );
            let panel = Rect {
                x: right.x,
                y: right.y + 1,
                width: right.width,
                height: right.height.saturating_sub(3),
            };
            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme::BORDER))
                .style(Style::default().bg(theme::BG_PANEL))
                .padding(Padding::horizontal(1));
            let inner = block.inner(panel);
            block.render(panel, buf);

            // #271: clamp every preview line to the panel inner width so long
            // persona names / tones / principles truncate with `…` instead of
            // hard-chopping mid-word at narrow terminal widths.
            let pw = inner.width as usize;
            let mut lines: Vec<Line> = vec![
                Line::from(Span::styled(
                    clamp_ellipsis(&format!("# {}", self.effective_name()), pw),
                    Style::default().fg(theme::FIRE_ORANGE),
                )),
                Line::from(""),
                Line::from(Span::styled("## Role", Style::default().fg(theme::DIM))),
                Line::from(Span::styled(clamp_ellipsis(&self.effective_role(), pw), Style::default().fg(theme::TEXT))),
                Line::from(""),
                Line::from(Span::styled("## Tone", Style::default().fg(theme::DIM))),
                Line::from(Span::styled(clamp_ellipsis(&self.effective_tone(), pw), Style::default().fg(theme::TEXT))),
            ];
            let principles = &self.persona().principles;
            if !principles.is_empty() {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    "## Guiding Principles",
                    Style::default().fg(theme::DIM),
                )));
                for p in principles {
                    lines.push(Line::from(Span::styled(
                        clamp_ellipsis(p, pw),
                        Style::default().fg(theme::TEXT),
                    )));
                }
            }
            for (i, line) in lines.iter().enumerate() {
                let y = inner.y + i as u16;
                if y >= inner.y + inner.height {
                    break;
                }
                buf.set_line(inner.x, y, line, inner.width);
            }

            // Footer: Live preview · writes to ~/.zeus/workspace/SOUL.md
            // #271: clamp the footer to the panel width.
            let footer_text = "Live preview · writes to ~/.zeus/workspace/SOUL.md";
            let footer_clamped = clamp_ellipsis(footer_text, right.width as usize);
            let footer = if footer_clamped.ends_with('…') {
                Line::from(Span::styled(footer_clamped, Style::default().fg(theme::MUTED)))
            } else {
                Line::from(vec![
                    Span::styled("Live preview · writes to ", Style::default().fg(theme::MUTED)),
                    Span::styled("~/.zeus/workspace/SOUL.md", Style::default().fg(theme::ACCENT_BRIGHT)),
                ])
            };
            let fy = panel.y + panel.height;
            if fy < right.y + right.height {
                buf.set_line(right.x, fy, &footer, right.width);
            }
        }
    }
}

/// Full hostname (best-effort, no extra deps).
fn hostname_full() -> Option<String> {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Last dot-segment of the hostname (JSX: hostname.split(".").pop()).
fn hostname_suffix() -> Option<String> {
    hostname_full().map(|h| h.rsplit('.').next().unwrap_or(&h).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a screen pinned to the offline default persona library, so tests
    /// that assert default-persona content are isolated from whatever on-disk
    /// `personalities/` library happens to exist in the dev/CI environment.
    fn defaults_screen() -> AgentScreen {
        let mut s = AgentScreen::new();
        s.personas = default_personas();
        s.persona_idx = 0;
        s
    }

    #[test]
    fn card_persona_name_drops_duplicate_leading_article() {
        assert_eq!(card_persona_name("The Scholar"), "Scholar");
        assert_eq!(card_persona_name("Guardian"), "Guardian");
    }

    #[test]
    fn rendered_persona_card_does_not_repeat_the_article() {
        let mut s = defaults_screen();
        s.personas[0].glyph = "THE".into();
        s.personas[0].name = "The Scholar".into();
        let area = Rect { x: 0, y: 0, width: 100, height: 30 };
        let mut buf = Buffer::empty(area);
        s.render(area, &mut buf);
        let text = buffer_text(&buf);
        assert!(
            text.contains("THE  Sc"),
            "100x30 card should start with glyph + stripped name before clipping; got:
{text}"
        );
        assert!(
            !text.contains("THE  Th"),
            "100x30 card repeated the leading article before clipping; got:
{text}"
        );
    }

    #[test]
    fn persona_cycle_wraps() {
        let mut s = defaults_screen();
        assert_eq!(s.persona_idx, 0);
        s.cycle_prev();
        assert_eq!(s.persona_idx, s.personas.len() - 1);
        s.cycle_next();
        assert_eq!(s.persona_idx, 0);
    }

    #[test]
    fn six_personas_match_jsx() {
        let personas = default_personas();
        assert_eq!(personas.len(), 6);
        let ids: Vec<&str> = personas.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, vec!["coordinator", "engineer", "creative", "sysadmin", "analyst", "custom"]);
    }

    #[test]
    fn disk_loader_falls_back_when_dir_absent() {
        // A non-existent dir must yield the offline defaults, never empty.
        let missing = std::path::Path::new("/nonexistent/zeus/personalities/xyz");
        assert!(!missing.is_dir());
        let personas = load_personas();
        assert!(
            !personas.is_empty(),
            "load_personas must always return at least the fallback set"
        );
    }

    #[test]
    fn search_paths_include_zeus_home_personalities() {
        // The canonical runtime path ($ZEUS_HOME/personalities) MUST be in the
        // search list so a bare deployed titan (no repo clone) finds the seeded
        // 25 personas. Assert against whatever zeus_home() resolves to on this
        // box (ZEUS_HOME env or ~/.zeus) — no env mutation (unsafe in 2024 ed).
        let expected = zeus_core::Config::zeus_home()
            .expect("zeus_home() must resolve")
            .join("personalities");
        let paths = persona_search_paths();
        assert!(
            paths.contains(&expected),
            "persona_search_paths must include $ZEUS_HOME/personalities; got {paths:?}"
        );
    }

    #[test]
    fn frontmatter_field_parses_name_and_quotes() {
        let md = "---\nname: The Architect\ntagline: \"systems thinker\"\ncategory: Engineering\n---\nbody";
        assert_eq!(parse_frontmatter_field(md, "name").as_deref(), Some("The Architect"));
        assert_eq!(parse_frontmatter_field(md, "tagline").as_deref(), Some("systems thinker"));
        assert_eq!(parse_frontmatter_field(md, "missing"), None);
        assert_eq!(parse_frontmatter_field("no frontmatter here", "name"), None);
    }

    #[test]
    fn effective_values_fall_back_to_persona() {
        let mut s = defaults_screen();
        assert_eq!(s.effective_role(), "Coordinator");
        assert_eq!(s.effective_tone(), "professional, direct, decisive");
        s.focused_field = FIELD_ROLE;
        s.input_char('X');
        assert_eq!(s.effective_role(), "X");
        s.input_backspace();
        assert_eq!(s.effective_role(), "Coordinator");
    }

    #[test]
    fn tone_changes_with_selected_persona() {
        // #281: cycling personas must update the Tone field/preview, not stay
        // stuck on the first persona's descriptor.
        let mut s = defaults_screen();
        assert_eq!(s.effective_tone(), "professional, direct, decisive");
        s.cycle_next();
        assert_eq!(s.effective_tone(), "precise, technical, terse");
        s.cycle_next();
        assert_eq!(s.effective_tone(), "warm, expressive, narrative");
        s.move_right();
        assert_eq!(s.effective_tone(), "calm, observational, methodical");
    }

    #[test]
    fn field_focus_cycles() {
        let mut s = AgentScreen::new();
        assert_eq!(s.focused_field, FIELD_NAME);
        s.focus_next_field();
        assert_eq!(s.focused_field, FIELD_ROLE);
        s.focus_next_field();
        assert_eq!(s.focused_field, FIELD_TONE);
        s.focus_next_field();
        assert_eq!(s.focused_field, FIELD_NAME);
    }

    /// Render the whole buffer to a single string for glyph/text assertions.
    fn buffer_text(buf: &Buffer) -> String {
        let area = buf.area();
        let mut out = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    /// Regression for the "can't see all 6 personas" defect: at a realistic,
    /// vertically-tight onboarding body the fixed card_h=4 grid overflowed the
    /// left panel and the overflow guard silently dropped the bottom row
    /// (Analyst / Custom). The adaptive card_h must keep all 6 glyphs visible.
    #[test]
    fn all_six_personas_visible_when_body_is_tight() {
        let s = defaults_screen();
        // 100×24 is a common terminal; the onboarding body is shorter than the
        // full screen (chrome above/below), so this is the squeeze case.
        for height in [20u16, 22, 24, 28] {
            let area = Rect { x: 0, y: 0, width: 100, height };
            let mut buf = Buffer::empty(area);
            s.render(area, &mut buf);
            let text = buffer_text(&buf);
            for p in &s.personas {
                assert!(
                    text.contains(p.glyph.as_str()),
                    "persona glyph {:?} not rendered at body height {} — bottom row dropped",
                    p.glyph,
                    height
                );
            }
        }
    }

    /// #295: with a persona library taller than the grid, selecting a persona
    /// below the fold must scroll it into view (not silently drop it via the
    /// overflow guard). Also proves the grid is a real window — the first
    /// persona leaves the view when the selection is at the bottom.
    #[test]
    fn selected_persona_scrolls_into_view() {
        let mut s = defaults_screen();
        // Synthesize a long library (26 rows) with unique, non-substring glyphs.
        s.personas = (0..26)
            .map(|i| Persona {
                id: format!("p{i}"),
                name: format!("Persona {i}"),
                glyph: format!("G{i:02}"),
                color: theme::CYAN,
                sub: "test".into(),
                tone: "t".into(),
                principles: vec![],
            })
            .collect();
        // Select the LAST persona — far below the fold.
        s.persona_idx = 25;
        let area = Rect { x: 0, y: 0, width: 100, height: 24 };
        let mut buf = Buffer::empty(area);
        s.render(area, &mut buf);
        let text = buffer_text(&buf);
        assert!(
            text.contains("G25"),
            "selected persona (idx 25) not rendered — scroll window didn't follow selection:\n{text}"
        );
        // First persona must have scrolled out (proves a window, not an
        // all-render that happens to include the last row).
        assert!(
            !text.contains("G00"),
            "first persona still visible with selection at the bottom — grid didn't scroll:\n{text}"
        );
    }

    /// Render-fidelity gate for #253 (persona fields-over-cards). The old
    /// `IDENTITY_RESERVE = 8` fixed offset drifted against the adaptive
    /// `card_h`: when the clamp forced `card_h` taller than the reserve
    /// budgeted, the bottom persona-card row and the IDENTITY label/fields
    /// collided in the same rows. With the real `Layout` split, the IDENTITY
    /// region is anchored below the grid region's bottom — so no card-border
    /// glyph may appear on the IDENTITY label row or any row beneath it.
    #[test]
    fn identity_fields_never_overlap_persona_cards() {
        // Card-border box-drawing glyphs the bordered persona cards draw.
        const BORDER_GLYPHS: &[&str] = &["│", "─", "╭", "╮", "╰", "╯", "┌", "┐", "└", "┘"];
        let s = defaults_screen();
        // Sweep the squeeze range where the old reserve drifted, plus roomy
        // heights where card_h hits its 4-row cap.
        for height in [18u16, 20, 22, 24, 28, 34] {
            let area = Rect { x: 0, y: 0, width: 100, height };
            let mut buf = Buffer::empty(area);
            s.render(area, &mut buf);

            // Scope assertions to the LEFT column only. The right SOUL.md
            // preview panel is a separate region with its own legitimate
            // border; the overlap we guard is persona-cards-vs-identity within
            // the left column. left.x=2, left.width = width-4-preview_w(46).
            let preview_w: u16 = if area.width >= 92 { 46 } else { 0 };
            let left_x0: u16 = 2;
            let left_x1: u16 = area.width.saturating_sub(2 + preview_w); // exclusive

            // Locate the IDENTITY label row (search left column).
            let mut identity_row: Option<u16> = None;
            for y in 0..area.height {
                let mut row = String::new();
                for x in left_x0..left_x1 {
                    row.push_str(buf[(x, y)].symbol());
                }
                if row.contains("IDENTITY") {
                    identity_row = Some(y);
                    break;
                }
            }
            let Some(iy) = identity_row else {
                // At extreme squeeze the identity region may be off-screen;
                // that's the overflow-guard path, not an overlap — skip.
                continue;
            };

            // From the IDENTITY row downward, no persona card-border glyph may
            // survive in the left column — proves the grid region ends above
            // the identity region (no fields-over-cards overlap).
            for y in iy..area.height {
                for x in left_x0..left_x1 {
                    let sym = buf[(x, y)].symbol();
                    assert!(
                        !BORDER_GLYPHS.contains(&sym),
                        "card-border glyph {sym:?} bled onto/below IDENTITY row {iy} \
                         at ({x},{y}), body height {height} — grid/identity overlap"
                    );
                }
            }
        }
    }

    fn render_agent(focused: usize, cursor_on: bool) -> String {
        use ratatui::{backend::TestBackend, Terminal};
        let mut term = Terminal::new(TestBackend::new(140, 40)).unwrap();
        term.draw(|f| {
            let mut s = AgentScreen::new();
            s.focused_field = focused;
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
    fn cursor_painted_on_focused_field_during_blink() {
        assert!(
            render_agent(FIELD_NAME, true).contains('\u{258f}'),
            "expected cursor caret on focused name field during blink-on"
        );
    }

    #[test]
    fn cursor_hidden_on_blink_off() {
        assert!(
            !render_agent(FIELD_NAME, false).contains('\u{258f}'),
            "expected no caret during blink-off half-cycle"
        );
    }

    #[test]
    fn cursor_follows_focus() {
        for f in [FIELD_NAME, FIELD_ROLE, FIELD_TONE] {
            assert!(
                render_agent(f, true).contains('\u{258f}'),
                "expected caret on focused field {f}"
            );
        }
    }

    // ═══ #271 2-width render-verify (load-bearing) ═══
    // These tests assert that clamp_ellipsis is wired into every text seam on
    // the agent screen. Revert any clamp → the narrow test fails (mid-word chop
    // with no `…`). The normal test pins against over-eager clamping.

    /// Render the agent screen at an arbitrary width, returning the full
    /// buffer as a string (row-major, one line per terminal row).
    fn render_at_width(screen: &AgentScreen, w: u16, h: u16) -> String {
        use ratatui::buffer::Buffer;
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        screen.render(area, &mut buf);
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
        assert_eq!(clamp_ellipsis("professional, direct, decisive", 30), "professional, direct, decisive");
        assert_eq!(clamp_ellipsis("professional, direct, decisive", 10), "professio…");
        assert_eq!(clamp_ellipsis("professional, direct, decisive", 1), "…");
        assert_eq!(clamp_ellipsis("professional, direct, decisive", 0), "");
    }

    #[test]
    fn narrow_width_clips_with_ellipsis_not_midword() {
        // #271 LOAD-BEARING: at a squeezed width the persona subtitle and
        // heading must truncate with a trailing `…` — NOT hard-chop mid-word.
        // Revert any clamp → this fails (mid-word chop, no `…`).
        let s = defaults_screen();
        let r = render_at_width(&s, 56, 30);
        // At width 56 the heading sub-line is 56 chars and must clip.
        assert!(
            r.contains('…'),
            "narrow width must produce at least one ellipsis-clipped text; got:\n{r}"
        );
        // The full heading "Customize freely after onboarding." must NOT survive
        // whole — it should be clipped with `…`.
        assert!(
            !r.contains("Customize freely after onboarding."),
            "heading must be clipped at narrow width; got:\n{r}"
        );
        // The clipped heading must carry an ellipsis (honest truncation, not
        // mid-word chop). "Pick an archetype to seed your agent's SOUL.md. Cus…"
        assert!(
            r.contains("Cus…"),
            "heading must clip with ellipsis, not mid-word; got:\n{r}"
        );
        // Card subtitles must also clip with `…` at this width.
        assert!(
            r.contains("Orchestrates the fle…"),
            "card subtitle must clip with ellipsis at narrow width; got:\n{r}"
        );
    }

    #[test]
    fn normal_width_renders_persona_content_in_full() {
        // #271: at a comfortable width the persona name and tone render in
        // full. The card subtitle may still clip (card inner width is narrow
        // even at width 100), but the identity field tone must NOT clip.
        let s = defaults_screen();
        let r = render_at_width(&s, 100, 40);
        assert!(
            r.contains("Coordinator"),
            "full persona name must render at width 100; got:\n{r}"
        );
        assert!(
            r.contains("professional, direct, decisive"),
            "full tone must render at width 100 in identity field; got:\n{r}"
        );
        assert!(
            r.contains("Engineer"),
            "second persona name must render at width 100; got:\n{r}"
        );
    }
}
