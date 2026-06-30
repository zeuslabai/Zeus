//! Integration tests for Zeus Config
//!
//! Tests configuration loading, validation, and model parsing.

use tempfile::TempDir;
use zeus_core::Config;

#[test]
fn test_config_default_values() {
    let config = Config::default();

    // Model is intentionally empty by default — forces user to select during onboarding
    assert!(config.model.is_empty());
    assert!(config.workspace.to_string_lossy().contains("zeus"));
    assert!(config.sessions.to_string_lossy().contains("sessions"));
    assert_eq!(config.max_iterations, 20);
}

#[test]
fn test_config_model_parsing() {
    let mut config = Config::default();
    config.model = "anthropic/claude-sonnet-4-6".to_string();
    let (provider, model) = config.parse_model();

    assert!(!format!("{:?}", provider).is_empty());
    assert!(!model.is_empty());
}

#[test]
fn test_config_model_format_validation() {
    let mut config = Config::default();

    // Valid model formats
    config.model = "ollama/llama3.2".to_string();
    let (_provider, model) = config.parse_model();
    assert_eq!(model, "llama3.2");

    config.model = "anthropic/claude-sonnet-4-20250514".to_string();
    let (_provider, model) = config.parse_model();
    assert_eq!(model, "claude-sonnet-4-20250514");

    config.model = "openai/gpt-4o".to_string();
    let (_provider, model) = config.parse_model();
    assert_eq!(model, "gpt-4o");
}

#[test]
fn test_config_workspace_path_creation() {
    let temp = TempDir::new().unwrap();
    let mut config = Config::default();
    config.workspace = temp.path().join("custom_workspace");

    assert!(config.workspace.is_absolute());
}

#[test]
fn test_config_sessions_path_creation() {
    let temp = TempDir::new().unwrap();
    let mut config = Config::default();
    config.sessions = temp.path().join("custom_sessions");

    assert!(config.sessions.is_absolute());
}

#[test]
fn test_config_max_iterations_bounds() {
    let mut config = Config::default();

    config.max_iterations = 1;
    assert_eq!(config.max_iterations, 1);

    config.max_iterations = 100;
    assert_eq!(config.max_iterations, 100);
}

#[test]
fn test_config_serialization() {
    // Default config skips default values (config bloat fix S22)
    let config = Config::default();
    let json = serde_json::to_value(&config).unwrap();
    // Empty model and default max_iterations are skipped
    assert!(json.get("model").is_none(), "empty model should be skipped");
    assert!(json.get("max_iterations").is_none(), "default max_iterations should be skipped");

    // Non-default values should serialize
    let mut config2 = Config::default();
    config2.model = "anthropic/claude-sonnet-4-6".to_string();
    config2.max_iterations = 42;
    let json2 = serde_json::to_value(&config2).unwrap();
    assert!(json2["model"].is_string());
    assert!(json2["max_iterations"].is_number());
}

#[test]
fn test_config_deserialization() {
    let json = serde_json::json!({
        "model": "ollama/llama3.2",
        "max_iterations": 10
    });

    let config: Config = serde_json::from_value(json).unwrap();
    assert_eq!(config.model, "ollama/llama3.2");
    assert_eq!(config.max_iterations, 10);
}

#[test]
fn test_config_tui_settings() {
    let mut config = Config::default();

    config.tui.theme = "light".to_string();
    config.tui.vim_mode = true;

    assert_eq!(config.tui.theme, "light");
    assert!(config.tui.vim_mode);
}

#[test]
fn test_config_provider_detection() {
    let mut config = Config::default();

    // Test Ollama
    config.model = "ollama/llama3.2".to_string();
    let (provider, _) = config.parse_model();
    assert_eq!(format!("{:?}", provider), "Ollama");

    // Test Anthropic
    config.model = "anthropic/claude-sonnet-4".to_string();
    let (provider, _) = config.parse_model();
    assert_eq!(format!("{:?}", provider), "Anthropic");

    // Test OpenAI
    config.model = "openai/gpt-4o".to_string();
    let (provider, _) = config.parse_model();
    assert_eq!(format!("{:?}", provider), "OpenAI");
}

#[test]
fn test_config_suppress_tool_errors() {
    let mut config = Config::default();

    // Default should be false
    assert!(!config.suppress_tool_errors);

    // Can be enabled
    config.suppress_tool_errors = true;
    assert!(config.suppress_tool_errors);
}

#[test]
fn test_config_toml_round_trip() {
    let temp = TempDir::new().unwrap();
    let config_path = temp.path().join("config.toml");

    // Create config
    let mut config = Config::default();
    config.model = "ollama/test-model".to_string();
    config.max_iterations = 15;

    // Serialize to TOML
    let toml_string = toml::to_string(&config).unwrap();
    std::fs::write(&config_path, &toml_string).unwrap();

    // Load from TOML
    let loaded_toml = std::fs::read_to_string(&config_path).unwrap();
    let loaded_config: Config = toml::from_str(&loaded_toml).unwrap();

    assert_eq!(loaded_config.model, "ollama/test-model");
    assert_eq!(loaded_config.max_iterations, 15);
}

#[test]
fn test_config_partial_deserialization() {
    // Config should handle partial data with defaults
    let json = serde_json::json!({
        "model": "ollama/llama3.2"
    });

    let config: Config = serde_json::from_value(json).unwrap();
    assert_eq!(config.model, "ollama/llama3.2");
    assert_eq!(config.max_iterations, 20); // Default value
}

#[test]
fn test_config_subsystem_defaults() {
    let config = Config::default();

    // Optional subsystems should default to None
    assert!(config.mnemosyne.is_none());
    assert!(config.athena.is_none());
    assert!(config.aegis.is_none());
    assert!(config.hermes.is_none());
    assert!(config.nous.is_none());
    assert!(config.talos.is_none());
    assert!(config.channels.is_none());
}

#[test]
fn test_config_thinking_level() {
    let mut config = Config::default();

    // #149: thinking defaults to "high" (was None pre-#149)
    assert_eq!(config.thinking_level, Some("high".to_string()));

    config.thinking_level = Some("extended".to_string());
    assert_eq!(config.thinking_level, Some("extended".to_string()));
}
