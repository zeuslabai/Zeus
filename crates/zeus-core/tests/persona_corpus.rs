//! #215 regression: the persona corpus on disk stays loadable and the six
//! root archetypes live in their category folders (folder == frontmatter
//! category, per personalities/README.md convention).

use std::path::Path;

#[test]
fn registry_loads_full_corpus_including_moved_archetypes() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../personalities");
    if !dir.exists() {
        // Packaged builds may not ship the corpus; only meaningful in-repo.
        return;
    }
    let reg = zeus_core::PersonaRegistry::load_from_dir(&dir).expect("corpus loads");
    let names: Vec<&str> = reg
        .personas
        .iter()
        .map(|p| p.name.as_str())
        .collect();
    for want in [
        "Guardian",
        "Innovator",
        "Mentor",
        "Optimizer",
        "Specialist",
        "Strategist",
    ] {
        assert!(
            names.iter().any(|n| *n == want),
            "archetype `{want}` missing after folder move; loaded: {names:?}"
        );
    }
}

#[test]
fn no_root_level_persona_files_remain() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../personalities");
    if !dir.exists() {
        return;
    }
    let strays: Vec<String> = std::fs::read_dir(&dir)
        .expect("read personalities dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension().map(|e| e == "md").unwrap_or(false)
                && p.file_name().map(|f| f != "README.md").unwrap_or(false)
        })
        .map(|p| p.display().to_string())
        .collect();
    assert!(
        strays.is_empty(),
        "persona files must live in category folders, found at root: {strays:?}"
    );
}

#[test]
fn corpus_has_no_hardcoded_model_pins() {
    for rel in ["../../personalities", "../../workspace/personas"] {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
        if !dir.exists() {
            continue;
        }
        check_no_pins(&dir);
    }
}

fn check_no_pins(dir: &Path) {
    for entry in std::fs::read_dir(dir).expect("read dir").flatten() {
        let path = entry.path();
        if path.is_dir() {
            check_no_pins(&path);
        } else if path.extension().map(|e| e == "md").unwrap_or(false) {
            let content = std::fs::read_to_string(&path).unwrap_or_default();
            assert!(
                !content.lines().any(|l| l.trim_start().starts_with("model:")),
                "{} pins a model — personas inherit the fleet default",
                path.display()
            );
        }
    }
}
