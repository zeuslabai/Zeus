//! Startup-routing tests for the onboarding READ half (Commit 2).
//!
//! `App::new_from_disk()` reads `Config::load()` at startup and sets `onboarding_complete`
//! from `!cfg.needs_onboarding()`. This is the READ half that closes the
//! re-onboard-every-launch loop: Commit 1 *writes* the marker; this *reads* it.
//!
//! These tests mutate the process-global, thread-unsafe `ZEUS_HOME` env var, so
//! they live in their own test binary (separate process) and run serially within
//! it via a shared guard. `Config::load()` respects `ZEUS_HOME`.

use crossterm::event::KeyCode;
use std::sync::Mutex;
use zeus_tui::app::App;

// Serialize ZEUS_HOME mutation across the tests in THIS binary.
static ENV_GUARD: Mutex<()> = Mutex::new(());

/// RAII: holds the serialization lock, sets `ZEUS_HOME`, and restores the
/// previous value on drop — INCLUDING on panic (#334).
///
/// Two failure classes this kills:
/// - **Poison cascade**: `ENV_GUARD.lock().unwrap()` turns ONE real test
///   failure into N spurious `PoisonError` failures in every later test in
///   this binary. Poison-tolerant locking (`into_inner`) matches the pattern
///   already used by onb_walkthrough_106 / onb_universal_advance /
///   onb_complete_1to1 and src/app.rs persist_tests.
/// - **Env leak on panic**: the manual `set_var .. remove_var` pairs skipped
///   the restore when an assert fired mid-test. Drop-based restore is
///   unconditional.
struct ZeusHomeGuard {
    _lock: std::sync::MutexGuard<'static, ()>,
    previous: Option<std::ffi::OsString>,
}

impl ZeusHomeGuard {
    fn set(dir: &std::path::Path) -> Self {
        let lock = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let previous = std::env::var_os("ZEUS_HOME");
        // SAFETY: serialized by ENV_GUARD for the guard's whole lifetime.
        unsafe {
            std::env::set_var("ZEUS_HOME", dir);
        }
        Self {
            _lock: lock,
            previous,
        }
    }
}

impl Drop for ZeusHomeGuard {
    fn drop(&mut self) {
        // SAFETY: still serialized — the lock is released after this runs.
        unsafe {
            match self.previous.take() {
                Some(prev) => std::env::set_var("ZEUS_HOME", prev),
                None => std::env::remove_var("ZEUS_HOME"),
            }
        }
    }
}

const COMPLETE_STEP: usize = 19;

fn step_forward_existing_config(app: &mut App) {
    if app.current_step == 4 {
        // Auth is live-probe gated; this test only verifies config preservation.
        app.current_step += 1;
        app.on_step_enter();
        return;
    }

    let s = app.current_step;
    if matches!(s, 1 | 7 | 9 | 10 | 12 | 16 | 18) {
        app.current_step += 1;
        app.on_step_enter();
    } else {
        app.handle_key(KeyCode::Right);
    }
    if app.current_step == s {
        app.handle_key(KeyCode::Enter);
    }
}

fn press_through_to_complete(app: &mut App) {
    let mut guard = 0;
    while app.current_step < COMPLETE_STEP {
        step_forward_existing_config(app);
        guard += 1;
        assert!(guard < 100, "press-through stalled before Complete");
    }
    assert_eq!(app.current_step, COMPLETE_STEP, "failed to reach Complete");
}

/// A fresh install (no config.toml on disk) must START IN ONBOARDING.
#[test]
fn fresh_install_starts_in_onboarding() {
    let dir = std::env::temp_dir().join(format!("zeus_onb_fresh_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let _env = ZeusHomeGuard::set(&dir);

    let app = App::new_from_disk();
    assert!(
        !app.onboarding_complete,
        "fresh install (no config.toml) must run the wizard"
    );
    assert!(
        !app.existing_config,
        "fresh install must not report an existing config"
    );

    drop(_env);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A completed install (config.toml with onboarding_complete=true) must SKIP
/// the wizard and start in the production UI.
#[test]
fn completed_install_skips_onboarding() {
    let dir = std::env::temp_dir().join(format!("zeus_onb_done_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // Write a config.toml with the completion marker + a real model.
    std::fs::write(
        dir.join("config.toml"),
        "model = \"anthropic/claude-opus-4-8\"\nonboarding_complete = true\n",
    )
    .unwrap();
    let _env = ZeusHomeGuard::set(&dir);

    let app = App::new_from_disk();
    assert!(
        app.onboarding_complete,
        "completed install (marker set) must skip the wizard"
    );
    assert!(
        app.existing_config,
        "a real config.toml on disk must report existing_config"
    );

    drop(_env);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Legacy migration: a config.toml with a model set but NO completion marker
/// (predates the marker field) must be treated as done → skip the wizard.
#[test]
fn legacy_model_set_no_marker_skips_onboarding() {
    let dir = std::env::temp_dir().join(format!("zeus_onb_legacy_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // Real config with a model but no onboarding_complete marker.
    std::fs::write(
        dir.join("config.toml"),
        "model = \"anthropic/claude-opus-4-8\"\n",
    )
    .unwrap();
    let _env = ZeusHomeGuard::set(&dir);

    let app = App::new_from_disk();
    assert!(
        app.onboarding_complete,
        "legacy config (model set, no marker) must be treated as done"
    );

    drop(_env);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn forced_onboarding_press_through_preserves_existing_config_bytes() {
    let dir = tempfile::tempdir().expect("temp ZEUS_HOME");
    let _env = ZeusHomeGuard::set(dir.path());

    {
        let image_store = dirs::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("~"))
            .join(".zeus/images")
            .display()
            .to_string();
        let existing = format!(
            r#"model = "sakana/fugu-ultra"
fallback_models = ["anthropic/claude-opus-4-8"]
workspace = "~/.zeus/workspace"
sessions = "~/.zeus/sessions"
thinking_level = "high"
onboarding_complete = true
enabled_skills = [
    "shell",
    "web_search",
    "notes",
]
persona = "Innovator"

[mnemosyne]
db_path = "~/.zeus/memory.db"
enable_fts = true
max_messages_per_session = 10000
enable_embeddings = false
embedding_dim = 768
ollama_url = "http://localhost:11434"
embedding_model = "nomic-embed-text"
vector_weight = 0.7
text_weight = 0.3
candidate_multiplier = 4
embedding_providers = ["ollama"]
fallback_threshold = 3
enable_session_indexing = true
session_delta_bytes = 100000
session_delta_messages = 50
enable_file_watcher = false
watch_paths = []
extra_memory_paths = []
chunk_overlap_tokens = 80
embed_batch_size = 100
enable_qmd = false
qmd_url = "http://localhost:7720"
qmd_timeout_ms = 3000
qmd_reranker_model = "cross-encoder/ms-marco-MiniLM-L-6-v2"
qmd_bm25_weight = 0.3
qmd_vector_weight = 0.3
qmd_reranker_weight = 0.4
qmd_candidate_multiplier = 4
compaction_fact_check = false
max_memories = 50000
dedup_threshold = 0.85
consolidation_session_limit = 200

[hermes]
default_channels = []
batch_low_priority = false

[nous]
enable_intent = true
enable_learning = true

[talos]
calendar = true
notes = true
reminders = true
contacts = true
browser = true
system = true
network = true

[channels.telegram]
api_id = 0
api_hash = ""
phone = ""
allow_bots = "mentions"

[channels.telegram.accounts]

[channels.discord]
token = ""
allow_bots = "mentions"

[channels.discord.accounts]

[search]
provider = "duckduckgo"
max_results = 5

[gateway]
host = "127.0.0.1"
port = 8877
public_url = ""
enable_channels = true
enable_cron = true
enable_heartbeat = true
enable_api = true
enable_mcp = true
mcp_port = 3002
web_port = 8081
timeout_secs = 3600
reconnect_delay_secs = 5
shutdown_hard_deadline_secs = 60
prevent_sleep = true
max_ws_message_bytes = 1048576
max_webhook_payload_bytes = 262144
max_webhook_message_bytes = 51200
max_inbound_message_len = 50000
enable_agent_processing = true
mentions_only = false
dm_scope = "main"
allow_peer_tagging = false

[gateway.rate_limit]
enabled = true
global_rpm = 120
llm_rpm = 20
burst_size = 10

[session_compaction]
max_context_tokens = 180000
compaction_threshold = 0.800000011920929

[pruning]
enabled = true
max_age_days = 7
max_sessions = 50
max_total_size_mb = 500
check_interval_secs = 3600
dry_run = false

[agent]
name = "zeus-titan"
persona = "Innovator"

[image_gen]
provider = "open_ai"
url = "http://localhost:8888"
default_width = 1024
default_height = 1024
store_path = "{image_store}"

[voice]
provider = "elevenlabs"
enabled = true

[credentials]
"#,
        );
        let config_path = dir.path().join("config.toml");
        std::fs::write(&config_path, existing.as_bytes()).expect("seed existing config");

        // Canonicalize the seed with one load → save round-trip BEFORE taking
        // the `before` snapshot (#334). The byte-identical assert must test the
        // WIZARD's effect, not serde field drift: every new defaulted field on
        // Config (e.g. `[gateway] shutdown_hard_deadline_secs` / `prevent_sleep`
        // from #329/#331) is serialized by save_unchecked, so a hardcoded seed
        // goes stale on the next field add and this test fails deterministically
        // — and its panic then poisoned ENV_GUARD, cascading PoisonError into
        // every later test in this binary and masquerading as a flaky env race.
        {
            let seeded = zeus_core::Config::load().expect("seeded config must load");
            seeded
                .save_unchecked()
                .expect("canonicalizing round-trip must save");
        }
        let before = std::fs::read(&config_path).expect("seeded config bytes");

        let mut app = App::new_from_disk();
        assert!(
            app.onboarding_complete,
            "seeded configured install should load as already onboarded"
        );

        // Mirrors `zeus onboard`: run the wizard despite a configured install.
        // Pressing through unchanged hydrated defaults must be a no-op on disk.
        app.onboarding_complete = false;
        press_through_to_complete(&mut app);
        app.advance_step();

        let after = std::fs::read(&config_path).expect("config after forced rerun");
        assert_eq!(
            after, before,
            "forced onboarding press-through must leave existing config.toml byte-identical"
        );
    }
}
