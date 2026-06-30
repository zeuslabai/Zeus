use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Block, Borders, Widget};

use crate::theme;

/// Skill entry — wired to the real on-disk skill library (#247).
/// Fields are owned (loaded at runtime from `~/.zeus/skills/*/SKILL.md`),
/// not `&'static` const data. The JSX proto's `category`/`recommended` have
/// no backend equivalent (0 of the installed SKILL.md declare a category), so
/// `category` is *derived* from a keyword map (`derive_category`) to preserve
/// the proto's pixel-identical category tabs, and `recommended` is derived
/// from a small curated id-set (`is_recommended`).
struct Skill {
    id: String,
    name: String,
    desc: String,
    category: String,
    recommended: bool,
}

/// The fallback skill list — used only when the on-disk skills dir is empty or
/// unreadable (fresh box before any skills installed). Mirrors the original
/// proto entries so the screen is never blank in a no-skills environment.
fn fallback_skills() -> Vec<Skill> {
    const FALLBACK: &[(&str, &str, &str, &str, bool)] = &[
        ("calendar-pro", "Calendar Pro", "Auto-schedule + conflict detection", "Productivity", true),
        ("email-triage", "Email Triage", "Inbox prioritization", "Productivity", true),
        ("git-flow", "Git Flow", "Branch + PR automation", "Dev", true),
    ];
    FALLBACK
        .iter()
        .map(|&(id, name, desc, cat, rec)| Skill {
            id: id.to_string(),
            name: name.to_string(),
            desc: desc.to_string(),
            category: cat.to_string(),
            recommended: rec,
        })
        .collect()
}

/// Curated recommended-skill ids (the proto flagged a handful; the backend has
/// no `recommended` field). Kept small + display-only.
fn is_recommended(id: &str) -> bool {
    matches!(
        id,
        "git-flow" | "executing-plans" | "brainstorming" | "doc-coauthoring" | "claude-api"
    )
}

/// Derive a category bucket from the skill id + description, since no installed
/// SKILL.md declares a category. Keyword-matched into the proto's fixed tab set
/// (`CATEGORIES`); anything unmatched falls into "Productivity" (the catch-all
/// the proto's "All" tab already covers). Pixel-identical tabs, derived data.
fn derive_category(id: &str, desc: &str) -> String {
    let hay = format!("{} {}", id, desc).to_lowercase();
    let has = |kws: &[&str]| kws.iter().any(|k| hay.contains(k));
    if has(&["git", "ci", "test", "code", "api", "mcp", "deploy", "debug", "build", "webapp", "frontend", "claw"]) {
        "Dev".to_string()
    } else if has(&["market", "seo", "ad", "campaign", "brand", "copy", "content", "email", "growth", "social", "cro", "aso"]) {
        "Marketing".to_string()
    } else if has(&["secur", "audit", "compli", "privacy", "vuln", "threat"]) {
        "Security".to_string()
    } else if has(&["research", "analy", "competitor", "trend", "data", "deep-research", "profiling"]) {
        "Research".to_string()
    } else {
        "Productivity".to_string()
    }
}

/// Thin, sync, zero-dep SKILL.md frontmatter parser. Reads `name:` +
/// `description:` from the `---`-delimited YAML head. Handles both bare and
/// double-quoted scalar values (descriptions are often long quoted strings with
/// embedded colons). Deliberately does NOT pull in `zeus-skills` (which would
/// drag wasmtime/libloading/reqwest into the TUI build) — the screen only needs
/// name/description/derived-category for display + the id for toggle-persist.
fn load_skills_from_disk() -> Vec<Skill> {
    let dir = match zeus_core::Config::zeus_home() {
        Ok(home) => home.join("skills"),
        Err(_) => return Vec::new(),
    };
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };
    let mut out: Vec<Skill> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let md = path.join("SKILL.md");
        let text = match std::fs::read_to_string(&md) {
            Ok(t) => t,
            Err(_) => continue,
        };
        // Dir name is the canonical skill id.
        let id = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        let (name, desc) = parse_frontmatter(&text);
        let name = name.unwrap_or_else(|| id.clone());
        let desc = desc.unwrap_or_default();
        let category = derive_category(&id, &desc);
        let recommended = is_recommended(&id);
        out.push(Skill { id, name, desc, category, recommended });
    }
    // Stable alphabetical order by name for deterministic rendering.
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

/// Extract `name` + `description` from a SKILL.md frontmatter block. Returns
/// `(name, description)`; either may be `None` if absent. Scans only the first
/// `---`…`---` fenced block.
fn parse_frontmatter(text: &str) -> (Option<String>, Option<String>) {
    let mut lines = text.lines();
    // Require an opening `---`.
    if lines.next().map(|l| l.trim()) != Some("---") {
        return (None, None);
    }
    let mut name = None;
    let mut desc = None;
    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            break; // end of frontmatter
        }
        if let Some(rest) = trimmed.strip_prefix("name:") {
            name = Some(unquote(rest.trim()));
        } else if let Some(rest) = trimmed.strip_prefix("description:") {
            desc = Some(unquote(rest.trim()));
        }
    }
    (name, desc)
}

/// Strip a single pair of surrounding double-quotes from a YAML scalar, if
/// present. (SKILL.md descriptions are sometimes bare, sometimes quoted.)
fn unquote(s: &str) -> String {
    let s = s.trim();
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}


const CATEGORIES: &[&str] = &["All", "Productivity", "Dev", "Marketing", "Security", "Research"];

pub struct SkillsScreen {
    /// Real on-disk skill library, loaded once at construction (#247).
    skills: Vec<Skill>,
    pub installed: Vec<String>,
    pub selected_idx: usize,
    pub active_category: usize,
    /// Live filter text (JSX `filter` state). Matches against skill name
    /// (case-insensitive substring), combined with the active category.
    pub filter: String,
}

impl Default for SkillsScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl SkillsScreen {
    pub fn new() -> Self {
        let mut skills = load_skills_from_disk();
        if skills.is_empty() {
            skills = fallback_skills();
        }
        // #263: all skills are installed (selected) by default — the onboarding
        // contract is opt-OUT, not opt-in. Seed `installed` with every loaded
        // skill id so a fresh seat lands with the full library on. The persist
        // path reads `self.installed` directly, so this flips the default with
        // no other code change.
        let installed = skills.iter().map(|s| s.id.clone()).collect();
        Self {
            skills,
            installed,
            selected_idx: 0,
            active_category: 0,
            filter: String::new(),
        }
    }

    /// Test-only seam: replace the loaded skill set with a deterministic
    /// fixture so render-fidelity tests don't depend on the host's live
    /// `~/.zeus/skills` contents. `entries` are `(id, name, desc, category,
    /// recommended)`. Not used in production (the real set loads from disk in
    /// `new()`).
    #[doc(hidden)]
    pub fn set_test_skills(&mut self, entries: &[(&str, &str, &str, &str, bool)]) {
        self.skills = entries
            .iter()
            .map(|&(id, name, desc, cat, rec)| Skill {
                id: id.to_string(),
                name: name.to_string(),
                desc: desc.to_string(),
                category: cat.to_string(),
                recommended: rec,
            })
            .collect();
        // #263: mirror `new()`'s opt-OUT default — every (fixture) skill is
        // installed by default. Without this re-seed the test seam would leave
        // `installed` pointing at the real on-disk ids while `skills` holds the
        // fixture, desyncing the "N selected" count from the rendered cards.
        self.installed = self.skills.iter().map(|s| s.id.clone()).collect();
        self.selected_idx = 0;
        self.active_category = 0;
        self.filter.clear();
    }

    /// Skills visible under the active category AND current filter.
    /// Mirrors the JSX `visible` computation (category gate + name substring).
    fn visible_skills(&self) -> Vec<&Skill> {
        let cat = CATEGORIES[self.active_category];
        let needle = self.filter.to_lowercase();
        self.skills
            .iter()
            .filter(|s| cat == "All" || s.category == cat)
            .filter(|s| needle.is_empty() || s.name.to_lowercase().contains(&needle))
            .collect()
    }

    /// Count of skills in a category (ignoring the filter) — for the tab `(N)`
    /// badges. "All" → total flat count.
    fn category_count(&self, cat: &str) -> usize {
        if cat == "All" {
            self.skills.len()
        } else {
            self.skills.iter().filter(|s| s.category == cat).count()
        }
    }

    /// Append a char to the live filter and re-clamp selection into the new
    /// visible set (the filtered list can shrink under the cursor).
    pub fn filter_push(&mut self, c: char) {
        self.filter.push(c);
        self.clamp_selection();
    }

    /// Char-safe backspace on the filter (never byte-slice — pop a codepoint).
    pub fn filter_pop(&mut self) {
        self.filter.pop();
        self.clamp_selection();
    }

    /// Keep `selected_idx` inside the visible set after a filter/category change.
    fn clamp_selection(&mut self) {
        let len = self.visible_skills().len();
        if len == 0 {
            self.selected_idx = 0;
        } else if self.selected_idx >= len {
            self.selected_idx = len - 1;
        }
    }

    pub fn toggle_selected(&mut self) {
        let visible = self.visible_skills();
        if let Some(skill) = visible.get(self.selected_idx) {
            let id = skill.id.to_string();
            if let Some(pos) = self.installed.iter().position(|s| s == &id) {
                self.installed.remove(pos);
            } else {
                self.installed.push(id);
            }
        }
    }

    pub fn move_up(&mut self) {
        let visible = self.visible_skills();
        if !visible.is_empty() {
            self.selected_idx = self.selected_idx.saturating_sub(1);
        }
    }

    pub fn move_down(&mut self) {
        let visible = self.visible_skills();
        if !visible.is_empty() {
            self.selected_idx = (self.selected_idx + 1).min(visible.len() - 1);
        }
    }

    pub fn next_category(&mut self) {
        self.active_category = (self.active_category + 1) % CATEGORIES.len();
        self.selected_idx = 0;
    }

    pub fn prev_category(&mut self) {
        self.active_category = if self.active_category == 0 { CATEGORIES.len() - 1 } else { self.active_category - 1 };
        self.selected_idx = 0;
    }

    /// Number of installed skills (for the "N selected" count).
    pub fn selected_count(&self) -> usize {
        self.installed.len()
    }

    pub fn render(&self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        self.render_with_cursor(area, buf, false);
    }

    /// Render with a blink-gated insertion caret on the filter field.
    /// `cursor_on` is the blink phase (`App::cursor_visible()`); the caret is
    /// only painted when the phase is on AND the filter has real input (never
    /// on the "filter..." placeholder — the placeholder-trap guard, mem 1249).
    pub fn render_with_cursor(&self, area: Rect, buf: &mut ratatui::buffer::Buffer, cursor_on: bool) {
        Clear.render(area, buf);
        // Header
        let header_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2), // title + sub (filter box overlays right)
                Constraint::Length(1), // category tabs + summary
                Constraint::Length(1), // bottom rule under tabs
                Constraint::Min(0),    // skills grid
            ])
            .split(area);

        // Title + sub (left) and the filter input box (right), on the header row.
        let title = Line::from(vec![
            Span::styled("Install starter skills", Style::default().fg(theme::TEXT).add_modifier(Modifier::BOLD)),
        ]);
        let sub = Line::from(vec![
            Span::styled("SKILL.md plugins from the registry. Each grants a set of tools.", Style::default().fg(theme::DIM)),
        ]);
        buf.set_line(header_chunks[0].x, header_chunks[0].y, &title, header_chunks[0].width);
        buf.set_line(header_chunks[0].x, header_chunks[0].y + 1, &sub, header_chunks[0].width);

        // Filter input — JSX renders a `/ filter...` box on the right of the
        // header row (1px muted border). We draw `/ <text-or-placeholder>`
        // right-aligned on the title row.
        let filter_display = if self.filter.is_empty() {
            "filter...".to_string()
        } else {
            self.filter.clone()
        };
        // char-count width (filter text is user input → never byte-slice).
        let filter_inner = format!("/ {} ", filter_display);
        let fbox_w = (filter_inner.chars().count() as u16 + 2).min(header_chunks[0].width);
        let fbox_x = header_chunks[0].x + header_chunks[0].width.saturating_sub(fbox_w);
        let fbox = Rect::new(fbox_x, header_chunks[0].y, fbox_w, 2);
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::MUTED))
            .render(fbox, buf);
        // `/` prefix dim, text fg (or placeholder dim).
        let text_style = if self.filter.is_empty() {
            Style::default().fg(theme::DIM)
        } else {
            Style::default().fg(theme::TEXT)
        };
        let mut fspans = vec![
            Span::styled("/ ", Style::default().fg(theme::DIM)),
            Span::styled(filter_display, text_style),
        ];
        // Blink-gated insertion caret (canonical `▏`, mem 1244). Painted only on
        // the blink-on phase AND when the filter has real input — never on the
        // "filter..." placeholder (placeholder-trap guard, mem 1249).
        if cursor_on && !self.filter.is_empty() {
            fspans.push(Span::styled("\u{258f}", Style::default().fg(theme::ACCENT)));
        }
        let fline = Line::from(fspans);
        buf.set_line(fbox.x + 1, fbox.y, &fline, fbox.width.saturating_sub(2));

        // Category tabs — active = accentFaint bg + accent border-ish + accent
        // text (JSX); inactive = dim. Each non-"All" tab shows `(N)` count.
        let cat_area = header_chunks[1];
        let mut x = cat_area.x;
        for (i, cat) in CATEGORIES.iter().enumerate() {
            let is_active = i == self.active_category;
            let style = if is_active {
                Style::default().fg(theme::ACCENT).bg(theme::ACCENT_FAINT).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme::DIM)
            };
            // "All" shows no count (matches JSX `cat !== "All" && (N)`).
            let label = if *cat == "All" {
                format!(" {} ", cat)
            } else {
                format!(" {} ({}) ", cat, self.category_count(cat))
            };
            let width = label.chars().count() as u16;
            if x + width <= cat_area.x + cat_area.width {
                buf.set_string(x, cat_area.y, &label, style);
                x += width + 1;
            }
        }

        // Right-aligned "N selected · M available" summary on the tab row.
        let n_sel = self.selected_count();
        let m_avail = self.skills.len();
        let summary = format!("{} selected · {} available", n_sel, m_avail);
        let sum_w = summary.chars().count() as u16;
        if sum_w < cat_area.width {
            let sum_x = cat_area.x + cat_area.width - sum_w;
            // Only draw if it doesn't collide with the last tab.
            if sum_x > x {
                let sline = Line::from(vec![
                    Span::styled(format!("{}", n_sel), Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD)),
                    Span::styled(" selected · ".to_string(), Style::default().fg(theme::DIM)),
                    Span::styled(format!("{}", m_avail), Style::default().fg(theme::TEXT)),
                    Span::styled(" available", Style::default().fg(theme::DIM)),
                ]);
                buf.set_line(sum_x, cat_area.y, &sline, sum_w);
            }
        }

        // Bottom border rule under the tab row (JSX `borderBottom: 1px muted`).
        let rule_area = header_chunks[2];
        let rule: String = "─".repeat(rule_area.width as usize);
        buf.set_string(rule_area.x, rule_area.y, &rule, Style::default().fg(theme::MUTED));

        // Skills grid (2 columns)
        let visible = self.visible_skills();
        let body = header_chunks[3];

        // Empty-state when the filter matches nothing in the active category.
        if visible.is_empty() {
            let msg = if self.filter.is_empty() {
                "No skills in this category."
            } else {
                "No skills match your filter."
            };
            buf.set_string(body.x + 1, body.y + 1, msg, Style::default().fg(theme::DIM));
            return;
        }

        let rows = visible.len().div_ceil(2);
        // The JSX card shape includes a name/badge/category row plus a short
        // blurb. The 100×30 gate still has enough body height for that 2-line
        // content, so keep the blurb instead of flattening cards to title-only.
        let shows_blurb = body.height >= 16;
        let row_height = if shows_blurb { 4u16 } else { 3u16 };
        let grid_area = Rect::new(body.x, body.y, body.width, (rows as u16 * row_height).min(body.height));

        let left_col = Rect::new(grid_area.x, grid_area.y, grid_area.width / 2, grid_area.height);
        let right_col = Rect::new(grid_area.x + grid_area.width / 2, grid_area.y, grid_area.width / 2, grid_area.height);

        for (i, skill) in visible.iter().enumerate() {
            let is_selected = i == self.selected_idx;
            let is_installed = self.installed.contains(&skill.id.to_string());
            let col = if i % 2 == 0 { left_col } else { right_col };
            let row = i / 2;
            let card_y = col.y + (row as u16 * row_height);
            if card_y + row_height > col.y + col.height {
                break;
            }
            let card = Rect::new(col.x + 1, card_y, col.width.saturating_sub(2), row_height);

            // Border
            let border_style = if is_selected {
                Style::default().fg(theme::ACCENT)
            } else if is_installed {
                Style::default().fg(theme::ACCENT_DIM)
            } else {
                Style::default().fg(theme::MUTED)
            };
            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(border_style);
            block.render(card, buf);

            // Left accent stripe
            if is_selected {
                buf.set_string(card.x, card.y + 1, "▌", Style::default().fg(theme::ACCENT));
            }

            // Content
            let inner = Rect::new(
                card.x + 2,
                card.y + 1,
                card.width.saturating_sub(4),
                if shows_blurb { 2 } else { 1 },
            );
            let mut spans = vec![];

            // Checkbox
            let check_style = if is_installed {
                Style::default().fg(theme::BG).bg(theme::ACCENT)
            } else {
                Style::default().fg(theme::MUTED)
            };
            spans.push(Span::styled(if is_installed { " ✓ " } else { "   " }, check_style));

            // Name
            spans.push(Span::styled(skill.name.clone(), Style::default().fg(theme::TEXT).add_modifier(Modifier::BOLD)));

            // ★ REC badge
            if skill.recommended {
                spans.push(Span::styled(" ★ REC", Style::default().fg(theme::GREEN).add_modifier(Modifier::BOLD)));
            }

            // Category tag (right-aligned)
            let cat_tag = skill.category.to_uppercase();
            let name_len = skill.name.len() as u16 + if skill.recommended { 6 } else { 0 } + 3;
            let tag_x = inner.x + name_len + 2;
            if tag_x + cat_tag.len() as u16 <= inner.x + inner.width {
                buf.set_string(tag_x, inner.y, &cat_tag, Style::default().fg(theme::MUTED));
            }

            buf.set_line(inner.x, inner.y, &Line::from(spans), inner.width);

            // Description line
            if inner.height > 1 {
                let desc_y = inner.y + 1;
                buf.set_string(inner.x + 5, desc_y, &skill.desc, Style::default().fg(theme::DIM));
            }
        }
    }
}

#[cfg(test)]
mod cursor_tests {
    use super::*;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    /// Render and return only the header row (y==0), where the `/ filter` box
    /// lives. Scoping to row 0 avoids the legitimate `▌` card-accent stripe on
    /// the selected skill card (mem 1244: the stripe is NOT a text caret and
    /// must be left intact — only the filter field gets the blink caret).
    fn filter_row_string(screen: &SkillsScreen, cursor_on: bool) -> String {
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        screen.render_with_cursor(area, &mut buf, cursor_on);
        let mut row = String::new();
        for x in 0..area.width {
            row.push_str(buf[(x, 0)].symbol());
        }
        row
    }

    #[test]
    fn filter_caret_painted_on_blink_phase_with_input() {
        let mut s = SkillsScreen::new();
        s.filter_push('p');
        s.filter_push('d');
        s.filter_push('f');
        let row = filter_row_string(&s, true);
        assert!(
            row.contains('\u{258f}'),
            "expected canonical caret `▏` on the filter row when cursor_on + filter has input; got:\n{row}"
        );
    }

    #[test]
    fn filter_caret_hidden_on_blink_off() {
        let mut s = SkillsScreen::new();
        s.filter_push('p');
        let row = filter_row_string(&s, false);
        assert!(
            !row.contains('\u{258f}'),
            "expected NO caret on the blink-off phase; got:\n{row}"
        );
    }

    #[test]
    fn filter_caret_absent_on_empty_placeholder() {
        // Placeholder-trap guard (mem 1249): the empty filter shows the
        // "filter..." hint, which is NOT an edit position — no caret.
        let s = SkillsScreen::new();
        let row = filter_row_string(&s, true);
        assert!(
            !row.contains('\u{258f}'),
            "expected NO caret on the empty 'filter...' placeholder; got:\n{row}"
        );
    }

    #[test]
    fn filter_field_uses_no_static_block_caret() {
        // Option A: the filter caret is the blink-gated `▏`, never a static
        // `▌`. (The `▌` card-accent stripe on the selected card is a separate,
        // legitimate marker on the grid rows — not on the filter row — so
        // scoping to row 0 isolates the filter field.)
        let mut s = SkillsScreen::new();
        s.filter_push('x');
        let row = filter_row_string(&s, true);
        assert!(
            !row.contains('\u{258c}'),
            "the filter field must not use the static `▌` block caret (Option A); got:\n{row}"
        );
    }
}
