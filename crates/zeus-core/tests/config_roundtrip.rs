//! Config TOML round-trip tests: serialization, deserialization, subsystem configs, validation.

use tempfile::tempdir;
use zeus_core::*;

// ============================================================================
// Basic round-trip
// ============================================================================

#[test]
fn default_config_roundtrips() {
    let config = Config::default();
    let toml_str = toml::to_string_pretty(&config).expect("should serialize");
    let loaded: Config = toml::from_str(&toml_str).expect("should parse successfully");

    assert_eq!(loaded.model, config.model);
    assert_eq!(loaded.max_iterations, config.max_iterations);
    assert_eq!(loaded.tui.theme, config.tui.theme);
    assert_eq!(loaded.tui.vim_mode, config.tui.vim_mode);
    assert_eq!(loaded.onboarding_complete, config.onboarding_complete);
}

#[test]
fn config_with_custom_model_roundtrips() {
    let mut config = Config::default();
    config.model = "openai/gpt-4o".to_string();

    let toml_str = toml::to_string_pretty(&config).expect("should serialize");
    let loaded: Config = toml::from_str(&toml_str).expect("should parse successfully");

    assert_eq!(loaded.model, "openai/gpt-4o");
}

#[test]
fn config_with_modified_tui_roundtrips() {
    let mut config = Config::default();
    config.tui.vim_mode = true;
    config.tui.theme = "light".to_string();

    let toml_str = toml::to_string_pretty(&config).expect("should serialize");
    let loaded: Config = toml::from_str(&toml_str).expect("should parse successfully");

    assert!(loaded.tui.vim_mode);
    assert_eq!(loaded.tui.theme, "light");
}

// ============================================================================
// Subsystem config round-trips
// ============================================================================

#[test]
fn config_with_mnemosyne_roundtrips() {
    let mut config = Config::default();
    config.mnemosyne = Some(MnemosyneConfig {
        db_path: "/tmp/zeus_test.db".into(),
        enable_fts: true,
        enable_embeddings: false,
        ..Default::default()
    });

    let toml_str = toml::to_string_pretty(&config).expect("should serialize");
    let loaded: Config = toml::from_str(&toml_str).expect("should parse successfully");

    let mn = loaded.mnemosyne.expect("operation should succeed");
    assert_eq!(
        mn.db_path.to_str().expect("to_str should succeed"),
        "/tmp/zeus_test.db"
    );
    assert!(mn.enable_fts);
    assert!(!mn.enable_embeddings);
}

#[test]
fn config_with_aegis_roundtrips() {
    let mut config = Config::default();
    config.aegis = Some(AegisConfig {
        sandbox_level: "strict".to_string(),
        require_confirmation_for: vec!["shell".to_string(), "write_file".to_string()],
        ..Default::default()
    });

    let toml_str = toml::to_string_pretty(&config).expect("should serialize");
    let loaded: Config = toml::from_str(&toml_str).expect("should parse successfully");

    let aegis = loaded.aegis.expect("operation should succeed");
    assert_eq!(aegis.sandbox_level, "strict");
    assert_eq!(aegis.require_confirmation_for.len(), 2);
    assert!(
        aegis
            .require_confirmation_for
            .contains(&"shell".to_string())
    );
}

#[test]
fn config_with_prometheus_roundtrips() {
    let mut config = Config::default();
    config.prometheus = Some(PrometheusConfig {
        enable_heartbeat: true,
        heartbeat_interval_secs: 600,
        enable_cognitive: true,
        max_iterations: 30,
        ..Default::default()
    });

    let toml_str = toml::to_string_pretty(&config).expect("should serialize");
    let loaded: Config = toml::from_str(&toml_str).expect("should parse successfully");

    let prom = loaded.prometheus.expect("operation should succeed");
    assert!(prom.enable_heartbeat);
    assert_eq!(prom.heartbeat_interval_secs, 600);
    assert!(prom.enable_cognitive);
    assert_eq!(prom.max_iterations, 30);
}

#[test]
fn config_with_nous_roundtrips() {
    let mut config = Config::default();
    config.nous = Some(NousConfig {
        enable_intent: true,
        enable_learning: true,
    });

    let toml_str = toml::to_string_pretty(&config).expect("should serialize");
    let loaded: Config = toml::from_str(&toml_str).expect("should parse successfully");

    let nous = loaded.nous.expect("operation should succeed");
    assert!(nous.enable_intent);
    assert!(nous.enable_learning);
}

#[test]
fn config_with_talos_roundtrips() {
    let mut config = Config::default();
    config.talos = Some(TalosConfig::default());

    let toml_str = toml::to_string_pretty(&config).expect("should serialize");
    let loaded: Config = toml::from_str(&toml_str).expect("should parse successfully");

    let talos = loaded.talos.expect("operation should succeed");
    assert!(talos.calendar);
    assert!(talos.notes);
    assert!(talos.browser);
}

#[test]
fn config_with_hermes_roundtrips() {
    let mut config = Config::default();
    config.hermes = Some(HermesConfig {
        default_channels: vec!["console".to_string(), "file".to_string()],
        batch_low_priority: true,
    });

    let toml_str = toml::to_string_pretty(&config).expect("should serialize");
    let loaded: Config = toml::from_str(&toml_str).expect("should parse successfully");

    let hermes = loaded.hermes.expect("operation should succeed");
    assert_eq!(hermes.default_channels.len(), 2);
    assert!(hermes.batch_low_priority);
}

#[test]
fn config_with_athena_roundtrips() {
    let mut config = Config::default();
    config.athena = Some(AthenaConfig {
        vault_path: "/tmp/zeus_vault".into(),
    });

    let toml_str = toml::to_string_pretty(&config).expect("should serialize");
    let loaded: Config = toml::from_str(&toml_str).expect("should parse successfully");

    let athena = loaded.athena.expect("operation should succeed");
    assert_eq!(
        athena.vault_path.to_str().expect("to_str should succeed"),
        "/tmp/zeus_vault"
    );
}

// ============================================================================
// Multiple subsystems at once
// ============================================================================

#[test]
fn config_with_all_subsystems_roundtrips() {
    let mut config = Config::default();
    config.model = "anthropic/claude-sonnet-4-20250514".to_string();
    config.max_iterations = 25;
    config.onboarding_complete = true;
    config.tui.vim_mode = true;
    config.mnemosyne = Some(MnemosyneConfig::default());
    config.athena = Some(AthenaConfig::default());
    config.aegis = Some(AegisConfig::default());
    config.hermes = Some(HermesConfig::default());
    config.prometheus = Some(PrometheusConfig::default());
    config.nous = Some(NousConfig::default());
    config.talos = Some(TalosConfig::default());

    let toml_str = toml::to_string_pretty(&config).expect("should serialize");
    let loaded: Config = toml::from_str(&toml_str).expect("should parse successfully");

    assert_eq!(loaded.model, "anthropic/claude-sonnet-4-20250514");
    assert_eq!(loaded.max_iterations, 25);
    assert!(loaded.onboarding_complete);
    assert!(loaded.tui.vim_mode);
    assert!(loaded.mnemosyne.is_some());
    assert!(loaded.athena.is_some());
    assert!(loaded.aegis.is_some());
    assert!(loaded.hermes.is_some());
    assert!(loaded.prometheus.is_some());
    assert!(loaded.nous.is_some());
    assert!(loaded.talos.is_some());
}

// ============================================================================
// File-based round-trip (write to disk, read back)
// ============================================================================

#[test]
fn config_file_roundtrip() {
    let dir = tempdir().expect("should create temp dir");
    let config_path = dir.path().join("config.toml");

    let mut config = Config::default();
    config.model = "google/gemini-2.0-flash".to_string();
    config.max_iterations = 15;
    config.tui.theme = "dracula".to_string();
    config.nous = Some(NousConfig {
        enable_intent: true,
        enable_learning: false,
    });

    // Write
    let toml_str = toml::to_string_pretty(&config).expect("should serialize");
    std::fs::write(&config_path, &toml_str).expect("should write file");

    // Read back
    let loaded = Config::load_from(&config_path).expect("operation should succeed");
    assert_eq!(loaded.model, "google/gemini-2.0-flash");
    assert_eq!(loaded.max_iterations, 15);
    assert_eq!(loaded.tui.theme, "dracula");
    let nous = loaded.nous.expect("operation should succeed");
    assert!(nous.enable_intent);
    assert!(!nous.enable_learning);
}

#[test]
fn config_file_modify_and_reload() {
    let dir = tempdir().expect("should create temp dir");
    let config_path = dir.path().join("config.toml");

    // Write initial
    let config = Config::default();
    let toml_str = toml::to_string_pretty(&config).expect("should serialize");
    std::fs::write(&config_path, &toml_str).expect("should write file");

    // Load, modify, save, reload
    let mut loaded = Config::load_from(&config_path).expect("operation should succeed");
    loaded.model = "ollama/llama3.2".to_string();
    loaded.onboarding_complete = true;

    let toml_str2 = toml::to_string_pretty(&loaded).expect("should serialize");
    std::fs::write(&config_path, &toml_str2).expect("should write file");

    let reloaded = Config::load_from(&config_path).expect("operation should succeed");
    assert_eq!(reloaded.model, "ollama/llama3.2");
    assert!(reloaded.onboarding_complete);
}

// ============================================================================
// parse_model
// ============================================================================

#[test]
fn parse_model_with_provider_prefix() {
    let mut config = Config::default();

    let cases = vec![
        (
            "anthropic/claude-sonnet-4-20250514",
            Provider::Anthropic,
            "claude-sonnet-4-20250514",
        ),
        ("openai/gpt-4o", Provider::OpenAI, "gpt-4o"),
        ("ollama/llama3", Provider::Ollama, "llama3"),
        (
            "google/gemini-2.0-flash",
            Provider::Google,
            "gemini-2.0-flash",
        ),
        ("groq/llama-3.3-70b", Provider::Groq, "llama-3.3-70b"),
        ("mistral/mistral-large", Provider::Mistral, "mistral-large"),
        (
            "together/meta-llama/Llama-3",
            Provider::Together,
            "meta-llama/Llama-3",
        ),
        ("azure/gpt-4o", Provider::Azure, "gpt-4o"),
        (
            "bedrock/anthropic.claude",
            Provider::Bedrock,
            "anthropic.claude",
        ),
    ];

    for (model_str, expected_provider, expected_model) in cases {
        config.model = model_str.to_string();
        let (provider, model) = config.parse_model();
        assert_eq!(
            provider, expected_provider,
            "Provider mismatch for {}",
            model_str
        );
        assert_eq!(model, expected_model, "Model mismatch for {}", model_str);
    }
}

#[test]
fn parse_model_auto_detect() {
    let mut config = Config::default();

    config.model = "claude-sonnet-4-20250514".to_string();
    let (provider, _) = config.parse_model();
    assert_eq!(provider, Provider::Anthropic);

    config.model = "gpt-4o".to_string();
    let (provider, _) = config.parse_model();
    assert_eq!(provider, Provider::OpenAI);

    config.model = "gemini-2.0".to_string();
    let (provider, _) = config.parse_model();
    assert_eq!(provider, Provider::Google);

    config.model = "mimo-7b-pro".to_string();
    let (provider, _) = config.parse_model();
    assert_eq!(provider, Provider::XiaomiMimo);
}

// ============================================================================
// Validation
// ============================================================================

#[test]
fn validate_default_config() {
    let config = Config::default();
    let warnings = config.validate();
    // Default config uses anthropic, which needs API key
    // So we expect at least one warning about ANTHROPIC_API_KEY (unless it's set)
    // The test should not panic
    let _ = warnings;
}

#[test]
fn validate_empty_model_warns() {
    let mut config = Config::default();
    config.model = String::new();

    let warnings = config.validate();
    assert!(
        warnings.iter().any(|w| w.contains("empty")),
        "Should warn about empty model"
    );
}

#[test]
fn validate_no_provider_prefix_warns() {
    let mut config = Config::default();
    config.model = "llama3".to_string();

    let warnings = config.validate();
    assert!(
        warnings.iter().any(|w| w.contains("auto-detect")),
        "Should warn about missing provider prefix"
    );
}

#[test]
fn validate_zero_max_iterations_warns() {
    let mut config = Config::default();
    config.max_iterations = 0;

    let warnings = config.validate();
    assert!(
        warnings.iter().any(|w| w.contains("max_iterations is 0")),
        "Should warn about zero iterations"
    );
}

#[test]
fn validate_high_max_iterations_warns() {
    let mut config = Config::default();
    config.max_iterations = 200;

    let warnings = config.validate();
    assert!(
        warnings.iter().any(|w| w.contains("very high")),
        "Should warn about very high iterations"
    );
}

#[test]
fn validate_ollama_provider_no_api_key_warning() {
    let mut config = Config::default();
    config.model = "ollama/llama3".to_string();

    let warnings = config.validate();
    // Ollama doesn't need an API key — should not have a warning about missing key
    assert!(
        !warnings
            .iter()
            .any(|w| w.contains("OLLAMA") && w.contains("not set")),
        "Ollama should not warn about missing API key"
    );
}

// ============================================================================
// Partial TOML (missing fields use defaults)
// ============================================================================

#[test]
fn partial_toml_uses_defaults() {
    let toml_str = r#"
model = "openai/gpt-4o"
"#;

    let config: Config = toml::from_str(toml_str).expect("should parse successfully");
    assert_eq!(config.model, "openai/gpt-4o");
    assert_eq!(config.max_iterations, 20); // default
    // TuiConfig::default() gives empty theme (serde field default only applies when [tui] section present)
    assert!(!config.tui.vim_mode); // default
    assert!(!config.onboarding_complete); // default: false (fresh installs run onboarding)
    assert!(config.mnemosyne.is_none());
    assert!(config.aegis.is_none());
}

#[test]
fn empty_toml_uses_all_defaults() {
    let config: Config = toml::from_str("").expect("should parse successfully");
    let default = Config::default();

    assert_eq!(config.model, default.model);
    assert_eq!(config.max_iterations, default.max_iterations);
    assert_eq!(config.tui.theme, default.tui.theme);
}

#[test]
fn toml_with_subsystem_section() {
    let toml_str = r#"
model = "anthropic/claude-sonnet-4-20250514"

[prometheus]
enable_heartbeat = true
heartbeat_interval_secs = 120

[nous]
enable_intent = true
enable_learning = true
"#;

    let config: Config = toml::from_str(toml_str).expect("should parse successfully");
    let prom = config.prometheus.expect("operation should succeed");
    assert!(prom.enable_heartbeat);
    assert_eq!(prom.heartbeat_interval_secs, 120);

    let nous = config.nous.expect("operation should succeed");
    assert!(nous.enable_intent);
    assert!(nous.enable_learning);
}

// ============================================================================
// Provider enum round-trip with config
// ============================================================================

#[test]
fn all_providers_parse_correctly() {
    let mut config = Config::default();
    let providers = [
        ("anthropic/model", Provider::Anthropic),
        ("openai/model", Provider::OpenAI),
        ("ollama/model", Provider::Ollama),
        ("openrouter/model", Provider::OpenRouter),
        ("google/model", Provider::Google),
        ("groq/model", Provider::Groq),
        ("mistral/model", Provider::Mistral),
        ("together/model", Provider::Together),
        ("fireworks/model", Provider::Fireworks),
        ("azure/model", Provider::Azure),
        ("bedrock/model", Provider::Bedrock),
        ("xiaomimimo/mimo-7b-pro", Provider::XiaomiMimo),
    ];

    for (model_str, expected) in &providers {
        config.model = model_str.to_string();
        let (provider, _) = config.parse_model();
        assert_eq!(provider, *expected, "Failed for {}", model_str);
    }
}

// ============================================================================
// #89.4: Gateway allow_peer_tagging config
// ============================================================================

#[test]
fn gateway_allow_peer_tagging_default_false() {
    let config = Config::default();
    let gateway = config.gateway.unwrap_or_default();
    assert!(!gateway.allow_peer_tagging, "allow_peer_tagging should default to false");
}

#[test]
fn gateway_allow_peer_tagging_roundtrip_true() {
    let mut config = Config::default();
    config.gateway.get_or_insert_with(Default::default).allow_peer_tagging = true;

    let toml_str = toml::to_string_pretty(&config).expect("should serialize");
    let loaded: Config = toml::from_str(&toml_str).expect("should parse successfully");

    assert!(loaded.gateway.unwrap().allow_peer_tagging);
}

#[test]
fn gateway_allow_peer_tagging_roundtrip_false() {
    let mut config = Config::default();
    config.gateway.get_or_insert_with(Default::default).allow_peer_tagging = false;

    let toml_str = toml::to_string_pretty(&config).expect("should serialize");
    let loaded: Config = toml::from_str(&toml_str).expect("should parse successfully");

    assert!(!loaded.gateway.unwrap().allow_peer_tagging);
}

#[test]
fn gateway_allow_peer_tagging_toml_parse() {
    let toml_str = r#"
model = "anthropic/claude-sonnet-4-6"

[gateway]
allow_peer_tagging = true
"#;
    let config: Config = toml::from_str(toml_str).expect("should parse");
    assert!(config.gateway.unwrap().allow_peer_tagging);
}

#[test]
fn gateway_allow_peer_tagging_absent_defaults_false() {
    let toml_str = r#"
model = "anthropic/claude-sonnet-4-6"

[gateway]
port = 8080
"#;
    let config: Config = toml::from_str(toml_str).expect("should parse");
    assert!(!config.gateway.unwrap().allow_peer_tagging);
}
