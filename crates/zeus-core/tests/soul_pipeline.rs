//! Regression tests for the #338 SOUL.md restore pipeline.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn run_deploy_identity(home: &Path) {
    let output = Command::new("bash")
        .arg(repo_root().join("scripts/deploy-identity.sh"))
        .arg("--home")
        .arg(home)
        .arg("--agent")
        .arg("novaxai1")
        .arg("--force")
        .output()
        .expect("deploy-identity.sh must run");

    assert!(
        output.status.success(),
        "deploy-identity.sh failed\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn deploy_identity_restores_soul_writer_with_stub_policy() {
    let script = fs::read_to_string(repo_root().join("scripts/deploy-identity.sh"))
        .expect("deploy-identity.sh must be readable");

    assert!(script.contains("Writes SOUL.md (personality)"));
    assert!(script.contains("local soul_file=\"$zeus_home/workspace/SOUL.md\""));
    assert!(script.contains("cat > \"$soul_file\" << SOUL_EOF"));
    assert!(script.contains("SOUL.md custom persona preserved (#202)"));
    assert!(script.contains("Run 'zeus onboard'"));
    assert!(script.contains("an autonomous Zeus agent"));
    assert!(script.contains("a focused, technically sharp Zeus AI agent"));
    assert!(
        !script.contains("SOUL.md untouched — persona rendering is owned by onboarding (#326)"),
        "deploy-identity must no longer be identity-only for SOUL.md"
    );
}

#[test]
fn deploy_identity_heals_stub_soul_but_preserves_custom_for_config_agent() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    let workspace = home.join("workspace");
    let personas = home.join("personalities/leadership");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(&personas).unwrap();
    fs::copy(
        repo_root().join("personalities/leadership/the-coordinator.md"),
        personas.join("the-coordinator.md"),
    )
    .unwrap();
    fs::write(
        home.join("config.toml"),
        r#"[agent]
name = "novaxai1"
role = "Specialist"
persona = "The Coordinator"
"#,
    )
    .unwrap();

    let soul = workspace.join("SOUL.md");
    fs::write(
        &soul,
        "# SOUL.md — novaxai1\n\nYou are novaxai1, an autonomous Zeus agent.\n",
    )
    .unwrap();
    run_deploy_identity(home);
    let healed = fs::read_to_string(&soul).unwrap();
    assert!(
        healed.contains("You are the coordinator —"),
        "stub/sludge SOUL.md should be replaced by configured persona; got:\n{healed}"
    );
    assert!(
        healed.contains("Leading your titans"),
        "configured coordinator template sections should be preserved; got:\n{healed}"
    );
    assert!(
        !healed.contains("an autonomous Zeus agent"),
        "legacy fallback sludge should be healed; got:\n{healed}"
    );

    fs::write(&soul, "# SOUL.md — Custom\n\nDo not overwrite me.\n").unwrap();
    run_deploy_identity(home);
    let preserved = fs::read_to_string(&soul).unwrap();
    assert!(
        preserved.contains("Do not overwrite me."),
        "custom SOUL.md must be preserved even during --force identity refresh; got:\n{preserved}"
    );
    assert!(
        !preserved.contains("You are the coordinator —"),
        "custom SOUL.md must not be replaced by configured persona; got:\n{preserved}"
    );
}

#[test]
fn install_and_update_document_full_identity_restore() {
    let install = fs::read_to_string(repo_root().join("scripts/install.sh"))
        .expect("install.sh must be readable");
    let update = fs::read_to_string(repo_root().join("scripts/update.sh"))
        .expect("update.sh must be readable");

    assert!(install.contains("also refresh workspace identity templates"));
    assert!(update.contains("AGENTS.md / SOUL.md / HEARTBEAT.md"));
    assert!(update.contains("Refresh workspace identity"));
    assert!(!install.contains("SOUL.md stays onboarding-owned"));
    assert!(!update.contains("SOUL.md stays onboarding-owned"));
    assert!(!update.contains("Refresh workspace identity docs"));
}
