//! Integration tests for zeus-skills: Tier 2 skill loading, parsing, and matching.

use std::path::PathBuf;
use tempfile::tempdir;
use zeus_skills::{
    load_skills_from_dir, load_skills_with_precedence, parse_frontmatter, parse_openclaw_skill,
    resolve_read_when, slugify, SkillSource,
};

// ============================================================================
// parse_frontmatter
// ============================================================================

#[test]
fn parse_frontmatter_extracts_name_and_description() {
    let content = "---\nname: test-skill\ndescription: Does something useful\n---\n# Body\n";
    let (fm, body) = parse_frontmatter(content);
    assert_eq!(fm.get("name").map(|s| s.as_str()), Some("test-skill"));
    assert_eq!(fm.get("description").map(|s| s.as_str()), Some("Does something useful"));
    assert!(body.contains("Body"));
}

#[test]
fn parse_frontmatter_no_delimiters_returns_empty_map() {
    let content = "No frontmatter here.";
    let (fm, body) = parse_frontmatter(content);
    assert!(fm.is_empty());
    assert!(body.contains("No frontmatter"));
}

// ============================================================================
// parse_openclaw_skill
// ============================================================================

#[test]
fn parse_skill_minimal_valid() {
    let content = "---\nname: my-skill\ndescription: A minimal skill\n---\nUse this when needed.\n";
    let skill = parse_openclaw_skill(content, PathBuf::from("/tmp/my-skill/SKILL.md"));
    assert!(skill.is_some());
    let skill = skill.unwrap();
    assert_eq!(skill.name, "my-skill");
    assert!(!skill.description.is_empty());
}

#[test]
fn parse_skill_empty_content_returns_none() {
    let skill = parse_openclaw_skill("", PathBuf::from("/tmp/empty/SKILL.md"));
    assert!(skill.is_none());
}

#[test]
fn parse_skill_body_preserved() {
    let content = "---\nname: body-skill\ndescription: Has body\n---\nThis is body content.\n";
    let skill = parse_openclaw_skill(content, PathBuf::from("/tmp/body-skill/SKILL.md")).unwrap();
    assert!(skill.instructions.contains("body"));
}

#[test]
fn parse_skill_no_panic_on_garbage_input() {
    let _ = parse_openclaw_skill("!!!@@@###$$$", PathBuf::from("/tmp/garbage/SKILL.md"));
}

// ============================================================================
// slugify
// ============================================================================

#[test]
fn slugify_lowercases_and_replaces_spaces() {
    assert_eq!(slugify("My Skill Name"), "my-skill-name");
}

#[test]
fn slugify_already_slug_unchanged() {
    assert_eq!(slugify("my-skill"), "my-skill");
}

#[test]
fn slugify_strips_special_chars() {
    let result = slugify("skill@v2.0!");
    assert!(!result.contains('@'));
    assert!(!result.contains('!'));
}

#[test]
fn slugify_empty_no_panic() {
    let _ = slugify("");
}

// ============================================================================
// resolve_read_when
// ============================================================================

#[test]
fn resolve_read_when_single_phrase() {
    let content = "---\nname: rw-skill\ndescription: test\nread_when: user asks about weather\n---\n";
    let (fm, _) = parse_frontmatter(content);
    let triggers = resolve_read_when(&fm);
    assert!(!triggers.is_empty());
    assert!(triggers[0].contains("weather"));
}

#[test]
fn resolve_read_when_missing_no_panic() {
    let content = "---\nname: no-trigger\ndescription: test\n---\n";
    let (fm, _) = parse_frontmatter(content);
    let _ = resolve_read_when(&fm);
}

// ============================================================================
// load_skills_from_dir
// ============================================================================

#[test]
fn load_skills_from_dir_empty_returns_empty() {
    let dir = tempdir().unwrap();
    let skills = load_skills_from_dir(dir.path(), SkillSource::Bundled);
    assert!(skills.is_empty());
}

#[test]
fn load_skills_from_dir_finds_skill_md() {
    let dir = tempdir().unwrap();
    let skill_dir = dir.path().join("my-skill");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: my-skill\ndescription: A test skill\n---\nBody.\n",
    ).unwrap();

    let skills = load_skills_from_dir(dir.path(), SkillSource::Managed);
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].0.name, "my-skill");
    assert!(matches!(skills[0].1, SkillSource::Managed));
}

#[test]
fn load_skills_from_dir_multiple_skills() {
    let dir = tempdir().unwrap();
    for name in &["alpha", "beta", "gamma"] {
        let skill_dir = dir.path().join(name);
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: Skill {name}\n---\nDoes {name} things.\n"),
        ).unwrap();
    }
    let skills = load_skills_from_dir(dir.path(), SkillSource::Bundled);
    assert_eq!(skills.len(), 3);
}

#[test]
fn load_skills_from_dir_ignores_non_skill_files() {
    let dir = tempdir().unwrap();
    std::fs::write(dir.path().join("README.md"), "Not a skill").unwrap();
    std::fs::write(dir.path().join("config.toml"), "[config]").unwrap();
    let skills = load_skills_from_dir(dir.path(), SkillSource::Bundled);
    assert!(skills.is_empty());
}

#[test]
fn load_skills_from_dir_skips_empty_skill_md() {
    let dir = tempdir().unwrap();
    let skill_dir = dir.path().join("bad-skill");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(skill_dir.join("SKILL.md"), "").unwrap();
    let skills = load_skills_from_dir(dir.path(), SkillSource::Bundled);
    assert!(skills.is_empty());
}

// ============================================================================
// load_skills_with_precedence
// ============================================================================

#[test]
fn load_with_precedence_empty_returns_empty() {
    let managed = tempdir().unwrap();
    let skills = load_skills_with_precedence(None, managed.path(), None, &[]);
    assert!(skills.is_empty());
}

#[test]
fn load_with_precedence_workspace_overrides_managed() {
    let workspace_dir = tempdir().unwrap();
    let managed_dir = tempdir().unwrap();

    for dir in &[workspace_dir.path(), managed_dir.path()] {
        let skill_dir = dir.join("shared-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: shared-skill\ndescription: Shared\n---\nContent.\n",
        ).unwrap();
    }

    let skills = load_skills_with_precedence(
        Some(workspace_dir.path()),
        managed_dir.path(),
        None,
        &[],
    );

    let shared: Vec<_> = skills.iter().filter(|(s, _)| s.name == "shared-skill").collect();
    assert_eq!(shared.len(), 1, "Duplicate skill should be deduplicated");
    assert!(matches!(shared[0].1, SkillSource::Workspace), "Workspace takes precedence over Managed");
}

#[test]
fn load_with_precedence_merges_unique_skills() {
    let workspace_dir = tempdir().unwrap();
    let managed_dir = tempdir().unwrap();

    let skill_dir_a = workspace_dir.path().join("skill-a");
    std::fs::create_dir_all(&skill_dir_a).unwrap();
    std::fs::write(skill_dir_a.join("SKILL.md"), "---\nname: skill-a\ndescription: A\n---\nA.\n").unwrap();

    let skill_dir_b = managed_dir.path().join("skill-b");
    std::fs::create_dir_all(&skill_dir_b).unwrap();
    std::fs::write(skill_dir_b.join("SKILL.md"), "---\nname: skill-b\ndescription: B\n---\nB.\n").unwrap();

    let skills = load_skills_with_precedence(
        Some(workspace_dir.path()),
        managed_dir.path(),
        None,
        &[],
    );

    assert_eq!(skills.len(), 2);
}
