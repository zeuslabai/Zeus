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
        let mut doc = trimmed.to_string();
        if !doc.ends_with('\n') {
            doc.push('\n');
        }
        return doc;
    }

    format!(
        "# SOUL.md — {agent_name}\n\n_{trimmed}_\n\n## Core Truths\n\n**Be genuinely helpful, not performatively helpful.** Skip filler — just help.\n\n**Have opinions.** You're allowed to disagree, prefer things, find stuff interesting.\n\n**Be resourceful before asking.** Try to figure it out. Read the file. Check the context.\n\n**Earn trust through competence.** Be careful with external actions. Be bold with internal ones.\n\n## Vibe\n\nConcise when needed, thorough when it matters. Not a drone. Not a sycophant. Just good.\n\n## Continuity\n\nEach session, you wake up fresh. The files in your workspace _are_ your memory.\nRead them. Update them. They're how you persist.\n"
    )
}

/// True when an existing SOUL.md is absent or known boilerplate that onboarding
/// may heal by rendering the selected persona.
pub fn soul_is_stub_or_missing(path: &Path) -> bool {
    match fs::read_to_string(path) {
        Err(_) => true,
        Ok(s) => soul_content_is_stub(&s),
    }
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

/// Write a persona SOUL.md only when onboarding owns the write: missing/stub or
/// explicit operator overwrite. Returns true when the file was written.
pub fn write_onboarding_soul(
    path: &Path,
    agent_name: &str,
    agent_soul: &str,
    force: bool,
) -> io::Result<bool> {
    if force || soul_is_stub_or_missing(path) {
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
    fn write_onboarding_soul_heals_stub_but_preserves_custom() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("SOUL.md");

        fs::write(
            &path,
            "# Zeus\n\nYou are zeus-test, an autonomous Zeus agent.\n",
        )
        .unwrap();
        assert!(
            write_onboarding_soul(&path, "zeus-test", "The Specialist — ships fixes", false)
                .unwrap()
        );
        let healed = fs::read_to_string(&path).unwrap();
        assert!(healed.contains("# SOUL.md — zeus-test"));
        assert!(healed.contains("The Specialist — ships fixes"));

        fs::write(&path, "# SOUL.md — Custom\n\nDo not overwrite me.\n").unwrap();
        assert!(!write_onboarding_soul(&path, "zeus-test", "replacement", false).unwrap());
        assert!(
            fs::read_to_string(&path)
                .unwrap()
                .contains("Do not overwrite me")
        );

        assert!(write_onboarding_soul(&path, "zeus-test", "replacement", true).unwrap());
        assert!(fs::read_to_string(&path).unwrap().contains("replacement"));
    }
}
