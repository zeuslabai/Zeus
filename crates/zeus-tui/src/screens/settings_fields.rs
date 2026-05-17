//! Settings field definitions — maps every Config struct field to a SettingsEntry.
//! Owner: zeus112 (feat/tui-settings-tab)
//!
//! Each entry has:
//!  - `key`     — dot-path into config.toml (e.g. "gateway.port")
//!  - `label`   — human-readable display name
//!  - `section` — logical grouping for the scrollable list
//!  - `kind`    — field type (Text, Password, Toggle, Select)
//!  - `default` — default value as a string
//!  - `hint`    — optional one-line description shown below the field

#![allow(dead_code)]

/// The type of a settings field — controls how it renders and how input is handled.
#[derive(Debug, Clone, PartialEq)]
pub enum FieldKind {
    /// Free-form text input.
    Text,
    /// Masked text input (show *** while typing).
    Password,
    /// Boolean toggle (true/false, rendered as [ON]/[OFF]).
    Toggle,
    /// Dropdown-style selection from a fixed list of options.
    Select(&'static [&'static str]),
}

/// A single editable settings entry.
#[derive(Debug, Clone)]
pub struct SettingsField {
    /// Dot-path key into config.toml (e.g. "gateway.port", "model").
    pub key: &'static str,
    /// Human-readable label shown in the TUI.
    pub label: &'static str,
    /// Section heading this field belongs to.
    pub section: &'static str,
    /// Input type.
    pub kind: FieldKind,
    /// Default value (as a TOML-serializable string).
    pub default: &'static str,
    /// Short hint shown below the field when focused.
    pub hint: &'static str,
}

/// All settings fields, in display order.
/// Sections: LLM, API Keys, Gateway, Channels, Image Gen, Voice, Security, Features, Memory, Agent
pub const ALL_FIELDS: &[SettingsField] = &[
    // ── LLM Provider ────────────────────────────────────────────────────────
    SettingsField {
        key: "model",
        label: "Model",
        section: "LLM",
        kind: FieldKind::Text, // Free-text — never hardcode model names
        default: "",
        hint: "provider/model-name (e.g. ollama/qwen3, anthropic/claude-sonnet-4-6)",
    },
    SettingsField {
        key: "ollama.url",
        label: "Ollama URL",
        section: "LLM",
        kind: FieldKind::Text,
        default: "http://localhost:11434",
        hint: "Base URL for local Ollama instance",
    },

    // ── API Keys (env vars — stored in ~/.zeus/.env) ─────────────────────────
    SettingsField {
        key: "env.ANTHROPIC_API_KEY",
        label: "Anthropic API Key",
        section: "API Keys",
        kind: FieldKind::Password,
        default: "",
        hint: "Saved to ~/.zeus/.env — never stored in config.toml",
    },
    SettingsField {
        key: "env.OPENAI_API_KEY",
        label: "OpenAI API Key",
        section: "API Keys",
        kind: FieldKind::Password,
        default: "",
        hint: "Saved to ~/.zeus/.env",
    },
    SettingsField {
        key: "env.OPENROUTER_API_KEY",
        label: "OpenRouter API Key",
        section: "API Keys",
        kind: FieldKind::Password,
        default: "",
        hint: "Saved to ~/.zeus/.env",
    },
    SettingsField {
        key: "env.GOOGLE_API_KEY",
        label: "Google API Key",
        section: "API Keys",
        kind: FieldKind::Password,
        default: "",
        hint: "Saved to ~/.zeus/.env",
    },
    SettingsField {
        key: "env.GROQ_API_KEY",
        label: "Groq API Key",
        section: "API Keys",
        kind: FieldKind::Password,
        default: "",
        hint: "Saved to ~/.zeus/.env",
    },
    SettingsField {
        key: "env.MISTRAL_API_KEY",
        label: "Mistral API Key",
        section: "API Keys",
        kind: FieldKind::Password,
        default: "",
        hint: "Saved to ~/.zeus/.env",
    },
    SettingsField {
        key: "auth.use_oauth",
        label: "Use OAuth",
        section: "API Keys",
        kind: FieldKind::Toggle,
        default: "false",
        hint: "Use OAuth token instead of API key (Claude.ai Pro)",
    },

    // ── Gateway ──────────────────────────────────────────────────────────────
    SettingsField {
        key: "gateway.host",
        label: "Gateway Host",
        section: "Gateway",
        kind: FieldKind::Text,
        default: "127.0.0.1",
        hint: "Host the gateway API listens on",
    },
    SettingsField {
        key: "gateway.port",
        label: "Gateway Port",
        section: "Gateway",
        kind: FieldKind::Text,
        default: "8080",
        hint: "Port for the gateway API (TUI connects here)",
    },
    SettingsField {
        key: "gateway.web_port",
        label: "Web UI Port",
        section: "Gateway",
        kind: FieldKind::Text,
        default: "8081",
        hint: "Port for the WebUI / onboarding wizard",
    },
    SettingsField {
        key: "gateway.enable_api",
        label: "Enable API",
        section: "Gateway",
        kind: FieldKind::Toggle,
        default: "true",
        hint: "Expose the REST/SSE API endpoints",
    },
    SettingsField {
        key: "gateway.enable_mcp",
        label: "Enable MCP Server",
        section: "Gateway",
        kind: FieldKind::Toggle,
        default: "true",
        hint: "Enable the Model Context Protocol server",
    },
    SettingsField {
        key: "gateway.enable_channels",
        label: "Enable Channels",
        section: "Gateway",
        kind: FieldKind::Toggle,
        default: "true",
        hint: "Enable Discord, Telegram, Slack, etc.",
    },
    SettingsField {
        key: "gateway.enable_heartbeat",
        label: "Enable Heartbeat",
        section: "Gateway",
        kind: FieldKind::Toggle,
        default: "false",
        hint: "Run proactive heartbeat tasks on a schedule",
    },
    SettingsField {
        key: "gateway.enable_cron",
        label: "Enable Cron",
        section: "Gateway",
        kind: FieldKind::Toggle,
        default: "false",
        hint: "Enable scheduled cron triggers",
    },

    // ── Channels ────────────────────────────────────────────────────────────
    SettingsField {
        key: "env.DISCORD_BOT_TOKEN",
        label: "Discord Bot Token",
        section: "Channels",
        kind: FieldKind::Password,
        default: "",
        hint: "Auto-enables Discord channel when set in ~/.zeus/.env",
    },
    SettingsField {
        key: "env.TELEGRAM_BOT_TOKEN",
        label: "Telegram Bot Token",
        section: "Channels",
        kind: FieldKind::Password,
        default: "",
        hint: "Saved to ~/.zeus/.env — get from @BotFather",
    },
    SettingsField {
        key: "env.TELEGRAM_CHAT_ID",
        label: "Telegram Chat ID",
        section: "Channels",
        kind: FieldKind::Text,
        default: "",
        hint: "Default chat/group ID for Telegram messages",
    },
    SettingsField {
        key: "env.SLACK_BOT_TOKEN",
        label: "Slack Bot Token",
        section: "Channels",
        kind: FieldKind::Password,
        default: "",
        hint: "xoxb-... token from your Slack app settings",
    },
    SettingsField {
        key: "env.SLACK_APP_TOKEN",
        label: "Slack App Token",
        section: "Channels",
        kind: FieldKind::Password,
        default: "",
        hint: "xapp-... token for Socket Mode",
    },

    // ── Image Generation ────────────────────────────────────────────────────
    SettingsField {
        key: "image.backend",
        label: "Image Backend",
        section: "Image Generation",
        kind: FieldKind::Select(&[
            "none",
            "openai",
            "stable-diffusion",
            "comfyui",
        ]),
        default: "none",
        hint: "Backend for image generation tools",
    },
    SettingsField {
        key: "image.sd_url",
        label: "Stable Diffusion URL",
        section: "Image Generation",
        kind: FieldKind::Text,
        default: "http://localhost:7860",
        hint: "Base URL of local SD WebUI / ComfyUI instance",
    },

    // ── Voice / STT / TTS ───────────────────────────────────────────────────
    SettingsField {
        key: "env.ZEUS_STT_PROVIDER",
        label: "STT Provider",
        section: "Voice",
        kind: FieldKind::Select(&[
            "none",
            "whisper",
            "openai",
        ]),
        default: "none",
        hint: "Speech-to-text backend",
    },
    SettingsField {
        key: "env.ZEUS_TTS_PROVIDER",
        label: "TTS Provider",
        section: "Voice",
        kind: FieldKind::Select(&[
            "none",
            "openai",
            "piper",
            "elevenlabs",
        ]),
        default: "none",
        hint: "Text-to-speech backend",
    },
    SettingsField {
        key: "env.ZEUS_WHISPER_URL",
        label: "Whisper URL",
        section: "Voice",
        kind: FieldKind::Text,
        default: "",
        hint: "URL for local Whisper server (if using local STT)",
    },
    SettingsField {
        key: "env.ZEUS_PIPER_URL",
        label: "Piper URL",
        section: "Voice",
        kind: FieldKind::Text,
        default: "",
        hint: "URL for local Piper TTS server",
    },
    SettingsField {
        key: "env.ELEVENLABS_API_KEY",
        label: "ElevenLabs API Key",
        section: "Voice",
        kind: FieldKind::Password,
        default: "",
        hint: "API key for ElevenLabs TTS",
    },

    // ── Security ────────────────────────────────────────────────────────────
    SettingsField {
        key: "env.ZEUS_API_TOKEN",
        label: "Gateway API Token",
        section: "Security",
        kind: FieldKind::Password,
        default: "",
        hint: "Bearer token required to call the gateway API",
    },
    SettingsField {
        key: "security.level",
        label: "Security Level",
        section: "Security",
        kind: FieldKind::Select(&[
            "low",
            "medium",
            "high",
        ]),
        default: "medium",
        hint: "Controls what tools and shell commands agents can run",
    },

    // ── Features / Toggles ──────────────────────────────────────────────────
    SettingsField {
        key: "nous.enable_learning",
        label: "Cognitive Learning",
        section: "Features",
        kind: FieldKind::Toggle,
        default: "true",
        hint: "Enable Nous cognitive engine (proactive learning)",
    },
    SettingsField {
        key: "mcp_server.enable_talos",
        label: "Talos (macOS Tools)",
        section: "Features",
        kind: FieldKind::Toggle,
        default: "true",
        hint: "Enable macOS automation via AppleScript (193 tools)",
    },
    SettingsField {
        key: "mcp_server.enable_agents",
        label: "Agent MCP Tools",
        section: "Features",
        kind: FieldKind::Toggle,
        default: "false",
        hint: "Expose agent management tools via MCP",
    },
    SettingsField {
        key: "mcp_server.enable_mnemosyne",
        label: "Memory MCP Tools",
        section: "Features",
        kind: FieldKind::Toggle,
        default: "true",
        hint: "Expose Mnemosyne memory tools via MCP",
    },
    SettingsField {
        key: "talos.enable_applescript",
        label: "AppleScript Execution",
        section: "Features",
        kind: FieldKind::Toggle,
        default: "true",
        hint: "Allow Talos to run AppleScript on this machine",
    },

    // ── Memory (Mnemosyne) ───────────────────────────────────────────────────
    SettingsField {
        key: "mnemosyne.db_path",
        label: "Memory DB Path",
        section: "Memory",
        kind: FieldKind::Text,
        default: "~/.zeus/memory.db",
        hint: "Path to the SQLite memory database",
    },
    SettingsField {
        key: "mnemosyne.enable_fts",
        label: "Full-Text Search",
        section: "Memory",
        kind: FieldKind::Toggle,
        default: "true",
        hint: "Enable FTS5 full-text search index on memory entries",
    },

    // ── Agent Persona ────────────────────────────────────────────────────────
    SettingsField {
        key: "agent.name",
        label: "Agent Name",
        section: "Agent",
        kind: FieldKind::Text,
        default: "",
        hint: "Display name for this agent (shown in chat and logs)",
    },
    SettingsField {
        key: "agent.persona",
        label: "Agent Persona",
        section: "Agent",
        kind: FieldKind::Text,
        default: "",
        hint: "One-line persona description injected into system prompt",
    },
    SettingsField {
        key: "max_iterations",
        label: "Max Iterations",
        section: "Agent",
        kind: FieldKind::Text,
        default: "20",
        hint: "Max tool-call iterations per agent run",
    },
    SettingsField {
        key: "max_subagent_iterations",
        label: "Max Subagent Iterations",
        section: "Agent",
        kind: FieldKind::Text,
        default: "15",
        hint: "Max iterations for spawned subagents",
    },

    // ── TUI Display ─────────────────────────────────────────────────────────
    SettingsField {
        key: "tui.theme",
        label: "TUI Theme",
        section: "Display",
        kind: FieldKind::Select(&[
            "dark",
            "light",
            "cyberpunk",
        ]),
        default: "dark",
        hint: "Color theme for the terminal UI",
    },
    SettingsField {
        key: "tui.vim_mode",
        label: "Vim Mode",
        section: "Display",
        kind: FieldKind::Toggle,
        default: "false",
        hint: "Enable vim-style keybindings in the TUI",
    },
];

/// Returns all unique section names in display order (deduped, preserving order).
pub fn sections() -> Vec<&'static str> {
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();
    for field in ALL_FIELDS {
        if seen.insert(field.section) {
            result.push(field.section);
        }
    }
    result
}

/// Returns all fields belonging to a given section.
pub fn fields_for_section(section: &str) -> Vec<&'static SettingsField> {
    ALL_FIELDS.iter().filter(|f| f.section == section).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_fields_have_unique_keys() {
        let mut keys = std::collections::HashSet::new();
        for field in ALL_FIELDS {
            assert!(
                keys.insert(field.key),
                "Duplicate key: {}",
                field.key
            );
        }
    }

    #[test]
    fn sections_are_non_empty() {
        let secs = sections();
        assert!(!secs.is_empty());
    }

    #[test]
    fn fields_for_section_returns_correct_subset() {
        let gateway_fields = fields_for_section("Gateway");
        assert!(gateway_fields.iter().all(|f| f.section == "Gateway"));
        assert!(gateway_fields.len() >= 4);
    }

    #[test]
    fn every_field_has_a_label() {
        for field in ALL_FIELDS {
            assert!(!field.label.is_empty(), "Empty label for key: {}", field.key);
        }
    }

    #[test]
    fn password_fields_are_in_right_sections() {
        for field in ALL_FIELDS {
            if matches!(field.kind, FieldKind::Password) {
                assert!(
                    matches!(field.section, "API Keys" | "Channels" | "Security" | "Voice"),
                    "Password field '{}' in unexpected section '{}'",
                    field.key,
                    field.section
                );
            }
        }
    }

    #[test]
    fn toggle_fields_have_boolean_defaults() {
        for field in ALL_FIELDS {
            if matches!(field.kind, FieldKind::Toggle) {
                assert!(
                    field.default == "true" || field.default == "false",
                    "Toggle field '{}' has non-boolean default: '{}'",
                    field.key,
                    field.default
                );
            }
        }
    }

    #[test]
    fn select_fields_have_at_least_one_option() {
        for field in ALL_FIELDS {
            if let FieldKind::Select(opts) = &field.kind {
                assert!(
                    !opts.is_empty(),
                    "Select field '{}' has no options",
                    field.key
                );
            }
        }
    }
}
