//! Startup-routing tests for the onboarding READ half (Commit 2).
//!
//! `App::new_from_disk()` reads `Config::load()` at startup and sets `onboarding_complete`
//! from `!cfg.needs_onboarding()`. This is the READ half that closes the
//! re-onboard-every-launch loop: Commit 1 *writes* the marker; this *reads* it.
//!
//! These tests mutate the process-global, thread-unsafe `ZEUS_HOME` env var, so
//! they live in their own test binary (separate process) and run serially within
//! it via a shared guard. `Config::load()` respects `ZEUS_HOME`.

use std::sync::Mutex;
use zeus_tui::app::App;

// Serialize ZEUS_HOME mutation across the tests in THIS binary.
static ENV_GUARD: Mutex<()> = Mutex::new(());

/// A fresh install (no config.toml on disk) must START IN ONBOARDING.
#[test]
fn fresh_install_starts_in_onboarding() {
    let _g = ENV_GUARD.lock().unwrap();
    let dir = std::env::temp_dir().join(format!("zeus_onb_fresh_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // SAFETY: serialized by ENV_GUARD; single-threaded within this test binary.
    unsafe {
        std::env::set_var("ZEUS_HOME", &dir);
    }

    let app = App::new_from_disk();
    assert!(
        !app.onboarding_complete,
        "fresh install (no config.toml) must run the wizard"
    );
    assert!(
        !app.existing_config,
        "fresh install must not report an existing config"
    );

    unsafe {
        std::env::remove_var("ZEUS_HOME");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A completed install (config.toml with onboarding_complete=true) must SKIP
/// the wizard and start in the production UI.
#[test]
fn completed_install_skips_onboarding() {
    let _g = ENV_GUARD.lock().unwrap();
    let dir = std::env::temp_dir().join(format!("zeus_onb_done_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // Write a config.toml with the completion marker + a real model.
    std::fs::write(
        dir.join("config.toml"),
        "model = \"anthropic/claude-opus-4-8\"\nonboarding_complete = true\n",
    )
    .unwrap();
    // SAFETY: serialized by ENV_GUARD; single-threaded within this test binary.
    unsafe {
        std::env::set_var("ZEUS_HOME", &dir);
    }

    let app = App::new_from_disk();
    assert!(
        app.onboarding_complete,
        "completed install (marker set) must skip the wizard"
    );
    assert!(
        app.existing_config,
        "a real config.toml on disk must report existing_config"
    );

    unsafe {
        std::env::remove_var("ZEUS_HOME");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Legacy migration: a config.toml with a model set but NO completion marker
/// (predates the marker field) must be treated as done → skip the wizard.
#[test]
fn legacy_model_set_no_marker_skips_onboarding() {
    let _g = ENV_GUARD.lock().unwrap();
    let dir = std::env::temp_dir().join(format!("zeus_onb_legacy_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // Real config with a model but no onboarding_complete marker.
    std::fs::write(
        dir.join("config.toml"),
        "model = \"anthropic/claude-opus-4-8\"\n",
    )
    .unwrap();
    // SAFETY: serialized by ENV_GUARD; single-threaded within this test binary.
    unsafe {
        std::env::set_var("ZEUS_HOME", &dir);
    }

    let app = App::new_from_disk();
    assert!(
        app.onboarding_complete,
        "legacy config (model set, no marker) must be treated as done"
    );

    unsafe {
        std::env::remove_var("ZEUS_HOME");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
