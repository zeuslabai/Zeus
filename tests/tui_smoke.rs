//! Smoke tests for Zeus TUI
//!
//! Basic sanity checks for TUI components and functionality.

use tempfile::TempDir;
use zeus_core::{Config, TuiConfig};

#[test]
fn test_tui_config_defaults() {
    let tui = TuiConfig::default();

    assert_eq!(tui.theme, "dark");
    assert!(!tui.vim_mode);
}

#[test]
fn test_tui_config_theme_validation() {
    let mut config = Config::default();

    // Valid themes
    config.tui.theme = "dark".to_string();
    assert_eq!(config.tui.theme, "dark");

    config.tui.theme = "light".to_string();
    assert_eq!(config.tui.theme, "light");
}

#[test]
fn test_tui_config_vim_mode_toggle() {
    let mut config = Config::default();

    assert!(!config.tui.vim_mode);

    config.tui.vim_mode = true;
    assert!(config.tui.vim_mode);

    config.tui.vim_mode = false;
    assert!(!config.tui.vim_mode);
}

#[test]
fn test_tui_config_serialization() {
    let tui = TuiConfig {
        theme: "dark".to_string(),
        vim_mode: true,
        resume_last_session: false,
        disabled_tools: vec![],
    };

    let json = serde_json::to_string(&tui).unwrap();
    let deserialized: TuiConfig = serde_json::from_str(&json).unwrap();

    assert_eq!(deserialized.theme, "dark");
    assert!(deserialized.vim_mode);
}

#[test]
fn test_tui_config_in_main_config() {
    let mut config = Config::default();
    config.tui.theme = "light".to_string();
    config.tui.vim_mode = true;

    let json = serde_json::to_value(&config).unwrap();
    assert_eq!(json["tui"]["theme"], "light");
    assert_eq!(json["tui"]["vim_mode"], true);
}

#[test]
fn test_tui_workspace_path_resolution() {
    let temp = TempDir::new().unwrap();
    let mut config = Config::default();
    config.workspace = temp.path().to_path_buf();

    assert!(config.workspace.exists());
    assert!(config.workspace.is_absolute());
}

#[test]
fn test_tui_sessions_path_resolution() {
    let temp = TempDir::new().unwrap();
    let mut config = Config::default();
    config.sessions = temp.path().join("sessions");

    assert!(config.sessions.is_absolute());
}

#[test]
fn test_tui_model_string_format() {
    let mut config = Config::default();

    // Default model is empty (forces selection during onboarding)
    // When set, it should be in provider/model format
    config.model = "anthropic/claude-sonnet-4-6".to_string();
    assert!(config.model.contains('/'));

    let parts: Vec<&str> = config.model.split('/').collect();
    assert_eq!(parts.len(), 2);
}

#[test]
fn test_tui_max_iterations_bounds() {
    let mut config = Config::default();

    // Should allow reasonable iteration limits
    config.max_iterations = 1;
    assert_eq!(config.max_iterations, 1);

    config.max_iterations = 100;
    assert_eq!(config.max_iterations, 100);
}

#[test]
fn test_tui_config_theme_persistence() {
    let temp = TempDir::new().unwrap();
    let config_path = temp.path().join("config.toml");

    let mut config = Config::default();
    config.tui.theme = "light".to_string();

    let toml = toml::to_string(&config).unwrap();
    std::fs::write(&config_path, toml).unwrap();

    let loaded_toml = std::fs::read_to_string(&config_path).unwrap();
    let loaded_config: Config = toml::from_str(&loaded_toml).unwrap();

    assert_eq!(loaded_config.tui.theme, "light");
}
