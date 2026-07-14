//! Shared SOUL.md rendering and overwrite policy.
//!
//! Onboarding is the only persona authority. It may overwrite absent, blank,
//! install-stub, or known stock/sludge SOUL.md files, but must preserve a custom
//! operator-authored SOUL.md.

use std::fs;
use std::io;
use std::path::Path;

/// Render a complete SOUL.md document from a selected persona body/tagline.
pub fn render_soul_md(agent_name: &str, agent_soul: &str) -> String {
    let trimmed = agent_soul.trim();
    if trimmed.trim_start().starts_with("# SOUL.md") {
        return ensure_trailing_newline(trimmed.to_string());
    }

    if is_full_persona_body(trimmed) {
        return ensure_trailing_newline(format!("# SOUL.md — {agent_name}\n\n{trimmed}"));
    }

    render_generic_soul_md(agent_name, trimmed)
}

fn ensure_trailing_newline(mut doc: String) -> String {
    if !doc.ends_with('\n') {
        doc.push('\n');
    }
    doc
}

fn is_full_persona_body(body: &str) -> bool {
    if body.is_empty()
        || body
            .lines()
            .any(|line| line.trim_start().starts_with("Tone:"))
    {
        return false;
    }

    body.lines()
        .any(|line| line.trim_start().starts_with("## "))
        || body.lines().filter(|line| !line.trim().is_empty()).count() > 3
}

fn render_generic_soul_md(agent_name: &str, tagline: &str) -> String {
    format!(
        "# SOUL.md — {agent_name}\n\n_{tagline}_\n\n## Core Truths\n\n**Be genuinely helpful, not performatively helpful.** Skip filler — just help.\n\n**Have opinions.** You're allowed to disagree, prefer things, find stuff interesting.\n\n**Be resourceful before asking.** Try to figure it out. Read the file. Check the context.\n\n**Earn trust through competence.** Be careful with external actions. Be bold with internal ones.\n\n## Vibe\n\nConcise when needed, thorough when it matters. Not a drone. Not a sycophant.\n"
    )
}

/// True when SOUL.md content is blank, install stub, or stock/sludge boilerplate.
pub fn soul_content_is_stub(content: &str) -> bool {
    let t = content.trim();
    if t.is_empty() {
        return true;
    }

    let lower = t.to_ascii_lowercase();
    (t.starts_with("# SOUL.md") && t.contains("Run 'zeus onboard'"))
        || lower.contains("an autonomous zeus agent")
        || lower.contains("a focused, technically sharp zeus ai agent")
}

pub fn soul_is_stub_or_missing(path: &Path) -> bool {
    match fs::read_to_string(path) {
        Ok(s) => soul_content_is_stub(&s),
        Err(e) if e.kind() == io::ErrorKind::NotFound => true,
        Err(_) => false,
    }
}

/// Write a persona SOUL.md only when onboarding owns the write: missing/stub or
/// `force` is explicit. Returns `Ok(true)` when a write occurred.
pub fn write_onboarding_soul(
    path: &Path,
    agent_name: &str,
    agent_soul: &str,
    force: bool,
) -> io::Result<bool> {
    if force || soul_is_stub_or_missing(path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, render_soul_md(agent_name, agent_soul))?;
        Ok(true)
    } else {
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_stub_and_sludge_souls_as_overwritable() {
        assert!(soul_content_is_stub("   \n"));
        assert!(soul_content_is_stub(
            "# SOUL.md — Run 'zeus onboard' to configure\n"
        ));
        assert!(soul_content_is_stub(
            "# Zeus — Soul\n\nYou are zeus-test, an autonomous Zeus agent.\n"
        ));
        assert!(soul_content_is_stub(
            "# SOUL.md — zeus\n\n_A focused, technically sharp Zeus AI agent._\n"
        ));
        assert!(!soul_content_is_stub(
            "# SOUL.md — The Coordinator\n\nYou route work and protect the team.\n"
        ));
    }

    #[test]
    fn render_soul_md_wraps_full_persona_body_without_generic_template() {
        let body = "You are the coordinator — you turn a pile of agents into a team that ships.\n\nLeading your titans\nEvery titan report gets a coordinator reply.\n\nVoice & channel discipline\nTalk like a human teammate.";
        let rendered = render_soul_md("The Coordinator", body);

        assert!(rendered.starts_with("# SOUL.md — The Coordinator\n\nYou are the coordinator —"));
        assert!(rendered.contains("Leading your titans"));
        assert!(rendered.contains("Voice & channel discipline"));
        assert!(!rendered.contains("## Core Truths"));
    }

    #[test]
    fn render_soul_md_keeps_generic_template_for_short_fallback_text() {
        let rendered = render_soul_md("zeus-test", "A focused Zeus agent.");

        assert!(rendered.contains("_A focused Zeus agent._"));
        assert!(rendered.contains("## Core Truths"));
    }

    #[test]
    fn write_onboarding_soul_heals_stub_but_preserves_custom() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("SOUL.md");

        assert!(
            write_onboarding_soul(&path, "zeus-test", "The Specialist — ships fixes", false)
                .unwrap()
        );
        let healed = fs::read_to_string(&path).unwrap();
        assert!(healed.contains("# SOUL.md — zeus-test"));
        assert!(healed.contains("The Specialist — ships fixes"));

        fs::write(&path, "# SOUL.md — Custom\n\nDo not overwrite me.\n").unwrap();
        assert!(!write_onboarding_soul(&path, "zeus-test", "replacement", false).unwrap());
        assert!(fs::read_to_string(&path)
            .unwrap()
            .contains("Do not overwrite me"));

        assert!(write_onboarding_soul(&path, "zeus-test", "replacement", true).unwrap());
        assert!(fs::read_to_string(&path).unwrap().contains("replacement"));
    }
}
