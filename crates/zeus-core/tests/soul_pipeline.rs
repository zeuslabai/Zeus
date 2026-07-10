//! Regression tests for the #326 single-writer SOUL.md pipeline.

use std::fs;
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn deploy_identity_is_identity_only_and_never_writes_soul() {
    let script = fs::read_to_string(repo_root().join("scripts/deploy-identity.sh"))
        .expect("deploy-identity.sh must be readable");

    assert!(
        script.contains("SOUL.md untouched — persona rendering is owned by onboarding (#326)"),
        "operator-visible skip message documents the onboarding-owned SOUL path"
    );
    assert!(
        !script.contains("cat > \"$soul_file\""),
        "deploy-identity must not write SOUL.md"
    );
    assert!(
        !script.contains("SOUL_EOF"),
        "deploy-identity must not carry a SOUL.md heredoc writer"
    );
    assert!(
        !script.contains("local soul_file=\"$zeus_home/workspace/SOUL.md\""),
        "deploy-identity should not own a SOUL.md write target"
    );
}

#[test]
fn install_and_update_document_onboarding_owned_soul() {
    let install = fs::read_to_string(repo_root().join("scripts/install.sh"))
        .expect("install.sh must be readable");
    let update = fs::read_to_string(repo_root().join("scripts/update.sh"))
        .expect("update.sh must be readable");

    assert!(install.contains("SOUL.md stays onboarding-owned"));
    assert!(update.contains("SOUL.md stays onboarding-owned"));
    assert!(!update.contains("AGENTS.md/SOUL.md"));
    assert!(!update.contains("AGENTS.md / SOUL.md / HEARTBEAT.md"));
}
