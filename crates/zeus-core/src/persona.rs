//! Persona frontmatter → first-class struct + routing (GAP #4).
//!
//! Today `parse_frontmatter_field` reads only `name:` and silently drops
//! `tagline/description/category/default_skills/tools/model/effort`. This module
//! makes the full frontmatter first-class via [`Persona`], loads every persona
//! file into a [`PersonaRegistry`], and adds a [`route`](PersonaRegistry::route)
//! that picks the best-matching persona for a task by scoring the task text
//! against each persona's `description`, then applies that persona's
//! `model`/`effort`/`tools` to the run via [`RunProfile`].
//!
//! Parsing is **tolerant**: any missing field is simply `None`/empty, never an
//! error. A persona file with only `name:` still loads.

use std::path::Path;

// ── Persona struct ─────────────────────────────────────────────────────────────

/// A persona parsed from a personality `.md` file's YAML-ish frontmatter.
///
/// All optional fields tolerate absence — a file with only `name:` yields a
/// `Persona` with every other field empty/`None`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Persona {
    /// `name:` — display name, e.g. "The Backend Dev". The only field the legacy
    /// parser read.
    pub name: String,
    /// `tagline:` — one-line hook.
    pub tagline: Option<String>,
    /// `description:` — the routing-relevant "use for X, not for Y" blurb.
    pub description: Option<String>,
    /// `category:` — e.g. "Engineering".
    pub category: Option<String>,
    /// `default_skills:` — list form `[a, b, c]`.
    pub default_skills: Vec<String>,
    /// `tools:` — list form `[read_file, shell, ...]` applied to the run.
    pub tools: Vec<String>,
    /// `model:` — provider/model string applied to the run, e.g.
    /// "anthropic/claude-opus-4-8".
    pub model: Option<String>,
    /// `effort:` — reasoning effort applied to the run (low/medium/high).
    pub effort: Option<String>,
    /// The markdown body after the closing `---` (the persona's prose).
    pub body: String,
}

impl Persona {
    /// Parse a full persona file (`---` frontmatter + body) tolerantly.
    ///
    /// Returns `None` only when there is no parseable `name:` — every persona
    /// needs a name to be selectable. All other fields are best-effort.
    pub fn parse(content: &str) -> Option<Persona> {
        let trimmed = content.trim_start();
        let (frontmatter, body) = split_frontmatter(trimmed);

        let name = frontmatter
            .as_ref()
            .and_then(|fm| scalar(fm, "name"))
            .filter(|s| !s.is_empty())?;

        let fm = frontmatter.unwrap_or_default();

        Some(Persona {
            name,
            tagline: scalar(&fm, "tagline"),
            description: scalar(&fm, "description"),
            category: scalar(&fm, "category"),
            default_skills: list(&fm, "default_skills"),
            tools: list(&fm, "tools"),
            model: scalar(&fm, "model"),
            effort: scalar(&fm, "effort"),
            body: body.trim().to_string(),
        })
    }

    /// Render this persona as a `SOUL.md` document (#296). Onboarding writes the
    /// result to `~/.zeus/workspace/SOUL.md` so the selected archetype's actual
    /// prose becomes the agent's soul — replacing the install-time stub and the
    /// old generic boilerplate. Falls back to the tagline when the body is empty.
    pub fn render_soul(&self) -> String {
        let mut out = format!("# SOUL.md — {}\n\n", self.name);
        if let Some(tag) = self.tagline.as_deref().filter(|t| !t.is_empty()) {
            out.push_str(&format!("_{tag}_\n\n"));
        }
        if !self.body.is_empty() {
            out.push_str(&self.body);
            if !self.body.ends_with('\n') {
                out.push('\n');
            }
        } else if let Some(tag) = self.tagline.as_deref().filter(|t| !t.is_empty()) {
            out.push_str(tag);
            out.push('\n');
        }
        out
    }

    /// The run profile this persona imposes — the fields a router applies to a run.
    pub fn run_profile(&self) -> RunProfile {
        RunProfile {
            persona_name: self.name.clone(),
            model: self.model.clone(),
            effort: self.effort.clone(),
            tools: self.tools.clone(),
        }
    }
}

/// The subset of a [`Persona`] that a router *applies to a run*: which model,
/// how much reasoning effort, and which tools the agent may use.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RunProfile {
    pub persona_name: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub tools: Vec<String>,
}

// ── Frontmatter parsing helpers ────────────────────────────────────────────────

/// Split a leading `--- ... ---` frontmatter block from the body.
/// Returns `(Some(frontmatter), body)` when a well-formed block exists,
/// else `(None, whole_input)`.
fn split_frontmatter(content: &str) -> (Option<String>, &str) {
    if !content.starts_with("---") {
        return (None, content);
    }
    // Find the closing delimiter after the opening one.
    let after_open = &content[3..];
    match after_open.find("\n---") {
        Some(rel) => {
            let frontmatter = after_open[..rel].to_string();
            // Body starts after the closing "---" line.
            let rest = &after_open[rel + 4..]; // skip "\n---"
            let body = rest.strip_prefix('\n').unwrap_or(rest);
            (Some(frontmatter), body)
        }
        // Tolerate a legacy block delimited by a bare "---" with no newline.
        None => match after_open.find("---") {
            Some(rel) => (Some(after_open[..rel].to_string()), &after_open[rel + 3..]),
            None => (None, content),
        },
    }
}

/// Read a scalar `field: value` from frontmatter, trimming quotes. `None` if absent.
fn scalar(frontmatter: &str, field: &str) -> Option<String> {
    let prefix = format!("{field}:");
    for line in frontmatter.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix(&prefix) {
            let value = value.trim().trim_matches('"').trim_matches('\'').trim();
            if value.is_empty() {
                return None;
            }
            return Some(value.to_string());
        }
    }
    None
}

/// Read a list `field: [a, b, c]` (or a single scalar) from frontmatter.
/// Returns an empty vec if absent. Tolerates spaces and quotes.
fn list(frontmatter: &str, field: &str) -> Vec<String> {
    let Some(raw) = scalar_raw(frontmatter, field) else {
        return Vec::new();
    };
    let inner = raw.trim();
    let inner = inner
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(inner);
    inner
        .split(',')
        .map(|s| s.trim().trim_matches('"').trim_matches('\'').trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

/// Like [`scalar`] but keeps brackets/commas intact (for list parsing).
fn scalar_raw(frontmatter: &str, field: &str) -> Option<String> {
    let prefix = format!("{field}:");
    for line in frontmatter.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix(&prefix) {
            let value = value.trim();
            if value.is_empty() {
                return None;
            }
            return Some(value.to_string());
        }
    }
    None
}

// ── Registry + router ──────────────────────────────────────────────────────────

/// All loaded personas, queryable and routable.
#[derive(Debug, Clone, Default)]
pub struct PersonaRegistry {
    pub personas: Vec<Persona>,
}

/// One routing candidate: the matched persona plus its relevance score and the
/// run profile to apply.
#[derive(Debug, Clone)]
pub struct RouteMatch<'a> {
    pub persona: &'a Persona,
    pub score: u32,
    pub profile: RunProfile,
}

impl PersonaRegistry {
    /// Build a registry from already-parsed personas.
    pub fn new(personas: Vec<Persona>) -> Self {
        Self { personas }
    }

    /// Load every `.md` persona file under `dir` (recursively one level into
    /// category subfolders, matching the on-disk `personalities/<cat>/<name>.md`
    /// layout), parsing each tolerantly. Unparseable files are skipped.
    pub fn load_from_dir(dir: &Path) -> std::io::Result<Self> {
        let mut personas = Vec::new();
        Self::collect_dir(dir, &mut personas, 0)?;
        Ok(Self::new(personas))
    }

    fn collect_dir(dir: &Path, out: &mut Vec<Persona>, depth: u8) -> std::io::Result<()> {
        if depth > 2 {
            return Ok(());
        }
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                Self::collect_dir(&path, out, depth + 1)?;
            } else if path.extension().map(|e| e == "md").unwrap_or(false) {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Some(p) = Persona::parse(&content) {
                        out.push(p);
                    }
                }
            }
        }
        Ok(())
    }

    /// Find a persona by exact display name.
    pub fn by_name(&self, name: &str) -> Option<&Persona> {
        self.personas.iter().find(|p| p.name == name)
    }

    /// Tolerant persona lookup (#296). Matches a selection string against a
    /// persona's display name (`The Coordinator`), its slug (`the-coordinator`),
    /// or a slug without the `the-` prefix (`coordinator`) — case-insensitively.
    /// Lets onboarding resolve whatever form the UI/CLI/web stored in
    /// `config.persona` back to the on-disk persona.
    pub fn find(&self, sel: &str) -> Option<&Persona> {
        let want = persona_slug(sel);
        if want.is_empty() {
            return None;
        }
        self.personas.iter().find(|p| {
            let pslug = persona_slug(&p.name);
            pslug == want
                || pslug.strip_prefix("the-") == Some(want.as_str())
                || want.strip_prefix("the-") == Some(pslug.as_str())
        })
    }

    /// **Router.** Pick the persona whose `description` best matches `task`, and
    /// return it together with the [`RunProfile`] to apply (`model`/`effort`/`tools`).
    ///
    /// Scoring: case-insensitive token overlap between the task and the persona's
    /// `description`. Each distinct task token (≥3 chars) that appears in the
    /// description scores 1; a persona with no `description` scores 0. The
    /// highest-scoring persona wins; ties break toward the first declared. Returns
    /// `None` only when no persona scores above zero (caller falls back to default).
    pub fn route(&self, task: &str) -> Option<RouteMatch<'_>> {
        let task_tokens = tokenize(task);
        if task_tokens.is_empty() {
            return None;
        }

        let mut best: Option<RouteMatch> = None;
        for persona in &self.personas {
            let Some(desc) = persona.description.as_deref() else {
                continue;
            };
            let desc_lower = desc.to_lowercase();
            let mut score = 0u32;
            for tok in &task_tokens {
                if desc_lower.contains(tok.as_str()) {
                    score += 1;
                }
            }
            if score == 0 {
                continue;
            }
            let better = match &best {
                Some(b) => score > b.score,
                None => true,
            };
            if better {
                best = Some(RouteMatch {
                    persona,
                    score,
                    profile: persona.run_profile(),
                });
            }
        }
        best
    }
}

/// Lowercase, de-duplicated task tokens of length ≥ 3 — the routing vocabulary.
/// Normalize a persona name/selection to a comparable slug (#296):
/// lowercase, runs of non-alphanumerics collapse to a single `-`, trimmed.
/// `"The Coordinator"`, `"the-coordinator"`, and `"The  Coordinator!"` all
/// map to `"the-coordinator"`.
fn persona_slug(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_dash = false;
    for c in s.trim().chars() {
        if c.is_alphanumeric() {
            for lc in c.to_lowercase() {
                out.push(lc);
            }
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

fn tokenize(text: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for raw in text.split(|c: char| !c.is_alphanumeric()) {
        let tok = raw.to_lowercase();
        if tok.len() >= 3 && seen.insert(tok.clone()) {
            out.push(tok);
        }
    }
    out
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const BACKEND_DEV: &str = r#"---
name: The Backend Dev
tagline: API-shaper, schema-owner, the-load-is-the-spec engineer
description: Use for API design, database schema and migrations, query optimization, service architecture, and production-load/scaling work — especially Rust/Axum services.
category: Engineering
default_skills: [postgres, sqlite, code-review, verify, plan]
tools: [read_file, edit_file, write_file, list_dir, shell, web_fetch]
model: anthropic/claude-opus-4-8
effort: high
---

You build the part nobody sees until it breaks.
"#;

    const POLYGLOT: &str = r#"---
name: The Polyglot
tagline: Frontend-and-everything generalist
description: Use for UI work, frontend components, CSS, React, and cross-language glue. Not for database internals.
category: Engineering
tools: [read_file, edit_file, shell]
model: anthropic/claude-sonnet-4
effort: medium
---

You make the part everyone sees.
"#;

    // Tolerant case: only `name:` present, everything else missing.
    const MINIMAL: &str = "---\nname: The Minimalist\n---\n\nBody only.\n";

    #[test]
    fn parses_all_frontmatter_fields() {
        let p = Persona::parse(BACKEND_DEV).expect("should parse");
        assert_eq!(p.name, "The Backend Dev");
        assert_eq!(
            p.tagline.as_deref(),
            Some("API-shaper, schema-owner, the-load-is-the-spec engineer")
        );
        assert!(p.description.as_deref().unwrap().starts_with("Use for API design"));
        assert_eq!(p.category.as_deref(), Some("Engineering"));
        assert_eq!(
            p.default_skills,
            vec!["postgres", "sqlite", "code-review", "verify", "plan"]
        );
        assert_eq!(
            p.tools,
            vec!["read_file", "edit_file", "write_file", "list_dir", "shell", "web_fetch"]
        );
        assert_eq!(p.model.as_deref(), Some("anthropic/claude-opus-4-8"));
        assert_eq!(p.effort.as_deref(), Some("high"));
        assert_eq!(p.body, "You build the part nobody sees until it breaks.");
    }

    #[test]
    fn tolerates_missing_fields() {
        let p = Persona::parse(MINIMAL).expect("name-only must still parse");
        assert_eq!(p.name, "The Minimalist");
        assert_eq!(p.tagline, None);
        assert_eq!(p.description, None);
        assert_eq!(p.category, None);
        assert!(p.default_skills.is_empty());
        assert!(p.tools.is_empty());
        assert_eq!(p.model, None);
        assert_eq!(p.effort, None);
        assert_eq!(p.body, "Body only.");
    }

    #[test]
    fn no_name_fails_to_parse() {
        let no_name = "---\ntagline: orphan\n---\nbody";
        assert!(Persona::parse(no_name).is_none());
    }

    #[test]
    fn no_frontmatter_fails_to_parse() {
        assert!(Persona::parse("just a plain markdown body").is_none());
    }

    fn registry() -> PersonaRegistry {
        PersonaRegistry::new(vec![
            Persona::parse(BACKEND_DEV).unwrap(),
            Persona::parse(POLYGLOT).unwrap(),
            Persona::parse(MINIMAL).unwrap(),
        ])
    }

    // ── Router: selection ──
    #[test]
    fn routes_backend_task_to_backend_dev() {
        let reg = registry();
        let m = reg
            .route("Design a database schema and migrations for a new API service")
            .expect("should route");
        assert_eq!(m.persona.name, "The Backend Dev");
        assert!(m.score > 0);
    }

    #[test]
    fn routes_frontend_task_to_polyglot() {
        let reg = registry();
        let m = reg
            .route("Build a React frontend component with CSS styling")
            .expect("should route");
        assert_eq!(m.persona.name, "The Polyglot");
    }

    #[test]
    fn picks_higher_scoring_persona() {
        let reg = registry();
        // Mentions both lanes but is database-heavy → backend should win on overlap.
        let m = reg
            .route("optimize the database query and migration schema for the service")
            .expect("should route");
        assert_eq!(m.persona.name, "The Backend Dev");
        // Backend description overlaps several tokens; assert a clear lead.
        assert!(m.score >= 3, "expected strong overlap, got {}", m.score);
    }

    #[test]
    fn no_match_returns_none() {
        let reg = registry();
        // Persona with no description (Minimalist) can never match; unrelated task.
        assert!(reg.route("xyzzy plugh quux frobnicate").is_none());
    }

    #[test]
    fn persona_without_description_is_never_routed() {
        let reg = registry();
        // The Minimalist has no description → must never be the routed match.
        for task in ["body only", "the minimalist himself", "minimal"] {
            if let Some(m) = reg.route(task) {
                assert_ne!(m.persona.name, "The Minimalist");
            }
        }
    }

    // ── Router: application ──
    #[test]
    fn applies_persona_run_profile() {
        let reg = registry();
        let m = reg
            .route("Design an API and database schema with query optimization")
            .expect("should route");
        let profile = &m.profile;
        assert_eq!(profile.persona_name, "The Backend Dev");
        assert_eq!(profile.model.as_deref(), Some("anthropic/claude-opus-4-8"));
        assert_eq!(profile.effort.as_deref(), Some("high"));
        assert_eq!(
            profile.tools,
            vec!["read_file", "edit_file", "write_file", "list_dir", "shell", "web_fetch"]
        );
    }

    #[test]
    fn run_profile_matches_route_match() {
        // The profile on the RouteMatch must equal the persona's own run_profile().
        let reg = registry();
        let m = reg
            .route("frontend React component CSS")
            .expect("should route");
        assert_eq!(m.profile, m.persona.run_profile());
        assert_eq!(m.profile.model.as_deref(), Some("anthropic/claude-sonnet-4"));
        assert_eq!(m.profile.effort.as_deref(), Some("medium"));
    }

    #[test]
    fn by_name_lookup() {
        let reg = registry();
        assert_eq!(reg.by_name("The Polyglot").unwrap().name, "The Polyglot");
        assert!(reg.by_name("Nonexistent").is_none());
    }

    // #296: tolerant persona lookup — display name, slug, and bare forms.
    #[test]
    fn find_resolves_name_slug_and_bare_forms() {
        let reg = registry();
        assert_eq!(reg.find("The Backend Dev").unwrap().name, "The Backend Dev");
        assert_eq!(reg.find("the-backend-dev").unwrap().name, "The Backend Dev");
        assert_eq!(reg.find("THE BACKEND DEV").unwrap().name, "The Backend Dev");
        // Bare slug (no "the-" prefix) resolves to the "the-" persona.
        assert_eq!(reg.find("polyglot").unwrap().name, "The Polyglot");
        assert!(reg.find("").is_none());
        assert!(reg.find("nonexistent-persona").is_none());
    }

    // #296: render_soul produces a SOUL.md doc containing the persona's prose.
    #[test]
    fn render_soul_includes_body_and_heading() {
        let p = Persona::parse(BACKEND_DEV).expect("parse");
        let soul = p.render_soul();
        assert!(soul.starts_with("# SOUL.md — The Backend Dev"));
        assert!(soul.contains("You build the part nobody sees until it breaks."));
        // It must NOT be the generic boilerplate / stub.
        assert!(!soul.contains("Run 'zeus onboard'"));
    }

    #[test]
    fn persona_slug_normalizes() {
        assert_eq!(persona_slug("The Coordinator"), "the-coordinator");
        assert_eq!(persona_slug("  The  Coordinator! "), "the-coordinator");
        assert_eq!(persona_slug("the-coordinator"), "the-coordinator");
        assert_eq!(persona_slug(""), "");
    }
}
