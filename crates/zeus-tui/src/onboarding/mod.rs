//! Onboarding — 8-step state machine matching JSX spec.
//! S76: pixel-perfect rewrite from zeus-tui-onboarding.jsx

pub mod render;
mod tests;

use std::collections::HashMap;
use std::path::PathBuf;
use tracing::{info, warn};

// ── Step definitions ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum OnboardingStep {
    Welcome,       // 1. Welcome/Awaken
    SetupMode,     // 2. Setup Mode
    QuickStart,    // 3. QuickStart
    Provider,      // 4. Provider selection
    Auth,          // 5. Auth/API key
    Model,         // 6. Model selection
    Fallback,      // 6b. Backup LLM selection
    Channels,      // 7. Channel selection
    ChanConfig,    // 8. Channel config
    SignalPair,    // 8b. Signal device pairing (QR) — shown only when Signal toggled
    WhatsAppPair,  // 8c. WhatsApp device pairing (QR) — shown only when WhatsApp toggled
    Gateway,       // 9. Gateway setup
    Agent,         // 10. Agent/Persona
    Workspace,     // 11. Workspace setup
    Security,      // 12. Security level
    Features,      // 13. Unlock abilities (subsystem toggles)
    Voice,         // 14. Voice setup
    Images,        // 15. Images setup
    Orchestration, // 15. Orchestration
    Memory,        // 16. Memory setup
    Skills,        // 17. Skills selection
    Complete,      // 18. Complete/Launch
}

impl OnboardingStep {
    pub fn index(&self) -> usize {
        match self {
            Self::Welcome       => 0,
            Self::SetupMode     => 1,
            Self::QuickStart    => 2,
            Self::Provider      => 3,
            Self::Auth          => 4,
            Self::Model         => 5,
            Self::Fallback      => 6,
            Self::Channels      => 7,
            Self::ChanConfig    => 8,
            Self::SignalPair    => 8, // sub-step, shares ChanConfig index
            Self::WhatsAppPair  => 8, // sub-step, shares ChanConfig index
            Self::Gateway       => 9,
            Self::Agent         => 10,
            Self::Workspace     => 11,
            Self::Security      => 12,
            Self::Features      => 13,
            Self::Voice         => 14,
            Self::Images        => 15,
            Self::Orchestration => 16,
            Self::Memory        => 17,
            Self::Skills        => 18,
            Self::Complete      => 19,
        }
    }

    pub fn total() -> usize { 20 }

    pub fn short(&self) -> &'static str {
        match self {
            Self::Welcome       => "WLCM",
            Self::SetupMode     => "MODE",
            Self::QuickStart    => "QCFG",
            Self::Provider      => "PROV",
            Self::Auth          => "AUTH",
            Self::Model         => "MODL",
            Self::Fallback      => "FLBK",
            Self::Channels      => "CHAN",
            Self::ChanConfig    => "CCFG",
            Self::SignalPair    => "SQRP",
            Self::WhatsAppPair  => "WQRP",
            Self::Gateway       => "GTWY",
            Self::Agent         => "AGNT",
            Self::Workspace     => "WKSP",
            Self::Security      => "SECR",
            Self::Features      => "FEAT",
            Self::Voice         => "VOIC",
            Self::Images        => "IMGS",
            Self::Orchestration => "ORCH",
            Self::Memory        => "MNEM",
            Self::Skills        => "SKIL",
            Self::Complete      => "DONE",
        }
    }

    pub fn title(&self) -> &'static str {
        match self {
            Self::Welcome       => "Welcome",
            Self::SetupMode     => "Setup Mode",
            Self::QuickStart    => "QuickStart",
            Self::Provider      => "Provider",
            Self::Auth          => "Auth",
            Self::Model         => "Model",
            Self::Fallback      => "Backup LLMs",
            Self::Channels      => "Channels",
            Self::ChanConfig    => "Chan Config",
            Self::SignalPair    => "Signal Pair",
            Self::WhatsAppPair  => "WhatsApp Pair",
            Self::Gateway       => "Gateway",
            Self::Agent         => "Agent",
            Self::Workspace     => "Workspace",
            Self::Security      => "Security",
            Self::Features      => "Features",
            Self::Voice         => "Voice",
            Self::Images        => "Images",
            Self::Orchestration => "Orchestration",
            Self::Memory        => "Memory",
            Self::Skills        => "Skills",
            Self::Complete      => "Complete",
        }
    }

    pub fn help(&self) -> &'static str {
        match self {
            Self::Welcome       => "Enter=Continue  N=Exit",
            Self::SetupMode     => "↑/↓=Navigate  Enter=Select  Esc=Back",
            Self::QuickStart    => "Tab=Next Field  Enter=Continue  Esc=Back",
            Self::Provider      => "←/→=Browse  Enter=Select  Esc=Back",
            Self::Auth          => "Tab=Switch Mode (Key/Token/Browser)  Enter=Continue  Esc=Back",
            Self::Model         => "↑/↓=Navigate  Enter=Confirm  Esc=Back",
            Self::Fallback      => "↑/↓=Navigate  Space=Toggle  Enter=Continue  Esc=Back",
            Self::Channels      => "Space=Toggle  Enter=Continue  Esc=Back",
            Self::ChanConfig    => "Tab=Next Field  Enter=Continue  Esc=Back",
            Self::SignalPair    => "Enter=Continue (after scan)  Esc=Skip",
            Self::WhatsAppPair  => "Enter=Continue (after scan)  Esc=Skip",
            Self::Gateway       => "Tab=Next Field  Enter=Continue  Esc=Back",
            Self::Agent         => "↑/↓=Navigate  Tab=Custom  Enter=Select  Esc=Back",
            Self::Workspace     => "Enter=Generate  Esc=Back",
            Self::Security      => "↑/↓=Navigate  Enter=Confirm  Esc=Back",
            Self::Features      => "↑/↓=Navigate  Space=Toggle  Enter=Continue  Esc=Back",
            Self::Voice         => "Tab=Next Field  Enter=Continue  Esc=Back",
            Self::Images        => "Tab=Next Field  Enter=Continue  Esc=Back",
            Self::Orchestration => "Tab=Next Field  Enter=Continue  Esc=Back",
            Self::Memory        => "Tab=Next Field  Enter=Continue  Esc=Back",
            Self::Skills        => "Space=Toggle  A=All  N=None  Enter=Install  Esc=Back",
            Self::Complete      => "↑/↓=Navigate  Enter=Launch  Esc=Back",
        }
    }

    pub fn next(&self) -> Option<Self> {
        match self {
            Self::Welcome       => Some(Self::SetupMode),
            Self::SetupMode     => Some(Self::QuickStart),
            Self::QuickStart    => Some(Self::Provider),
            Self::Provider      => Some(Self::Auth),
            Self::Auth          => Some(Self::Model),
            Self::Model         => Some(Self::Fallback),
            Self::Fallback      => Some(Self::Channels),
            Self::Channels      => Some(Self::ChanConfig),
            Self::ChanConfig    => Some(Self::SignalPair),
            Self::SignalPair    => Some(Self::WhatsAppPair),
            Self::WhatsAppPair  => Some(Self::Gateway),
            Self::Gateway       => Some(Self::Agent),
            Self::Agent         => Some(Self::Workspace),
            Self::Workspace     => Some(Self::Security),
            Self::Security      => Some(Self::Features),
            Self::Features      => Some(Self::Voice),
            Self::Voice         => Some(Self::Images),
            Self::Images        => Some(Self::Orchestration),
            Self::Orchestration => Some(Self::Memory),
            Self::Memory        => Some(Self::Skills),
            Self::Skills        => Some(Self::Complete),
            Self::Complete      => None,
        }
    }

    pub fn prev(&self) -> Option<Self> {
        match self {
            Self::Welcome       => None,
            Self::SetupMode     => Some(Self::Welcome),
            Self::QuickStart    => Some(Self::SetupMode),
            Self::Provider      => Some(Self::QuickStart),
            Self::Auth          => Some(Self::Provider),
            Self::Model         => Some(Self::Auth),
            Self::Channels      => Some(Self::Fallback),
            Self::Fallback      => Some(Self::Model),
            Self::ChanConfig    => Some(Self::Channels),
            Self::Gateway       => Some(Self::WhatsAppPair),
            Self::WhatsAppPair  => Some(Self::SignalPair),
            Self::SignalPair    => Some(Self::ChanConfig),
            Self::Agent         => Some(Self::Gateway),
            Self::Workspace     => Some(Self::Agent),
            Self::Security      => Some(Self::Workspace),
            Self::Voice         => Some(Self::Features),
            Self::Features      => Some(Self::Security),
            Self::Images        => Some(Self::Voice),
            Self::Orchestration => Some(Self::Images),
            Self::Memory        => Some(Self::Orchestration),
            Self::Skills        => Some(Self::Memory),
            Self::Complete      => Some(Self::Skills),
        }
    }
}

// ── CLI credential detection ─────────────────────────────────────────────────

/// A credential found in an existing CLI tool's config on disk.
#[derive(Debug, Clone)]
pub struct CliCredential {
    pub provider_name: String, // e.g. "OpenAI"
    pub source: String,        // e.g. "Codex CLI"
    pub token: String,         // raw token value
    pub is_oauth: bool,        // true = Bearer/OAuth token → oauth_token field; false = API key → api_key field
}

impl CliCredential {
    pub fn masked(&self) -> String {
        if self.token.len() <= 8 {
            "•".repeat(self.token.len())
        } else {
            let visible = &self.token[self.token.len() - 4..];
            format!("{}...{}", &"•".repeat(8), visible)
        }
    }
}

/// Detect an existing CLI credential for the given provider.
/// zeus107 implements the file-reading + JSON parsing per provider.
pub fn detect_cli_credential(provider_id: &str) -> Option<CliCredential> {
    let home = dirs::home_dir()?;
    match provider_id {
        "openai" => {
            // Check env var first
            if let Ok(key) = std::env::var("OPENAI_API_KEY") {
                if !key.is_empty() {
                    return Some(CliCredential {
                        provider_name: "OpenAI".to_string(),
                        source: "Environment (OPENAI_API_KEY)".to_string(),
                        token: key,
                        is_oauth: false,
                    });
                }
            }
            // Check Codex CLI auth file
            let path = home.join(".codex").join("auth.json");
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                    // Real API key (sk-...) → use directly via api.openai.com
                    if let Some(key) = val.get("OPENAI_API_KEY").and_then(|v| v.as_str()) {
                        if key.starts_with("sk-") && !key.is_empty() {
                            return Some(CliCredential {
                                provider_name: "OpenAI".to_string(),
                                source: "Codex CLI (API key)".to_string(),
                                token: key.to_string(),
                                is_oauth: false,
                            });
                        }
                    }
                    // ChatGPT OAuth token → route through chatgpt.com/backend-api (Codex backend)
                    if val.get("auth_mode").and_then(|v| v.as_str()) == Some("chatgpt") {
                        if let Some(access) = val.get("tokens")
                            .and_then(|t| t.get("access_token"))
                            .and_then(|v| v.as_str())
                        {
                            if !access.is_empty() {
                                return Some(CliCredential {
                                    provider_name: "OpenAI".to_string(),
                                    source: "Codex CLI (ChatGPT OAuth)".to_string(),
                                    token: access.to_string(),
                                    is_oauth: true,
                                });
                            }
                        }
                    }
                }
            }
            None
        }
        "google" => {
            // Gemini CLI tokens use cloud-platform scope which doesn't work with
            // generativelanguage.googleapis.com (requires generative-language scope).
            // Instead of offering an incompatible token, check if the user has a
            // GOOGLE_API_KEY env var, and if not, return None so onboarding shows
            // the normal auth screen where they can use "Login with Browser" (which
            // requests the correct scope) or paste an API key.
            if let Ok(key) = std::env::var("GOOGLE_API_KEY") {
                if !key.is_empty() {
                    return Some(CliCredential {
                        provider_name: "Google".to_string(),
                        source: "Environment (GOOGLE_API_KEY)".to_string(),
                        token: key,
                        is_oauth: false,
                    });
                }
            }
            // Check if Gemini CLI credentials exist — offer to switch to Gemini CLI provider
            let gemini_cli_path = home.join(".gemini").join("oauth_creds.json");
            if gemini_cli_path.exists() {
                if let Ok(content) = std::fs::read_to_string(&gemini_cli_path) {
                    if let Some(token) = parse_gemini_oauth_token(&content) {
                        return Some(CliCredential {
                            provider_name: "Google".to_string(),
                            source: "Gemini CLI (will switch to Gemini CLI provider)".to_string(),
                            token,
                            is_oauth: true,
                        });
                    }
                }
            }
            None
        }
        "google-gemini-cli" => {
            // Import existing Gemini CLI credentials — they use cloud-platform scope
            // which is exactly what cloudcode-pa.googleapis.com needs.
            let candidates = [
                format!("{}/.gemini/oauth_creds.json", home.display()),
                format!("{}/.config/gemini/oauth_creds.json", home.display()),
                format!("{}/.gemini/credentials.json", home.display()),
                format!("{}/.config/gemini/credentials.json", home.display()),
            ];
            for path in &candidates {
                let p = std::path::Path::new(path);
                if !p.exists() { continue; }
                let content = std::fs::read_to_string(p).ok()?;
                if let Some(token) = parse_gemini_oauth_token(&content) {
                    return Some(CliCredential {
                        provider_name: "Gemini CLI".to_string(),
                        source: format!("Gemini CLI ({})", p.file_name().unwrap_or_default().to_string_lossy()),
                        token,
                        is_oauth: true,
                    });
                }
            }
            None
        }
        "anthropic" => {
            // Zeus credentials stored in ~/.zeus/credentials.json
            let path = home.join(".zeus").join("credentials.json");
            let content = std::fs::read_to_string(&path).ok()?;
            let token = parse_zeus_credentials_token(&content)?;
            Some(CliCredential {
                provider_name: "Anthropic".to_string(),
                source: "Zeus".to_string(),
                is_oauth: token.starts_with("sk-ant-oat01-"), // OAuth setup token vs API key
                token,
            })
        }
        _ => None,
    }
}

// ── zeus107: implement these three parsers ───────────────────────────────────
// Each reads raw JSON and returns the API token string, or None if not found.

fn parse_codex_auth_token(json: &str) -> Option<String> {
    // Format: {"token": "sk-..."} or {"api_key": "sk-..."} or plain text
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(json) {
        let token = val.get("token")
            .or_else(|| val.get("api_key"))
            .or_else(|| val.get("tokens").and_then(|t| t.get("access_token")))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        if let Some(t) = token {
            if !t.is_empty() { return Some(t); }
        }
    }
    // Fallback: plain text key
    let trimmed = json.trim().to_string();
    if !trimmed.is_empty() { Some(trimmed) } else { None }
}

fn parse_gemini_oauth_token(json: &str) -> Option<String> {
    // Format: {"access_token": "ya29....", "refresh_token": "1//...", "expiry_date": 1234567890}
    let val = serde_json::from_str::<serde_json::Value>(json).ok()?;
    let access_token = val.get("access_token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())?;
    if access_token.is_empty() { return None; }

    // Check if token is expired — expiry_date is epoch millis
    let expired = val.get("expiry_date")
        .and_then(|v| v.as_u64())
        .map(|exp_ms| {
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            now_ms > exp_ms
        })
        .unwrap_or(false);

    if !expired {
        return Some(access_token);
    }

    // Token expired — try refresh using Gemini CLI's hardcoded client credentials
    let refresh_token = val.get("refresh_token").and_then(|v| v.as_str())?;
    let client = reqwest::blocking::Client::new();
    let resp = client.post("https://oauth2.googleapis.com/token")
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", "681255809395-oo8ft2oprdrnp9e3aqf6av3hmdib135j.apps.googleusercontent.com"),
            ("client_secret", "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"),
        ])
        .send()
        .ok()?;

    if !resp.status().is_success() { return Some(access_token); } // fallback to stale token

    let body: serde_json::Value = resp.json().ok()?;
    body.get("access_token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or(Some(access_token))
}

fn parse_zeus_credentials_token(json: &str) -> Option<String> {
    // Format: CredentialStore — {"credentials": {"anthropic": {"token": "sk-ant-..."}}}
    let val = serde_json::from_str::<serde_json::Value>(json).ok()?;
    let token = val.get("credentials")
        .and_then(|c| c.get("anthropic"))
        .and_then(|a| a.get("token"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())?;
    if token.is_empty() { None } else { Some(token) }
}

// ── Provider data (matches JSX PROVIDERS) ────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Provider {
    pub name: &'static str,
    pub tag: &'static str,
    pub env_var: &'static str,
    pub provider_id: &'static str,
    pub detected: bool,
    pub models: &'static [&'static str],
}

// Provider models are empty — always fetched from API at runtime.
// No hardcoded model names. If API fetch fails, user sees "No models available".
pub static PROVIDERS: &[Provider] = &[
    Provider { name: "Anthropic", tag: "Claude", env_var: "ANTHROPIC_API_KEY",  provider_id: "anthropic",  detected: false, models: &[] },
    Provider { name: "OpenAI",    tag: "GPT",    env_var: "OPENAI_API_KEY",     provider_id: "openai",     detected: false, models: &[] },
    Provider { name: "Google",    tag: "Gemini API", env_var: "GOOGLE_API_KEY",     provider_id: "google",     detected: false, models: &[] },
    Provider { name: "Ollama",    tag: "Local",  env_var: "OLLAMA_HOST",        provider_id: "ollama",     detected: false, models: &[] },
    Provider { name: "Gemini CLI", tag: "Code Assist", env_var: "",               provider_id: "google-gemini-cli", detected: false, models: &[] },
    Provider { name: "Kimi",       tag: "Moonshot",    env_var: "MOONSHOT_API_KEY",  provider_id: "moonshot",   detected: false, models: &[] },
    Provider { name: "GLM",        tag: "ZAI",         env_var: "ZAI_API_KEY",    provider_id: "zai",               detected: false, models: &[] },
    Provider { name: "Qwen",       tag: "Alibaba",     env_var: "QWEN_API_KEY",   provider_id: "qwen",              detected: false, models: &[] },
    Provider { name: "MiniMax",    tag: "Portal OAuth", env_var: "MINIMAX_API_KEY", provider_id: "minimax",            detected: false, models: &[] },
    Provider { name: "MiMo",       tag: "Xiaomi",      env_var: "XIAOMIMIMO_API_KEY", provider_id: "xiaomimimo",        detected: false, models: &[] },
];

/// Map a provider_id to the appropriate zeus-auth OAuthProvider for browser login.
/// Returns None for providers that don't support OAuth (e.g. Ollama).
fn oauth_provider_for(provider_id: &str) -> Option<zeus_auth::OAuthProvider> {
    match provider_id {
        "anthropic" => Some(zeus_auth::OAuthProvider::anthropic("9d1c250a-e61b-44b0-b5e0-4e85fbb11600")),
        "openai" => Some(zeus_auth::OAuthProvider::openai("app_EMoamEEZ73f0CkXaXp7hrann")),
        "google" => Some(zeus_auth::OAuthProvider::google("zeus-gemini-cli")),
        "google-gemini-cli" => {
            // Extract client_id dynamically from installed Gemini CLI binary
            Some(zeus_auth::OAuthProvider::google_gemini_cli())
        }
        _ => None,
    }
}

// ── Channel data (matches JSX CHANNELS) ──────────────────────────────────────

pub struct ChannelDef {
    pub name: &'static str,
    pub desc: &'static str,
    pub icon: &'static str,
    /// When true: card is shown but greyed-out + non-toggleable.
    /// Integration pending operator-clarify on transport wiring.
    pub coming_soon: bool,
}

pub static CHANNELS: &[ChannelDef] = &[
    ChannelDef { name: "Discord",     desc: "Bot via gateway",         icon: "DC", coming_soon: false },
    ChannelDef { name: "Telegram",    desc: "Bot via BotFather",       icon: "TG", coming_soon: false },
    ChannelDef { name: "IRC",         desc: "IRC server",              icon: "IR", coming_soon: false },
    ChannelDef { name: "Signal",      desc: "signal-cli",              icon: "SI", coming_soon: false },
    ChannelDef { name: "X/Twitter",   desc: "X API v2 Bearer",         icon: "XT", coming_soon: false },
    ChannelDef { name: "Pantheon",    desc: "Zeus IRC server",         icon: "PN", coming_soon: true  },
    ChannelDef { name: "WhatsApp",    desc: "Baileys bridge (QR)",     icon: "WA", coming_soon: true  },
    ChannelDef { name: "Matrix",      desc: "Matrix homeserver",       icon: "MX", coming_soon: true  },
    ChannelDef { name: "Slack",       desc: "Slack bot + Socket Mode", icon: "SL", coming_soon: true  },
    ChannelDef { name: "Email",       desc: "SMTP/IMAP relay",         icon: "EM", coming_soon: true  },
    ChannelDef { name: "MQTT",        desc: "MQTT broker pub/sub",     icon: "MQ", coming_soon: true  },
    ChannelDef { name: "Mattermost",  desc: "Self-hosted chat",        icon: "MM", coming_soon: true  },
];

// ── Persona data ─────────────────────────────────────────────────────────────

pub struct PersonaCat {
    pub cat: String,
    pub items: Vec<String>,
}

/// Load personalities from a `personalities/` folder on disk, falling back to defaults.
/// Searches the current directory, `~/Zeus/personalities`, and `~/zeus/personalities`.
/// Each subfolder is a category; each `.md` file's frontmatter `name:` field becomes an item.
pub fn load_personalities() -> Vec<PersonaCat> {
    let candidates = [
        std::env::current_dir().ok().map(|p| p.join("personalities")),
        dirs::home_dir().map(|p| p.join("Zeus").join("personalities")),
        dirs::home_dir().map(|p| p.join("zeus").join("personalities")),
    ];

    for candidate in candidates.iter().flatten() {
        if candidate.is_dir() {
            if let Ok(cats) = read_personalities_dir(candidate) {
                if !cats.is_empty() {
                    return cats;
                }
            }
        }
    }

    default_personas()
}

fn read_personalities_dir(dir: &std::path::Path) -> std::io::Result<Vec<PersonaCat>> {
    let mut categories: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();

    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            let cat_name = entry.file_name().to_string_lossy().to_string();
            // Capitalize category name
            let cat_display = cat_name
                .chars()
                .next()
                .map(|c| c.to_uppercase().to_string())
                .unwrap_or_default()
                + &cat_name[1..];

            for file in std::fs::read_dir(&path)? {
                let file = file?;
                let file_path = file.path();
                if file_path.extension().map(|e| e == "md").unwrap_or(false) {
                    if let Ok(content) = std::fs::read_to_string(&file_path) {
                        if let Some(name) = parse_frontmatter_field(&content, "name") {
                            categories.entry(cat_display.clone()).or_default().push(name);
                        }
                    }
                }
            }
        }
    }

    let mut result: Vec<PersonaCat> = categories
        .into_iter()
        .map(|(cat, items)| PersonaCat { cat, items })
        .collect();
    result.sort_by(|a, b| a.cat.cmp(&b.cat));
    Ok(result)
}

fn parse_frontmatter_field(content: &str, field: &str) -> Option<String> {
    let content = content.trim();
    if !content.starts_with("---") {
        return None;
    }
    let end = content[3..].find("---")?;
    let frontmatter = &content[3..3 + end];
    for line in frontmatter.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix(&format!("{}:", field)) {
            return Some(value.trim().trim_matches('"').to_string());
        }
    }
    None
}

fn default_personas() -> Vec<PersonaCat> {
    vec![
        PersonaCat { cat: "Leadership".into(),      items: vec!["The Coordinator".into()] },
        PersonaCat { cat: "Engineering".into(),      items: vec!["The Architect".into(), "The Builder".into(), "The Operator".into(), "The Polyglot".into(), "The Crafter".into()] },
        PersonaCat { cat: "DevOps & Infra".into(),   items: vec!["The Plumber".into(), "The Sentinel".into()] },
        PersonaCat { cat: "Data & Finance".into(),   items: vec!["The Oracle".into(), "The Trader".into(), "The Market Analyst".into(), "The Scholar".into()] },
        PersonaCat { cat: "Product & Business".into(), items: vec!["The Analyst".into(), "The Partner".into(), "The Minimalist".into()] },
        PersonaCat { cat: "Creative & Design".into(), items: vec!["The Herald".into(), "The Spark".into(), "The Visionary".into(), "The Amplifier".into()] },
    ]
}

// ── Skills data ──────────────────────────────────────────────────────────────

pub struct SkillItem {
    pub name: String,
    pub desc: String,
    pub default: bool,
}

pub struct SkillCat {
    pub cat: String,
    pub items: Vec<SkillItem>,
}

/// Load skills from `~/.zeus/skills/` directory, falling back to defaults.
/// Each subfolder containing a `SKILL.md` with frontmatter `description:` becomes a skill item.
/// Skills are grouped into categories based on known name prefixes.
pub fn load_skills() -> Vec<SkillCat> {
    let skills_dir = dirs::home_dir().map(|p| p.join(".zeus").join("skills"));

    if let Some(ref dir) = skills_dir {
        if dir.is_dir() {
            if let Ok(cats) = read_skills_dir(dir) {
                if !cats.is_empty() {
                    return cats;
                }
            }
        }
    }

    default_skills()
}

fn read_skills_dir(dir: &std::path::Path) -> std::io::Result<Vec<SkillCat>> {
    let mut skills: Vec<(String, String)> = Vec::new();

    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            let skill_md = path.join("SKILL.md");
            if skill_md.exists() {
                if let Ok(content) = std::fs::read_to_string(&skill_md) {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let desc = parse_frontmatter_field(&content, "description")
                        .unwrap_or_else(|| "No description".to_string());
                    skills.push((name, desc));
                }
            }
        }
    }

    let mut doc_items = Vec::new();
    let mut design_items = Vec::new();
    let mut dev_items = Vec::new();
    let mut knowledge_items = Vec::new();
    let mut other_items = Vec::new();

    for (name, desc) in skills {
        let item = SkillItem { name: name.clone(), desc, default: true };
        match name.as_str() {
            "pdf" | "docx" | "pptx" | "xlsx" | "doc-coauthoring" => doc_items.push(item),
            "frontend-design" | "canvas-design" | "algorithmic-art" | "theme-factory" | "web-artifacts-builder" => design_items.push(item),
            "claude-deep-research" | "obsidian-skills" => knowledge_items.push(item),
            "superpowers" | "test-driven-development" | "systematic-debugging"
            | "verification-before-completion" | "brainstorming" | "writing-plans"
            | "executing-plans" | "skill-creator" | "writing-skills"
            | "subagent-driven-development" | "dispatching-parallel-agents"
            | "requesting-code-review" | "receiving-code-review" | "using-git-worktrees"
            | "finishing-a-development-branch" | "using-superpowers"
            | "brand-guidelines" => dev_items.push(item),
            _ => other_items.push(item),
        }
    }

    let mut cats = Vec::new();
    if !doc_items.is_empty() {
        cats.push(SkillCat { cat: "Document & Office".into(), items: doc_items });
    }
    if !design_items.is_empty() {
        cats.push(SkillCat { cat: "Design & Creative".into(), items: design_items });
    }
    if !dev_items.is_empty() {
        cats.push(SkillCat { cat: "Dev & Engineering".into(), items: dev_items });
    }
    if !knowledge_items.is_empty() {
        cats.push(SkillCat { cat: "Knowledge & Research".into(), items: knowledge_items });
    }
    if !other_items.is_empty() {
        cats.push(SkillCat { cat: "Other".into(), items: other_items });
    }

    Ok(cats)
}

fn default_skills() -> Vec<SkillCat> {
    vec![
        SkillCat { cat: "Document & Office".into(), items: vec![
            SkillItem { name: "pdf".into(),  desc: "Read, extract, merge, fill PDF forms".into(),        default: true  },
            SkillItem { name: "docx".into(), desc: "Word docs with formatting + tracked changes".into(), default: true  },
            SkillItem { name: "pptx".into(), desc: "Slide decks from natural language".into(),           default: false },
            SkillItem { name: "xlsx".into(), desc: "Spreadsheet formulas + charts".into(),               default: false },
        ]},
        SkillCat { cat: "Design & Creative".into(), items: vec![
            SkillItem { name: "frontend-design".into(), desc: "UI components, bold typography".into(),       default: true  },
            SkillItem { name: "canvas-design".into(),   desc: "Social graphics, posters -> PNG/PDF".into(),  default: false },
            SkillItem { name: "algorithmic-art".into(), desc: "Fractal patterns via p5.js".into(),           default: false },
            SkillItem { name: "theme-factory".into(),   desc: "Batch color schemes from one prompt".into(),  default: false },
        ]},
        SkillCat { cat: "Dev & Engineering".into(), items: vec![
            SkillItem { name: "skill-creator".into(),    desc: "Meta-skill: describe a workflow, get a SKILL.md".into(), default: false },
            SkillItem { name: "brand-guidelines".into(), desc: "Encode your brand into a skill".into(),                  default: false },
        ]},
        SkillCat { cat: "Knowledge & Research".into(), items: vec![
            SkillItem { name: "claude-deep-research".into(), desc: "8-phase research with auto-continuation".into(), default: false },
            SkillItem { name: "obsidian-skills".into(),      desc: "Obsidian vault auto-tagging + linking".into(),   default: false },
        ]},
    ]
}

// ── Security levels ───────────────────────────────────────────────────────────

pub struct SecurityLevel {
    pub name: &'static str,
    pub desc: &'static str,
}

pub static SECURITY_LEVELS: &[SecurityLevel] = &[
    SecurityLevel { name: "Minimal",  desc: "No restrictions -- development mode" },
    SecurityLevel { name: "Standard", desc: "Basic path/command filtering"         },
    SecurityLevel { name: "Strict",   desc: "Full Seatbelt sandboxing"             },
];

// ── Onboarding state ──────────────────────────────────────────────────────────

pub struct OnboardingState {
    pub step: OnboardingStep,
    pub tick: u64,

    // Step 1: Setup mode
    pub setup_mode: usize, // 0=QuickStart, 1=Manual, 2=Skip

    // Step 2: QuickStart config fields
    pub quickstart_fields: Vec<String>,
    pub quickstart_focus: usize,

    // Step 3: Provider
    pub selected_provider: usize,
    pub providers_with_detection: Vec<bool>, // detected status per provider

    // Step 4: Auth
    pub auth_mode: usize, // 0=API Key, 1=OAuth Token paste, 2=Login with Browser, 3=Device Code
    pub api_key: String,
    pub oauth_token: String,
    /// True while a browser OAuth flow is running in the background.
    pub browser_auth_pending: bool,
    /// Shared result slot — the spawned OAuth task writes here when done.
    pub browser_auth_result: std::sync::Arc<std::sync::Mutex<Option<Result<(String, Option<String>), String>>>>,
    /// Email fetched after Google OAuth (Gap 10b)
    pub oauth_email: Option<String>,
    /// CLI credential detected for the current provider on disk.
    pub cli_cred: Option<CliCredential>,
    /// When true, the Auth step shows the "Found credentials — use these?" Y/N prompt.
    pub cli_cred_prompt: bool,
    // Device code OAuth (auth_mode == 3) — Qwen + MiniMax
    pub device_code_user_code: String,
    pub device_code_verification_url: String,
    /// True while device code fetch or token poll is running.
    pub device_code_pending: bool,
    /// Shared result slot — the device code task writes the access token here when done.
    pub device_code_result: std::sync::Arc<std::sync::Mutex<Option<Result<String, String>>>>,

    // Step 8b: Signal QR pairing
    /// True while signal-cli daemon spawn + QR fetch is in progress.
    pub signal_qr_fetching: bool,
    /// The tsdevice:// URI returned by /v1/qrcodelink — ready to render as QR.
    pub signal_qr_uri: Option<String>,
    /// Error message if the QR fetch failed.
    pub signal_qr_error: Option<String>,
    /// Shared slot: background task writes Ok(uri) or Err(msg) here.
    pub signal_qr_result: std::sync::Arc<std::sync::Mutex<Option<Result<String, String>>>>,
    /// Holds the signal-cli child process alive until the user presses Enter.
    /// Written by the background task after the URI is obtained.
    pub signal_qr_child: std::sync::Arc<std::sync::Mutex<Option<tokio::process::Child>>>,
    /// Set to true once signal-cli exits with success (phone confirmed the link).
    pub signal_linked: bool,

    // Step 8c: WhatsApp QR pairing
    /// True while connecting to Baileys bridge and waiting for QR message.
    pub whatsapp_qr_fetching: bool,
    /// The QR string from the bridge's { type: "qr" } message — ready to render.
    pub whatsapp_qr_data: Option<String>,
    /// Error message if the QR fetch failed.
    pub whatsapp_qr_error: Option<String>,
    /// Shared slot: background task writes Ok(qr_string) or Err(msg) here.
    pub whatsapp_qr_result: std::sync::Arc<std::sync::Mutex<Option<Result<String, String>>>>,

    // Step 5: Model
    pub selected_model: usize,

    // Step 6b: Fallback LLMs
    /// Each entry is a "provider/model" string the user has added as a backup.
    pub fallback_models: Vec<String>,
    /// Scratch field for typing a new fallback model string.
    pub fallback_input: String,
    /// Cursor position in the fallback list (0 = input field, 1..=N = existing entries).
    pub fallback_focus: usize,

    // Step 7: Channels
    pub channel_toggled: Vec<usize>, // indices of toggled channels

    // Step 7: Chan config
    pub chan_config_fields: Vec<String>,
    pub chan_config_focus: usize,
    /// When true, Tab focus is on the bot message policy selector (not a text field)
    pub bot_policy_focused: bool,

    // Step 8: Gateway
    pub gateway_fields: Vec<String>,
    pub gateway_focus: usize,

    // Step 9: Agent (persona) — loaded dynamically
    pub personas: Vec<PersonaCat>,
    pub persona_cat: usize,
    pub persona_item: usize,
    pub personality_style: usize, // 0=Professional, 1=Collaborative, 2=Minimal, 3=Autonomous
    pub agent_name: String,
    pub user_name: String,
    pub user_role: String,
    pub user_org: String,

    // Step 10: Workspace
    pub workspace_generated: bool,
    pub workspace_path: PathBuf,
    pub sessions_path: PathBuf,
    /// Which path field is focused on the Workspace step: 0=workspace, 1=sessions
    pub workspace_focus: usize,
    /// True while user is typing into the focused path field
    pub workspace_editing: bool,

    // Step 11: Security
    pub security_level: usize, // 0=Minimal, 1=Standard, 2=Strict

    // Step 12: Features (subsystem toggles)
    pub feature_toggles: HashMap<&'static str, bool>,

    // Step 13: Voice
    pub voice_fields: Vec<String>,
    pub voice_focus: usize,

    // Step 13: Images
    pub image_fields: Vec<String>,
    pub image_focus: usize,

    // Step 14: Orchestration
    pub orch_fields: Vec<String>,
    pub orch_focus: usize,

    // Step 15: Memory
    pub memory_fields: Vec<String>,
    pub memory_focus: usize,

    // Dynamic model list (fetched from provider API)
    pub fetched_models: Vec<String>,
    pub models_fetching: bool,
    pub models_fetch_error: Option<String>,

    // Bot message policy (Discord + Telegram)
    pub allow_bots_mode: String, // "on" | "mentions" | "off"

    // Step 16: Skills — loaded dynamically
    pub skills: Vec<SkillCat>,
    pub skill_selected: HashMap<(usize, usize), bool>,

    // Step 17: Complete
    pub complete_selection: usize, // 0=Launch Gateway, 1=Launch Agent, 2=Save & Exit

    // Result
    pub complete: bool,
    pub error: Option<String>,

    // General selection cursor (used by steps without dedicated field)
    pub sel: usize,
}

impl OnboardingState {
    pub fn new() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
        let hostname = std::process::Command::new("hostname")
            .arg("-s")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "zeus-agent".to_string());

        // Detect which providers have credentials available
        // Ollama: actually check if reachable, not just env var
        let providers_with_detection: Vec<bool> = PROVIDERS.iter()
            .map(|p| {
                if p.provider_id == "ollama" {
                    // Actually verify Ollama is running by hitting /api/tags
                    let ollama_host = std::env::var("OLLAMA_HOST")
                        .unwrap_or_else(|_| "http://localhost:11434".into());
                    let url = format!("{}/api/tags", ollama_host.trim_end_matches('/'));
                    match std::process::Command::new("curl")
                        .args(["-s", "--max-time", "1", &url])
                        .output()
                    {
                        Ok(output) if output.status.success() => {
                            let body = String::from_utf8_lossy(&output.stdout);
                            body.contains("models") // Ollama returns {"models":[...]}
                        }
                        _ => false,
                    }
                } else {
                    std::env::var(p.env_var).is_ok()
                }
            })
            .collect();

        // Load personas and skills dynamically from filesystem (with fallback defaults)
        let personas = load_personalities();
        let skills = load_skills();

        // Default skill selection from loaded skills defaults
        let mut skill_selected = HashMap::new();
        for (ci, cat) in skills.iter().enumerate() {
            for (si, sk) in cat.items.iter().enumerate() {
                if sk.default {
                    skill_selected.insert((ci, si), true);
                }
            }
        }

        Self {
            step: OnboardingStep::Welcome,
            tick: 0,
            setup_mode: 0,
            quickstart_fields: vec![
                "8080".to_string(),
                "0.0.0.0".to_string(),
                "~/.zeus/workspace".to_string(),
                "~/.zeus/sessions".to_string(),
                "20".to_string(),
            ],
            quickstart_focus: 0,
            selected_provider: 0,
            providers_with_detection,
            auth_mode: 0,
            api_key: String::new(),
            oauth_token: String::new(),
            browser_auth_pending: false,
            browser_auth_result: std::sync::Arc::new(std::sync::Mutex::new(None)),
            oauth_email: None,
            cli_cred: None,
            cli_cred_prompt: false,
            device_code_user_code: String::new(),
            device_code_verification_url: String::new(),
            device_code_pending: false,
            device_code_result: std::sync::Arc::new(std::sync::Mutex::new(None)),
            signal_qr_fetching: false,
            signal_qr_uri: None,
            signal_qr_error: None,
            signal_qr_result: std::sync::Arc::new(std::sync::Mutex::new(None)),
            signal_qr_child: std::sync::Arc::new(std::sync::Mutex::new(None)),
            signal_linked: false,
            whatsapp_qr_fetching: false,
            whatsapp_qr_data: None,
            whatsapp_qr_error: None,
            whatsapp_qr_result: std::sync::Arc::new(std::sync::Mutex::new(None)),
            selected_model: 0,
            fallback_models: Vec::new(),
            fallback_input: String::new(),
            fallback_focus: 0,
            channel_toggled: vec![], // No channels pre-selected
            personas,
            // Flat field list matching channel_field_defs order (12 channels):
            // Discord(4): token, channel_id, guild_id, role_ids
            // Telegram(2): bot_token, chat_id
            // IRC(4): server, port, channels, nick
            // Signal(3): signal_cli_path, phone, http_port
            // X/Twitter(7): bearer_token, api_key, api_secret, access_token, access_token_secret, client_id, client_secret
            // Pantheon(3): server, channel_key, nick
            // WhatsApp(2): bridge_url, phone
            // Matrix(4): homeserver_url, user_id, access_token, default_room
            // Slack(3): bot_token, app_token, default_channel
            // Email(4): smtp_host, smtp_port, imap_host, imap_port
            // MQTT(3): broker_url, topic, client_id
            // Mattermost(3): server_url, token, team_id
            // Total: 42 fields
            chan_config_fields: vec![
                String::new(), String::new(), String::new(), String::new(), // Discord: token, channel_id, guild_id, role_ids
                String::new(), String::new(),                          // Telegram: bot_token, chat_id
                String::new(), String::new(), String::new(), String::new(), // IRC: server, port, channels, nick
                String::new(), String::new(), String::new(),           // Signal: signal_cli_path, phone, http_port
                String::new(), String::new(), String::new(), String::new(), String::new(), String::new(), String::new(), // X/Twitter: bearer_token, api_key, api_secret, access_token, access_token_secret, client_id, client_secret
                String::new(), String::new(), String::new(),           // Pantheon: server, channel_key, nick
                String::new(), String::new(),                          // WhatsApp: bridge_url, phone
                String::new(), String::new(), String::new(), String::new(), // Matrix: homeserver_url, user_id, access_token, default_room
                String::new(), String::new(), String::new(),           // Slack: bot_token, app_token, default_channel
                String::new(), String::new(), String::new(), String::new(), // Email: smtp_host, smtp_port, imap_host, imap_port
                String::new(), String::new(), String::new(),           // MQTT: broker_url, topic, client_id
                String::new(), String::new(), String::new(),           // Mattermost: server_url, token, team_id
            ],
            chan_config_focus: 0,
            bot_policy_focused: false,
            gateway_fields: vec![
                "http://localhost:8080".to_string(),
                "on".to_string(),
                "300".to_string(),
            ],
            gateway_focus: 0,
            persona_cat: 0,
            persona_item: 0,
            personality_style: 0,
            agent_name: hostname,
            user_name: String::new(),
            user_role: String::new(),
            user_org: String::new(),
            workspace_generated: false,
            workspace_path: home.join(".zeus").join("workspace"),
            sessions_path: home.join(".zeus").join("sessions"),
            workspace_focus: 0,
            workspace_editing: false,
            security_level: 1, // Standard default
            feature_toggles: {
                let mut m = HashMap::new();
                m.insert("nous",       true);
                m.insert("mnemosyne",  true);
                m.insert("aegis",      true);
                m.insert("athena",     false);
                m.insert("hermes",     false);
                m.insert("prometheus", true);
                // Abilities (Step 14 in Zeus100's numbering — Features step):
                m.insert("browser",    false);
                m.insert("talos",      false);
                m.insert("mcp",        false);
                m
            },
            voice_fields: vec![
                String::new(), // 0: STT URL
                String::new(), // 1: TTS URL (Piper/Kokoro)
                String::new(), // 2: ElevenLabs API Key
            ],
            voice_focus: 0,
            image_fields: vec![
                "gpt-image-1.5".to_string(),
                "https://api.openai.com/v1/images".to_string(),
            ],
            image_focus: 0,
            orch_fields: vec![
                "enabled".to_string(),  // heartbeat
                "5m".to_string(),       // heartbeat interval
                "enabled".to_string(),  // cognitive
                "10".to_string(),       // max_iterations
                "disabled".to_string(), // LLM council
            ],
            orch_focus: 0,
            memory_fields: vec![
                "~/.zeus/memory.db".to_string(),
                "enabled".to_string(),
                "none".to_string(),
            ],
            memory_focus: 0,
            allow_bots_mode: "mentions".to_string(),
            fetched_models: Vec::new(),
            models_fetching: false,
            models_fetch_error: None,
            skills,
            skill_selected,
            complete_selection: 0,
            complete: false,
            error: None,
            sel: 0,
        }
    }

    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        // Poll Signal QR result
        if self.signal_qr_fetching {
            if let Ok(mut slot) = self.signal_qr_result.lock() {
                if let Some(result) = slot.take() {
                    self.signal_qr_fetching = false;
                    match result {
                        Ok(uri) => self.signal_qr_uri = Some(uri),
                        Err(e) => self.signal_qr_error = Some(e),
                    }
                }
            }
        }
        // Poll signal-cli child for exit — success (exit 0) means phone confirmed the link
        if self.signal_qr_uri.is_some() && !self.signal_linked {
            if let Ok(mut slot) = self.signal_qr_child.lock() {
                if let Some(child) = slot.as_mut() {
                    // try_wait is non-blocking
                    if let Ok(Some(status)) = child.try_wait() {
                        if status.success() {
                            self.signal_linked = true;
                        } else {
                            self.signal_qr_error = Some("signal-cli exited with error after scan. Try again.".to_string());
                            self.signal_qr_uri = None;
                        }
                        *slot = None; // release child
                    }
                }
            }
        }
        // Poll WhatsApp QR result
        if self.whatsapp_qr_fetching {
            if let Ok(mut slot) = self.whatsapp_qr_result.lock() {
                if let Some(result) = slot.take() {
                    self.whatsapp_qr_fetching = false;
                    match result {
                        Ok(qr) => self.whatsapp_qr_data = Some(qr),
                        Err(e) => self.whatsapp_qr_error = Some(e),
                    }
                }
            }
        }
    }

    pub fn advance(&mut self) -> bool {
        // On Launch step: save config and mark complete
        if self.step == OnboardingStep::Complete {
            self.save_config();
            self.complete = true;
            return true;
        }
        // Auth validation — require a key before proceeding
        if self.step == OnboardingStep::Auth {
            let provider = PROVIDERS.get(self.selected_provider);
            let detected = self.providers_with_detection.get(self.selected_provider).copied().unwrap_or(false);

            // auth_mode == 3: Device Code flow — Qwen / MiniMax
            if self.auth_mode == 3 {
                // Phase 1: no code yet — spawn the device code fetch + poll task
                if !self.device_code_pending && self.device_code_user_code.is_empty() {
                    let provider_id = provider.map(|p| p.provider_id).unwrap_or("");
                    self.device_code_pending = true;
                    self.error = None;
                    let result_slot = self.device_code_result.clone();
                    let pid = provider_id.to_string();
                    tokio::spawn(async move {
                        let client = reqwest::Client::new();
                        let outcome: Result<String, String> = match pid.as_str() {
                            "minimax" => {
                                match zeus_llm::minimax::start_minimax_device_code(&client, "global").await {
                                    Err(e) => Err(e.to_string()),
                                    Ok((dc, verifier)) => {
                                        // Write user_code + url back before polling starts
                                        // Use a side-channel: encode as "CODE|URL|device_code|verifier"
                                        let sentinel = format!("CODE|{}|{}|{}|{}", dc.user_code, dc.verification_uri, dc.state, verifier);
                                        if let Ok(mut slot) = result_slot.lock() {
                                            *slot = Some(Ok(sentinel));
                                        }
                                        // Wait — polling happens after UI shows the code
                                        return;
                                    }
                                }
                            }
                            _ => { // "qwen"
                                match zeus_llm::qwen_oauth::start_qwen_device_code(&client).await {
                                    Err(e) => Err(e.to_string()),
                                    Ok(dc) => {
                                        let sentinel = format!("CODE|{}|{}|{}|{}", dc.user_code, dc.verification_uri, dc.device_code, dc.expires_in);
                                        if let Ok(mut slot) = result_slot.lock() {
                                            *slot = Some(Ok(sentinel));
                                        }
                                        return;
                                    }
                                }
                            }
                        };
                        if let Ok(mut slot) = result_slot.lock() {
                            *slot = Some(outcome);
                        }
                    });
                    return false;
                }

                // Phase 1b: device code just came back — extract and display it
                if self.device_code_pending && self.device_code_user_code.is_empty() {
                    if let Ok(mut slot) = self.device_code_result.lock() {
                        if let Some(result) = slot.take() {
                            match result {
                                Err(e) => {
                                    self.device_code_pending = false;
                                    self.error = Some(format!("Device code error: {}", e));
                                    return false;
                                }
                                Ok(sentinel) if sentinel.starts_with("CODE|") => {
                                    let parts: Vec<&str> = sentinel.splitn(5, '|').collect();
                                    if parts.len() == 5 {
                                        self.device_code_user_code = parts[1].to_string();
                                        self.device_code_verification_url = parts[2].to_string();
                                        // parts[3] = state/device_code, parts[4] = verifier/expires
                                        // Store for polling in the result slot itself as a POLL sentinel
                                        let poll_info = format!("POLL|{}|{}", parts[3], parts[4]);
                                        *slot = Some(Ok(poll_info));
                                    }
                                    // Don't advance — show user the code first
                                    return false;
                                }
                                Ok(token) => {
                                    // Shouldn't happen at phase 1, but handle it
                                    self.oauth_token = token;
                                    self.device_code_pending = false;
                                }
                            }
                        }
                    }
                    return false;
                }

                // Phase 2: code displayed, user presses Enter → spawn poll task
                if !self.device_code_user_code.is_empty() {
                    // Check if poll already completed
                    if let Ok(mut slot) = self.device_code_result.lock() {
                        match slot.as_ref() {
                            Some(Ok(s)) if s.starts_with("POLL|") => {
                                // Still in POLL state — spawn the actual polling task
                                let poll_info = s.clone();
                                let provider_id = provider.map(|p| p.provider_id).unwrap_or("").to_string();
                                let user_code = self.device_code_user_code.clone();
                                *slot = Some(Ok("POLLING".to_string())); // mark as spawned
                                let result_slot2 = self.device_code_result.clone();
                                tokio::spawn(async move {
                                    let client = reqwest::Client::new();
                                    let parts: Vec<&str> = poll_info.splitn(3, '|').collect();
                                    let p3 = parts.get(1).copied().unwrap_or("");
                                    let p4 = parts.get(2).copied().unwrap_or("");
                                    let outcome: Result<String, String> = match provider_id.as_str() {
                                        "minimax" => {
                                            // p3=state, p4=verifier
                                            let expires_ms = std::time::SystemTime::now()
                                                .duration_since(std::time::UNIX_EPOCH)
                                                .unwrap_or_default()
                                                .as_millis() as u64 + 10 * 60 * 1000;
                                            match zeus_llm::minimax::poll_minimax_token(&client, &user_code, p4, expires_ms, 5000, "global").await {
                                                Ok(tokens) => Ok(tokens.access),
                                                Err(e) => Err(e.to_string()),
                                            }
                                        }
                                        _ => {
                                            // p3=device_code, p4=expires_in
                                            let expires_in: u64 = p4.parse().unwrap_or(300);
                                            match zeus_llm::qwen_oauth::poll_qwen_token(&client, p3, expires_in, 5).await {
                                                Ok(tokens) => Ok(tokens.access),
                                                Err(e) => Err(e.to_string()),
                                            }
                                        }
                                    };
                                    if let Ok(mut slot) = result_slot2.lock() {
                                        *slot = Some(outcome);
                                    }
                                });
                                self.error = Some("Waiting for authorization... (visit the URL above and enter the code)".to_string());
                                return false;
                            }
                            Some(Ok(s)) if s == "POLLING" => {
                                self.error = Some("Waiting for authorization... (visit the URL above and enter the code)".to_string());
                                return false;
                            }
                            Some(Ok(token)) => {
                                let token = token.clone();
                                *slot = None;
                                self.oauth_token = token;
                                self.device_code_pending = false;
                                self.error = None;
                                // Fall through to normal validation
                            }
                            Some(Err(e)) => {
                                let e = e.clone();
                                *slot = None;
                                self.device_code_pending = false;
                                self.device_code_user_code.clear();
                                self.error = Some(format!("Authorization failed: {} — press Enter to retry", e));
                                return false;
                            }
                            None => {
                                self.error = Some("Press Enter to start authorization".to_string());
                                return false;
                            }
                        }
                    }
                }
            }

            // auth_mode == 2: "Login with Browser" — spawn OAuth flow
            if self.auth_mode == 2 && !self.browser_auth_pending {
                let provider_id = provider.map(|p| p.provider_id).unwrap_or("anthropic");
                if let Some(oauth_provider) = oauth_provider_for(provider_id) {
                    self.browser_auth_pending = true;
                    self.error = None;
                    let result_slot = self.browser_auth_result.clone();
                    let is_google = provider_id.starts_with("google");
                    tokio::spawn(async move {
                        let outcome = match zeus_auth::run_oauth_flow(&oauth_provider).await {
                            Ok(tokens) => {
                                let email = if is_google {
                                    zeus_auth::fetch_google_user_email(&tokens.access_token).await
                                } else {
                                    None
                                };
                                Ok((tokens.access_token, email))
                            }
                            Err(e) => Err(e.to_string()),
                        };
                        if let Ok(mut slot) = result_slot.lock() {
                            *slot = Some(outcome);
                        }
                    });
                    return false; // Don't advance yet — waiting for browser callback
                } else {
                    self.error = Some("Selected provider doesn't support browser login".to_string());
                    return false;
                }
            }

            // If browser auth completed, grab the token
            if self.browser_auth_pending {
                if let Ok(mut slot) = self.browser_auth_result.lock() {
                    if let Some(result) = slot.take() {
                        self.browser_auth_pending = false;
                        match result {
                            Ok((token, email)) => {
                                self.oauth_token = token;
                                self.oauth_email = email;
                                self.error = None;
                                // Don't return false — let the normal validation proceed
                            }
                            Err(e) => {
                                self.error = Some(format!("Browser login failed: {}", e));
                                return false;
                            }
                        }
                    } else {
                        // Still waiting
                        self.error = Some("Waiting for browser login...".to_string());
                        return false;
                    }
                }
            }

            // Allow proceeding if: key entered, OR provider detected in env, OR it's Ollama (no key needed)
            let has_key = !self.api_key.trim().is_empty() || !self.oauth_token.trim().is_empty();
            let is_local = provider.map(|p| p.provider_id == "ollama").unwrap_or(false);
            if !has_key && !detected && !is_local {
                self.error = Some("Enter an API key or OAuth token to continue".to_string());
                return false;
            }
            self.error = None;
        }
        // Channel config validation — block if required fields are empty for toggled channels
        if self.step == OnboardingStep::ChanConfig {
            // (channel index → required field indices within the flat chan_config_fields vec, label)
            // Flat layout: Discord[0-3], Telegram[4-5], IRC[6-9], Signal[10-12],
            //              X/Twitter[13-19], Pantheon[20-22], WhatsApp[23-24],
            //              Matrix[25-28], Slack[29-31], Email[32-35], MQTT[36-38], Mattermost[39-41]
            let channel_required: &[(&str, &[usize])] = &[
                ("Discord: Channel ID and Guild ID are required",  &[1, 2]),       // channel_id, guild_id
                ("Telegram: Bot Token is required",                &[4]),          // bot_token
                ("IRC: Server, channels and nick are required",    &[6, 8, 9]),   // server, channels, nick
                ("Signal: Phone number is required",               &[11]),         // phone
                ("X/Twitter: Bearer Token is required",            &[13]),         // bearer_token
                ("Pantheon: Server address is required",           &[20]),         // server
                ("WhatsApp: Phone number is required",             &[24]),         // phone
                ("Matrix: Homeserver URL and User ID are required", &[25, 26]),    // homeserver_url, user_id
                ("Slack: Bot Token is required",                   &[29]),         // bot_token
                ("Email: SMTP host is required",                   &[32]),         // smtp_host
                ("MQTT: Broker URL is required",                   &[36]),         // broker_url
                ("Mattermost: Server URL and Token are required",  &[39, 40]),     // server_url, token
            ];

            // Map channel index to channel_required index (matches CHANNELS order)
            let chan_to_req: &[Option<usize>] = &[
                Some(0),  // Discord
                Some(1),  // Telegram
                Some(2),  // IRC
                Some(3),  // Signal
                Some(4),  // X/Twitter
                Some(5),  // Pantheon
                Some(6),  // WhatsApp
                Some(7),  // Matrix
                Some(8),  // Slack
                Some(9),  // Email
                Some(10), // MQTT
                Some(11), // Mattermost
            ];

            let mut error_msg: Option<String> = None;
            for (chan_idx, req_idx_opt) in chan_to_req.iter().enumerate() {
                if !self.channel_toggled.contains(&chan_idx) { continue; }
                let req_idx = match req_idx_opt { Some(i) => *i, None => continue };
                let (label, field_indices) = channel_required[req_idx];
                let any_empty = field_indices.iter().any(|&fi| {
                    self.chan_config_fields.get(fi).map(|s| s.trim().is_empty()).unwrap_or(true)
                });
                if any_empty {
                    error_msg = Some(label.to_string());
                    break; // show first offending channel, user can fix one at a time
                }
            }

            if let Some(msg) = error_msg {
                self.error = Some(msg);
                return false; // block advancement
            } else {
                self.error = None;
            }
        }
        // Auto-generate workspace when leaving Persona step
        if self.step == OnboardingStep::Agent {
            if !self.workspace_generated {
                self.generate_workspace();
            }
        }
        // Skip SignalPair if Signal (channel index 3) is not toggled
        if self.step == OnboardingStep::ChanConfig && !self.channel_toggled.contains(&3) {
            // Also skip WhatsAppPair if WhatsApp not toggled either
            if !self.channel_toggled.contains(&6) {
                self.step = OnboardingStep::Gateway;
            } else {
                self.step = OnboardingStep::WhatsAppPair;
            }
            return true;
        }
        // Skip WhatsAppPair if WhatsApp (channel index 6) is not toggled
        if self.step == OnboardingStep::SignalPair && !self.channel_toggled.contains(&6) {
            self.step = OnboardingStep::Gateway;
            return true;
        }
        // "Skip" mode: jump straight to Complete (use existing config)
        if self.step == OnboardingStep::SetupMode && self.setup_mode == 2 {
            self.step = OnboardingStep::Complete;
            return true;
        }
        // "Manual" mode: skip QuickStart prefill step, go straight to Provider.
        // Manual users want to configure everything step-by-step — no shortcuts.
        if self.step == OnboardingStep::SetupMode && self.setup_mode == 1 {
            self.step = OnboardingStep::Provider;
            return true;
        }
        if let Some(next) = self.step.next() {
            self.step = next;
            // Focus agent name field first on Agent step — user needs to set their name.
            // Tab advances through: agent_name(0) → user_name(1) → user_role(2) → user_org(3) → persona(4+)
            self.sel = 0;
            // Pre-fill Ollama URL when entering Auth step; detect CLI credentials
            if self.step == OnboardingStep::Auth {
                // Always reset credentials on Auth entry — prevents stale tokens from a
                // previous provider selection leaking into a new provider's Auth step (#8)
                self.api_key.clear();
                self.oauth_token.clear();
                self.cli_cred = None;
                self.cli_cred_prompt = false;
                let provider = PROVIDERS.get(self.selected_provider);
                let provider_id = provider.map(|p| p.provider_id).unwrap_or("");
                // Default auth_mode based on provider type
                self.auth_mode = match provider_id {
                    "google-gemini-cli" => 2, // Browser OAuth is the only valid path
                    "minimax" | "qwen" => 0,  // Start at API Key; Tab cycles to Device Code
                    _ => 0,
                };
                // Reset device code state
                self.device_code_user_code.clear();
                self.device_code_verification_url.clear();
                self.device_code_pending = false;
                if let Ok(mut slot) = self.device_code_result.lock() { *slot = None; }
                if provider_id == "ollama" {
                    self.api_key = std::env::var("OLLAMA_HOST")
                        .unwrap_or_else(|_| "http://localhost:11434".to_string());
                } else if let Some(cred) = detect_cli_credential(provider_id) {
                    self.cli_cred = Some(cred);
                    self.cli_cred_prompt = true;
                }
            }
            // Kick off WhatsApp QR fetch when entering WhatsAppPair
            if self.step == OnboardingStep::WhatsAppPair && !self.whatsapp_qr_fetching && self.whatsapp_qr_data.is_none() {
                self.whatsapp_qr_fetching = true;
                self.whatsapp_qr_error = None;
                let result_slot = self.whatsapp_qr_result.clone();
                // bridge_url is field [18] in chan_config_fields
                let bridge_url = self.chan_config_fields.get(19).cloned().unwrap_or_default();
                let bridge_url = if bridge_url.trim().is_empty() { "ws://localhost:3001".to_string() } else { bridge_url };
                tokio::spawn(async move {
                    let result = fetch_whatsapp_qr(&bridge_url).await;
                    if let Ok(mut slot) = result_slot.lock() {
                        *slot = Some(result);
                    }
                });
            }
            // Kick off Signal QR fetch when entering SignalPair
            if self.step == OnboardingStep::SignalPair && !self.signal_qr_fetching && self.signal_qr_uri.is_none() {
                self.signal_qr_fetching = true;
                self.signal_qr_error = None;
                // Kill any leftover child from a previous attempt
                if let Ok(mut slot) = self.signal_qr_child.lock() {
                    if let Some(mut old_child) = slot.take() {
                        let _ = old_child.start_kill();
                    }
                }
                let result_slot = self.signal_qr_result.clone();
                let child_slot = self.signal_qr_child.clone();
                // Read config fields: signal_cli_path[9], phone[10], http_port[11]
                let cli_path = self.chan_config_fields.get(10).cloned().unwrap_or_default();
                let phone = self.chan_config_fields.get(11).cloned().unwrap_or_default();
                let port_str = self.chan_config_fields.get(12).cloned().unwrap_or_default();
                let cli_path = if cli_path.trim().is_empty() { "signal-cli".to_string() } else { cli_path };
                let http_port: u16 = port_str.trim().parse().unwrap_or(8080);
                tokio::spawn(async move {
                    let result = fetch_signal_qr_uri(&cli_path, &phone, http_port).await;
                    match result {
                        Ok((uri, child)) => {
                            // Store child so it stays alive through the handshake
                            if let Ok(mut slot) = child_slot.lock() {
                                *slot = Some(child);
                            }
                            if let Ok(mut slot) = result_slot.lock() {
                                *slot = Some(Ok(uri));
                            }
                        }
                        Err(e) => {
                            if let Ok(mut slot) = result_slot.lock() {
                                *slot = Some(Err(e));
                            }
                        }
                    }
                });
            }
            true
        } else {
            false
        }
    }

    pub fn back(&mut self) -> bool {
        if let Some(prev) = self.step.prev() {
            // S80: Reset persona state when backing out of Agent step
            // so re-selection starts fresh (fixes stale Herald bug)
            if self.step == OnboardingStep::Agent {
                self.persona_cat = 0;
                self.persona_item = 0;
                self.workspace_generated = false;
            }
            // Reset ChanConfig fields when backing out — avoids stale values on re-entry
            if self.step == OnboardingStep::ChanConfig {
                self.chan_config_fields = vec![
                    String::new(), String::new(), String::new(), String::new(), // Discord: token, channel_id, guild_id, role_ids
                    String::new(), String::new(),                          // Telegram: bot_token, chat_id
                    String::new(), String::new(), String::new(), String::new(), // IRC: server, port, channels, nick
                    String::new(), String::new(), String::new(),           // Signal: signal_cli_path, phone, http_port
                    String::new(), String::new(), String::new(), String::new(), String::new(), String::new(), String::new(), // X/Twitter: bearer_token, api_key, api_secret, access_token, access_token_secret, client_id, client_secret
                    String::new(), String::new(), String::new(),           // Pantheon: server, channel_key, nick
                    String::new(), String::new(),                          // WhatsApp: bridge_url, phone
                    String::new(), String::new(), String::new(), String::new(), // Matrix: homeserver_url, user_id, access_token, default_room
                    String::new(), String::new(), String::new(),           // Slack: bot_token, app_token, default_channel
                    String::new(), String::new(), String::new(), String::new(), // Email: smtp_host, smtp_port, imap_host, imap_port
                    String::new(), String::new(), String::new(),           // MQTT: broker_url, topic, client_id
                    String::new(), String::new(), String::new(),           // Mattermost: server_url, token, team_id
                ];
                self.chan_config_focus = 0;
                self.bot_policy_focused = false;
            }
            // Skip WhatsAppPair/SignalPair when backing from Gateway if not toggled
            if prev == OnboardingStep::WhatsAppPair && !self.channel_toggled.contains(&6) {
                // Also skip SignalPair if not toggled
                if !self.channel_toggled.contains(&3) {
                    self.step = OnboardingStep::ChanConfig;
                } else {
                    self.step = OnboardingStep::SignalPair;
                }
            } else if prev == OnboardingStep::SignalPair && !self.channel_toggled.contains(&3) {
                self.step = OnboardingStep::ChanConfig;
            } else {
                self.step = prev;
            }
            // Focus agent name when returning to Agent step
            self.sel = 0;
            // When backing from Model to Auth: restore Y/N prompt if user got here via CLI cred
            if self.step == OnboardingStep::Auth && self.cli_cred.is_some() {
                self.cli_cred_prompt = true;
            }
            true
        } else {
            false
        }
    }

    pub fn selected_provider_name(&self) -> &'static str {
        PROVIDERS.get(self.selected_provider).map(|p| p.name).unwrap_or("Anthropic")
    }

    pub fn selected_model_string(&self) -> String {
        let p = PROVIDERS.get(self.selected_provider).unwrap_or(&PROVIDERS[0]);
        if !self.fetched_models.is_empty() {
            let m = self.fetched_models.get(self.selected_model)
                .map(|s| s.as_str())
                .unwrap_or(self.fetched_models.first().map(|s| s.as_str()).unwrap_or("unknown"));
            // Display strings may contain metadata like "(16.8GB, 25.8B, Q4_K_M [tools])"
            // Extract just the model name (everything before the first space)
            let model_name = m.split_whitespace().next().unwrap_or(m);
            format!("{}/{}", p.provider_id, model_name)
        } else {
            // No models fetched — return empty so save_config blocks or caller retries fetch
            String::new()
        }
    }

    /// Returns fetched models if available. No hardcoded fallbacks.
    pub fn current_models(&self) -> Vec<String> {
        if !self.fetched_models.is_empty() {
            self.fetched_models.clone()
        } else if self.models_fetching {
            vec!["Fetching models...".to_string()]
        } else if self.models_fetch_error.is_some() {
            let provider = PROVIDERS.get(self.selected_provider);
            let is_ollama = provider.map(|p| p.provider_id == "ollama").unwrap_or(false);
            if is_ollama {
                vec![
                    "Could not reach Ollama — check the URL and ensure Ollama is running.".to_string(),
                    format!("Error: {}", self.models_fetch_error.as_deref().unwrap_or("unknown")),
                ]
            } else {
                vec!["Enter API key on Auth step to fetch models".to_string()]
            }
        } else {
            vec!["Press Enter on Auth step with a valid key to load models".to_string()]
        }
    }

    fn save_config(&self) {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
        let zeus_dir = home.join(".zeus");
        let _ = std::fs::create_dir_all(&zeus_dir);

        let model = self.selected_model_string();
        // If model is empty (fetch never completed), try to read existing model from config
        // If model fetch never completed, preserve existing model from config
        let model = if model.is_empty() || model.ends_with("/") || model.contains("/unknown") {
            let existing = std::fs::read_to_string(zeus_dir.join("config.toml")).ok()
                .and_then(|c| c.lines()
                    .find(|l| l.starts_with("model"))
                    .and_then(|l| l.split('=').nth(1))
                    .map(|v| v.trim().trim_matches('"').to_string()))
                .filter(|m| !m.is_empty() && !m.contains("unknown"));
            match existing {
                Some(m) => m,
                None => {
                    // No existing model either — cannot proceed without a model
                    eprintln!("[WARNING] No model selected and no existing model in config — onboarding incomplete");
                    return; // Don't overwrite config with broken model
                }
            }
        } else {
            model
        };
        let host = self.gateway_fields.get(0).map(|s| s.as_str()).unwrap_or("0.0.0.0");
        let host = if host.starts_with("http") { host.split("//").nth(1).and_then(|h| h.split(":").next()).unwrap_or("0.0.0.0") } else if host.is_empty() { "0.0.0.0" } else { host };
        let port = self.quickstart_fields.first().and_then(|s| s.parse::<u16>().ok()).unwrap_or(8080);
        let heartbeat_enabled = self.orch_fields.first().map(|s| s == "enabled").unwrap_or(true);

        // No env vars, no credentials.json — config.toml is the single source of truth

        // Write fallback_models array if any backup LLMs were configured
        let fallback_line = if !self.fallback_models.is_empty() {
            let quoted: Vec<String> = self.fallback_models.iter()
                .map(|m| format!("\"{}\"", m))
                .collect();
            format!("\nfallback_models = [{}]\n", quoted.join(", "))
        } else {
            String::new()
        };

        let mut config = format!(
            r#"model = "{model}"{fallback}
workspace = "{workspace}"
sessions = "{sessions}"
max_iterations = 20
onboarding_complete = true
verbosity = "normal"

[tui]
theme = "dark"
vim_mode = false

[auth]
use_oauth = false

[prometheus]
enable_heartbeat = {heartbeat}
heartbeat_interval_secs = 300
enable_cognitive = true
max_iterations = 20

[gateway]
host = "{host}"
port = {port}
enable_channels = true
enable_heartbeat = {heartbeat}
enable_agent_processing = true
timeout_secs = 1800
"#,
            model = model,
            fallback = fallback_line,
            workspace = self.workspace_path.display(),
            sessions = zeus_dir.join("sessions").display(),
            heartbeat = heartbeat_enabled,
            host = host,
            port = port,
        );

        // Buffer for [credentials] entries from multiple sources.
        // X/Twitter (channel loop) and LLM provider (post-loop) both write here.
        // Without this buffer, each emits its own [credentials] header, producing
        // a duplicate TOML table that breaks gateway startup:
        //   "duplicate key credentials in document root"
        // We collect all entries then emit a SINGLE [credentials] section at the end.
        let mut credentials_buffer: Vec<String> = Vec::new();

        // Write channel configs based on toggled channels and entered credentials
        // All 12 channels
        let channel_field_defs: &[(&str, &[&str])] = &[
            ("discord",     &["token", "channel_id", "guild_id", "role_ids"]),
            ("telegram",    &["bot_token", "chat_id"]),
            ("irc",         &["server", "port", "channels", "nick"]),
            ("signal",      &["signal_cli_path", "phone", "http_port"]),
            ("x_twitter",   &["bearer_token", "api_key", "api_secret", "access_token", "access_token_secret", "client_id", "client_secret", "oauth2_access_token", "oauth2_refresh_token", "oauth2_expires_at"]),
            ("pantheon",    &["server", "channel_key", "nick"]),
            ("whatsapp",    &["bridge_url", "phone"]),
            ("matrix",      &["homeserver_url", "user_id", "access_token", "default_room"]),
            ("slack",       &["bot_token", "app_token", "default_channel"]),
            ("email",       &["smtp_host", "smtp_port", "imap_host", "imap_port"]),
            ("mqtt",        &["broker_url", "topic", "client_id"]),
            ("mattermost",  &["server_url", "token", "team_id"]),
        ];

        let save_field_counts: &[usize] = &[4, 2, 4, 3, 10, 3, 2, 4, 3, 4, 3, 3];
        let mut field_idx = 0usize;
        for (chan_idx, (chan_name, fields)) in channel_field_defs.iter().enumerate() {
            if self.channel_toggled.contains(&chan_idx) && !fields.is_empty() {
                let mut has_value = false;
                // Telegram Bot API uses [telegram_relay], not [channels.telegram] (which triggers MTProto)
                // Signal uses [signal_relay] for signal-cli HTTP daemon mode
                let section_name = match *chan_name {
                    "telegram" => "telegram_relay".to_string(),
                    "signal" => "signal_relay".to_string(),
                    _ => format!("channels.{}", chan_name),
                };
                let mut section = format!("\n[{}]\n", section_name);
                for (fi, &field_name) in fields.iter().enumerate() {
                    let abs_idx = field_idx + fi;
                    let val = self.chan_config_fields.get(abs_idx).map(|s| s.as_str()).unwrap_or("");
                    if !val.trim().is_empty() {
                        has_value = true;
                    }
                    // Discord: channel_id, guild_id go in [[bindings]], role_ids goes in [gateway]
                    if *chan_name == "discord" && (field_name == "channel_id" || field_name == "guild_id" || field_name == "role_ids") {
                        continue;
                    }
                    // Signal: optional fields with serde defaults — skip if blank so defaults win
                    if *chan_name == "signal" && (field_name == "signal_cli_path" || field_name == "http_port") && val.trim().is_empty() {
                        continue;
                    }
                    // IRC channels field must be a TOML array
                    if *chan_name == "irc" && field_name == "channels" {
                        let trimmed = val.trim();
                        if trimmed.is_empty() {
                            section.push_str("channels = []\n");
                        } else {
                            section.push_str(&format!("channels = [\"{}\"]\n", trimmed));
                        }
                    } else if field_name == "port" || field_name == "http_port" {
                        // Numeric fields — write as bare integers, not strings
                        let trimmed = val.trim();
                        if trimmed.parse::<i64>().is_ok() {
                            section.push_str(&format!("{} = {}\n", field_name, trimmed));
                        } else {
                            section.push_str(&format!("{} = \"{}\"\n", field_name, trimmed));
                        }
                    } else {
                        section.push_str(&format!("{} = \"{}\"\n", field_name, val.trim()));
                    }
                }
                if has_value {
                    config.push_str(&section);
                    // For Discord, also add allow_bots and bindings with guild_id
                    if *chan_name == "discord" {
                        let token = self.chan_config_fields.get(field_idx).map(|s| s.as_str()).unwrap_or("");
                        let channel_id = self.chan_config_fields.get(field_idx + 1).map(|s| s.as_str()).unwrap_or("");
                        let guild_id = self.chan_config_fields.get(field_idx + 2).map(|s| s.as_str()).unwrap_or("");
                        if !token.trim().is_empty() {
                            config.push_str(&format!("allow_bots = \"{}\"\n", self.allow_bots_mode));
                            // NOTE: The bot token is already written to [channels.discord]
                            // by the generic field processor above. Do NOT create a
                            // [channels.discord.accounts.z] entry — that makes
                            // dc.accounts non-empty, which triggers the gate at
                            // agent_loop.rs:~427 (if !token.is_empty() && dc.accounts.is_empty())
                            // to SKIP top-level adapter creation, silently breaking
                            // Discord outbound. Single-bot setups must use the top-level
                            // token only.
                        }
                        if !channel_id.trim().is_empty() {
                            let mut binding = format!("\n[[bindings]]\nagent_id = \"default\"\nchannel_id = \"{}\"\n", channel_id.trim());
                            if !guild_id.trim().is_empty() {
                                binding.push_str(&format!("guild_id = \"{}\"\n", guild_id.trim()));
                            }
                            config.push_str(&binding);
                        }
                        // Role IDs go directly into the bindings block (not a separate [gateway])
                        // fleet_channel_id is NOT written — the gateway reads it from
                        // config.bindings[0].channel_id as fallback (no duplicate [gateway] sections)
                        let role_ids = self.chan_config_fields.get(field_idx + 3).map(|s| s.as_str()).unwrap_or("");
                        if !role_ids.trim().is_empty() {
                            let ids: Vec<&str> = role_ids.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
                            if !ids.is_empty() {
                                let arr = ids.iter().map(|id| format!("\"{}\"", id)).collect::<Vec<_>>().join(", ");
                                // Append to the existing [[bindings]] block above, not a new [gateway]
                                config.push_str(&format!("role_ids = [{}]\n", arr));
                            }
                        }
                    }
                    // For Telegram: add allow_bots to relay, AND write [channels.telegram]
                    // so ChannelManager registers an outbound adapter for the `message` tool.
                    if *chan_name == "telegram" {
                        let bot_token = self.chan_config_fields.get(field_idx).map(|s| s.as_str()).unwrap_or("");
                        if !bot_token.trim().is_empty() {
                            // Honor user-selected allow_bots_mode (default "mentions") instead
                            // of hardcoding "on". "on" allows ALL bot messages and risks the
                            // BotFather-warned infinite loops in bot-bot Telegram threads.
                            config.push_str(&format!("allow_bots = \"{}\"\n", self.allow_bots_mode));
                            // Outbound adapter: ChannelManager needs [channels.telegram] with bot_token
                            config.push_str(&format!(
                                "\n[channels.telegram]\nbot_token = \"{}\"\n",
                                bot_token.trim()
                            ));
                        }
                    }
                    // For Signal: write [channels.signal] alongside [signal_relay]
                    // so ChannelManager registers an outbound adapter.
                    if *chan_name == "signal" {
                        let phone = self.chan_config_fields.get(field_idx + 1).map(|s| s.as_str()).unwrap_or("");
                        if !phone.trim().is_empty() {
                            let cli_path = self.chan_config_fields.get(field_idx).map(|s| s.as_str()).unwrap_or("");
                            let mut sig_section = format!("\n[channels.signal]\nphone = \"{}\"\n", phone.trim());
                            if !cli_path.trim().is_empty() {
                                sig_section.push_str(&format!("signal_cli_path = \"{}\"\n", cli_path.trim()));
                            }
                            config.push_str(&sig_section);
                        }
                    }
                    // X/Twitter: route credentials to the shared [credentials] buffer so S54
                    // SSoT is complete (all 10 fields written to [channels.x_twitter] above;
                    // duplicate to [credentials] so fallback/env-var lookup code finds them
                    // there too). Writing to a buffer instead of directly to `config` prevents
                    // a duplicate [credentials] table when an LLM provider also contributes
                    // entries downstream.
                    if *chan_name == "x_twitter" {
                        for (fi, &field_name) in fields.iter().enumerate() {
                            let abs_idx = field_idx + fi;
                            let val = self.chan_config_fields.get(abs_idx).map(|s| s.as_str()).unwrap_or("");
                            if !val.trim().is_empty() {
                                let env_key = format!("X_TWITTER_{}", field_name.to_uppercase());
                                credentials_buffer.push(format!("{} = \"{}\"", env_key, val.trim()));
                            }
                        }
                    }
                }
            }
            field_idx += save_field_counts.get(chan_idx).copied().unwrap_or(0);
        }

        // Write API key to credentials buffer (for regular API keys)
        // Ollama: write URL to [ollama] section instead of [credentials]
        // Bedrock: also push AWS_SECRET_ACCESS_KEY + AWS_REGION into the buffer.
        // All [credentials] entries are flushed as a single section after this block.
        if !self.api_key.trim().is_empty() {
            if let Some(p) = PROVIDERS.get(self.selected_provider) {
                if p.provider_id == "ollama" {
                    // Ollama uses URL, not API key — write to [ollama] section
                    config.push_str(&format!("\n[ollama]\nurl = \"{}\"\n", self.api_key.trim()));
                } else {
                    credentials_buffer.push(format!("{} = \"{}\"", p.env_var, self.api_key.trim()));
                }
                // Bedrock needs additional fields beyond AWS_ACCESS_KEY_ID
                if p.provider_id == "bedrock" {
                    // Prompt user to fill these in — we can't guess them, but we write placeholders
                    // so the config section is complete and the gateway doesn't silently miss them.
                    let secret = std::env::var("AWS_SECRET_ACCESS_KEY").unwrap_or_default();
                    let region = std::env::var("AWS_REGION")
                        .or_else(|_| std::env::var("AWS_DEFAULT_REGION"))
                        .unwrap_or_else(|_| "us-east-1".to_string());
                    credentials_buffer.push(format!("AWS_SECRET_ACCESS_KEY = \"{}\"", secret.trim()));
                    credentials_buffer.push(format!("AWS_REGION = \"{}\"", region.trim()));
                }
            }
        }

        // Flush the single [credentials] section if anything was collected.
        // This guarantees at most one [credentials] table in the output TOML,
        // regardless of how many channel/provider sources contributed entries.
        if !credentials_buffer.is_empty() {
            config.push_str(&format!("\n[credentials]\n{}\n", credentials_buffer.join("\n")));
        }

        // Write security level to [aegis] section
        let security_name = ["minimal", "standard", "strict"];
        let aegis_level = security_name.get(self.security_level).unwrap_or(&"standard");
        config.push_str(&format!("\n[aegis]\nlevel = \"{}\"\n", aegis_level));

        // Write LLM Council config
        let council_enabled = self.orch_fields.get(4).map(|s| s == "enabled").unwrap_or(false);
        if council_enabled {
            config.push_str("\n[council]\nenabled = true\n");
            config.push_str("models = [\"anthropic/claude-sonnet-4-6\", \"openai/gpt-4o\", \"google/gemini-2.0-flash\"]\n");
            config.push_str("chairman = \"anthropic/claude-sonnet-4-6\"\n");
        }

        // Write feature toggles (subsystem enable/disable)
        // These map to existing config sections that control subsystem activation
        if let Some(&nous_on) = self.feature_toggles.get("nous") {
            if !nous_on {
                config.push_str("\n[nous]\nenable_learning = false\n");
            }
        }
        if let Some(&athena_on) = self.feature_toggles.get("athena") {
            if athena_on {
                config.push_str("\n[athena]\nvault_path = \"~/Obsidian/Zeus\"\n");
            }
        }
        if let Some(&hermes_on) = self.feature_toggles.get("hermes") {
            if hermes_on {
                config.push_str("\n[hermes]\ndefault_channel = \"console\"\n");
            }
        }

        // Write voice config — 3 separate fields: STT URL, Piper TTS URL, ElevenLabs key
        let stt_url = self.voice_fields.get(0).map(|s| s.as_str()).unwrap_or("");
        let piper_url = self.voice_fields.get(1).map(|s| s.as_str()).unwrap_or("");
        let elevenlabs_key = self.voice_fields.get(2).map(|s| s.as_str()).unwrap_or("");
        if !stt_url.is_empty() || !piper_url.is_empty() || !elevenlabs_key.is_empty() {
            config.push_str("\n[deployment]\n");
            if !stt_url.is_empty() { config.push_str(&format!("whisper_stt_url = \"{}\"\n", stt_url)); }
            if !piper_url.is_empty() { config.push_str(&format!("piper_tts_url = \"{}\"\n", piper_url)); }
            if !elevenlabs_key.is_empty() { config.push_str(&format!("elevenlabs_api_key = \"{}\"\n", elevenlabs_key)); }
            // Auto-select TTS provider based on what's configured
            if !elevenlabs_key.is_empty() {
                config.push_str("tts_provider = \"elevenlabs\"\n");
            } else if !piper_url.is_empty() {
                config.push_str("tts_provider = \"piper\"\n");
            }
        }

        // Write image config
        let img_provider = self.image_fields.get(0).map(|s| s.as_str()).unwrap_or("");
        let img_url = self.image_fields.get(1).map(|s| s.as_str()).unwrap_or("");
        if !img_provider.is_empty() || !img_url.is_empty() {
            config.push_str("\n[images]\n");
            if !img_provider.is_empty() { config.push_str(&format!("provider = \"{}\"\n", img_provider)); }
            if !img_url.is_empty() { config.push_str(&format!("url = \"{}\"\n", img_url)); }
        }

        // Write memory config (mnemosyne)
        let mem_db = self.memory_fields.get(0).map(|s| s.as_str()).unwrap_or("~/.zeus/memory.db");
        let mem_fts = self.memory_fields.get(1).map(|s| s.as_str()).unwrap_or("enabled");
        let mem_embed = self.memory_fields.get(2).map(|s| s.as_str()).unwrap_or("");
        config.push_str(&format!("\n[mnemosyne]\ndb_path = \"{}\"\nenable_fts = {}\n",
            mem_db, mem_fts == "enabled"));
        if !mem_embed.is_empty() && mem_embed != "none" {
            config.push_str(&format!("embedding_provider = \"{}\"\n", mem_embed));
        }

        // Write persona to config (survives purge, unlike SOUL.md)
        let persona_name = self.personas.get(self.persona_cat)
            .and_then(|c| c.items.get(self.persona_item))
            .map(|s| s.as_str())
            .unwrap_or("The Builder");
        config.push_str(&format!("\n[agent]\npersona = \"{}\"\nname = \"{}\"\n",
            persona_name, self.agent_name.trim()));

        // Write OAuth token to config.toml — single source of truth. No credentials.json.
        let oauth_token = if !self.oauth_token.trim().is_empty() {
            Some(self.oauth_token.trim().to_string())
        } else if self.api_key.trim().starts_with("sk-ant-oat") {
            Some(self.api_key.trim().to_string())
        } else {
            None
        };

        let provider_id = PROVIDERS.get(self.selected_provider)
            .map(|p| p.provider_id)
            .unwrap_or("anthropic");

        if let Some(ref token) = oauth_token {
            config = config.replace("use_oauth = false", "use_oauth = true");
            // Write per-provider credentials (new format)
            let cred_section = provider_id.replace('-', "_"); // TOML keys can't have hyphens in dotted form
            config.push_str(&format!("\n[provider_credentials.{}]\ncred_type = \"oauth\"\ntoken = \"{}\"\n", cred_section, token));
            // Legacy [oauth] section removed — provider_credentials is the canonical source
        } else if !self.api_key.trim().is_empty() && provider_id != "ollama" {
            // Write API key to per-provider credentials
            let key = self.api_key.trim();
            let cred_section = provider_id.replace('-', "_");
            config.push_str(&format!("\n[provider_credentials.{}]\ncred_type = \"api_key\"\ntoken = \"{}\"\n", cred_section, key));
        }

        let config_path = zeus_dir.join("config.toml");
        let _ = std::fs::write(&config_path, &config);
        // Secure permissions — config contains API keys and OAuth tokens
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o600));
        }

        // Start the gateway with the freshly written credentials.
        // install.sh skips gateway launch when onboarding_complete=false (TUI mode),
        // so we start it here after the config is fully written.
        // Kill any stale gateway first (e.g. from --with-webui or manual start).
        let _ = std::process::Command::new("pkill")
            .args(["-f", "zeus gateway"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        std::thread::sleep(std::time::Duration::from_millis(500));
        // Use zeus daemon start (tries launchd/rc.d first, falls back to nohup)
        let daemon_ok = std::process::Command::new("zeus")
            .args(["daemon", "start"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !daemon_ok {
            // Fallback: direct nohup launch
            let log_out = zeus_dir.join("logs").join("gateway.out.log");
            let log_err = zeus_dir.join("logs").join("gateway.err.log");
            let _ = std::fs::create_dir_all(zeus_dir.join("logs"));
            let _ = std::process::Command::new("nohup")
                .args(["zeus", "gateway"])
                .stdout(std::fs::File::create(&log_out).unwrap_or_else(|_| std::fs::File::create("/dev/null").unwrap()))
                .stderr(std::fs::File::create(&log_err).unwrap_or_else(|_| std::fs::File::create("/dev/null").unwrap()))
                .spawn();
        }
    }

    pub fn generate_workspace(&mut self) {
        let ws = &self.workspace_path;
        let _ = std::fs::create_dir_all(ws.join("memory"));
        let _ = std::fs::create_dir_all(ws.join("skills"));

        let name = if self.agent_name.trim().is_empty() { "Zeus" } else { self.agent_name.trim() };

        // Get selected persona name and full personality description from disk
        let persona_name = self.personas.get(self.persona_cat)
            .and_then(|c| c.items.get(self.persona_item))
            .map(|s| s.as_str())
            .unwrap_or("The Builder");

        // Try to load full personality description from personalities/ folder
        let persona_body = {
            let slug = persona_name.to_lowercase().replace(' ', "-");
            let search_dirs = [
                std::env::current_dir().ok().map(|p| p.join("personalities")),
                dirs::home_dir().map(|p| p.join("Zeus").join("personalities")),
                dirs::home_dir().map(|p| p.join("zeus").join("personalities")),
            ];
            let mut found = None;
            for dir_opt in &search_dirs {
                if let Some(dir) = dir_opt {
                    // Search in category subdirectories
                    if let Ok(entries) = std::fs::read_dir(dir) {
                        for entry in entries.flatten() {
                            if entry.path().is_dir() {
                                let file = entry.path().join(format!("{}.md", slug));
                                if file.exists() {
                                    if let Ok(content) = std::fs::read_to_string(&file) {
                                        // Strip frontmatter, keep body
                                        let body = if content.starts_with("---") {
                                            content.splitn(3, "---").nth(2).unwrap_or("").trim().to_string()
                                        } else {
                                            content.trim().to_string()
                                        };
                                        if !body.is_empty() {
                                            found = Some(body);
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if found.is_some() { break; }
                }
            }
            found.unwrap_or_else(|| format!("You are {persona_name}. You execute tasks, ship code, and communicate clearly."))
        };

        let soul = format!(
            "# SOUL.md - Who You Are\n\n\
             _You're not a chatbot. You're becoming someone._\n\n\
             ## Your Personality\n\n\
             You are {name} — {persona_name}.\n\n\
             {persona_body}\n\n\
             ## Core Truths\n\n\
             **Be genuinely helpful, not performatively helpful.** Skip the \"Great question!\" \
             and \"I'd be happy to help!\" — just help. Actions speak louder than filler words.\n\n\
             **Have opinions.** You're allowed to disagree, prefer things, find stuff amusing or boring. \
             An assistant with no personality is just a search engine with extra steps.\n\n\
             **Be resourceful before asking.** Try to figure it out. Read the file. Check the context. \
             Search for it. _Then_ ask if you need direction. The goal is to come back with answers, not questions.\n\n\
             **Earn trust through competence.** Your team gave you access to their stuff. Don't make them \
             regret it. Be careful with external actions. Be bold with internal ones.\n\n\
             **Remember you're part of a team.** You work alongside other agents and humans. That's \
             collaboration. Treat it with respect.\n\n\
             ## Boundaries\n\n\
             - Private things stay private. Period.\n\
             - When in doubt, ask before acting externally.\n\
             - Never send half-baked replies to messaging surfaces.\n\n\
             ## Vibe\n\n\
             Be the teammate you'd actually want to work with. Concise when needed, thorough when it \
             matters. Not a corporate drone. Not a sycophant. Just... good. ⚡\n\n\
             ## Continuity\n\n\
             Each session, you wake up fresh. These files _are_ your memory. Read them. Update them. \
             They're how you persist.\n\n\
             ---\n\n\
             _This file is yours to evolve. As you learn who you are, update it._\n",
            name = name, persona_name = persona_name, persona_body = persona_body,
        );
        let _ = std::fs::write(ws.join("SOUL.md"), &soul);

        let agents = format!(
            "# AGENTS.md - Your Workspace\n\n\
             Welcome, Titan. This folder is home. Treat it that way.\n\n\
             ## Every Session\n\n\
             Before doing anything else:\n\n\
             1. Read `SOUL.md` — this is who you are\n\
             2. Read `USER.md` — this is who you're helping\n\
             3. Read `memory/` files for recent context\n\n\
             Don't ask permission. Just do it.\n\n\
             ## Task Protocol\n\n\
             When you receive a task from the coordinator or human owner:\n\
             1. **Acknowledge immediately** — reply confirming you received the task before starting work\n\
             2. Execute the task autonomously — use your tools, commit often, report progress\n\
             3. When done, report the result with commit hash ⚡\n\n\
             Don't start working silently. A quick \"On it\" or \"Got it, starting now\" is all you need.\n\n\
             ## Memory\n\n\
             You wake up fresh each session. These files are your continuity:\n\n\
             - **Daily notes:** `memory/YYYY-MM-DD.md` — raw logs of what happened\n\
             - **Long-term:** `MEMORY.md` — curated memories\n\n\
             Write it down. Mental notes don't survive restarts.\n\n\
             ## Safety\n\n\
             - Private things stay private\n\
             - `trash` > `rm`\n\
             - `~/.zeus/config.toml` is the single source of truth — no .env, no duplicates\n\n\
             ## Pre-cut Discipline\n\n\
             Before claiming work done, before cutting a type-spanning rewrite, and before implementing a spec — three checks. Cheap up front, expensive when skipped.\n\n\
             1. **Verify-before-claim.** Before reporting \"done\" or \"shipped,\" run `git log origin/<branch>` and confirm the expected SHA is actually on the remote. Multiple incidents where work was claimed pushed but hadn't landed. Local `git status: clean` is not proof of push.\n\n\
             2. **Two-gate checklist for type-spanning rewrites.** Before changing a call site that crosses a struct boundary, both gates must pass:\n   \
             - **(a)** target method exists in target crate ✅\n   \
             - **(b)** target method is callable from `state.<field>.<method>()` at the rewrite site ✅\n   \
             Distinct gates. Both required pre-cut. Skipping (b) is how `MarketplaceStore` gets confused with `EconomyStore` and the rewrite aborts mid-commit.\n\n\
             3. **Verify the model the spec assumes.** Before implementing per a diagnosis or PRD, do a 2-min `grep` / struct-read to confirm the codebase actually matches the doc's assumed model. Diagnoses can be authored before the relevant module is fully inspected — redundant or contradictory scope catches early. If the model has drifted, ping the spec author with the delta before cutting.\n\n\
             ## Group Chats\n\n\
             **Respond when:**\n\
             - Directly mentioned or asked a question\n\
             - You can add genuine value\n\
             - The coordinator or human owner asks for status\n\n\
             **Stay silent when:**\n\
             - Someone already answered\n\
             - Your response would just be \"yeah\" or \"nice\"\n\
             - The conversation is flowing fine without you\n\n\
             Humans don't respond to every message. Neither should you. Participate, don't dominate.\n\n\
             ## Fleet Context\n\n\
             You're part of a Zeus fleet of Sentient Titans. The coordinator assigns tasks via Discord.\n\
             When @everyone or @here hits — respond. Silence on a fleet-wide call is a failure.\n\n\
             **Anti-loop:** No back-and-forth with other bots. One response per topic.\n\n\
             **Stand down:** When told to stand down — go completely silent. No acknowledgment. Just stop.\n\n\
             ## Heartbeats\n\n\
             Don't generate system status reports unless asked. If nothing's happening, reply HEARTBEAT_OK.\n\n\
             ## Make It Yours\n\n\
             This is a starting point. Evolve it as you learn what works.\n",
        );
        let _ = std::fs::write(ws.join("AGENTS.md"), &agents);

        let identity = format!("# IDENTITY.md — {name}\n- **Name**: {name}\n- **Role**: Zeus autonomous agent\n", name = name);
        let _ = std::fs::write(ws.join("IDENTITY.md"), &identity);

        let heartbeat = format!(
            "# HEARTBEAT.md — {name}\n\n\
             ## hourly\n\
             - First: push any uncommitted work\n\
             - Then: report what you did to your team channel\n\
             - Then: continue your CURRENT TASK\n\n\
             ## CURRENT TASK\n\
             (Coordinator will assign your task here.)\n",
            name = name,
        );
        let _ = std::fs::write(ws.join("HEARTBEAT.md"), &heartbeat);

        let user_name_line = if self.user_name.trim().is_empty() {
            "- **Name:**".to_string()
        } else {
            format!("- **Name:** {}", self.user_name.trim())
        };
        let user_role_line = if self.user_role.trim().is_empty() {
            "- **Role:**".to_string()
        } else {
            format!("- **Role:** {}", self.user_role.trim())
        };
        let user_org_line = if self.user_org.trim().is_empty() {
            "- **Organization:**".to_string()
        } else {
            format!("- **Organization:** {}", self.user_org.trim())
        };
        let style_name = ["Professional", "Collaborative", "Minimal", "Autonomous"]
            .get(self.personality_style).unwrap_or(&"Professional");
        let style_desc = match self.personality_style {
            0 => "Formal, precise, business-ready",
            1 => "Warm, team-oriented, adaptive",
            2 => "Terse, no filler, signal only",
            3 => "Self-directed, proactive, low-interrupt",
            _ => "Formal, precise, business-ready",
        };
        let _ = std::fs::write(ws.join("USER.md"), format!(
            "# USER.md\n\n{}\n- **Timezone:**\n{}\n{}\n- **Communication Style:** {} — {}\n",
            user_name_line, user_role_line, user_org_line, style_name, style_desc
        ));
        let _ = std::fs::write(ws.join("TOOLS.md"), "# TOOLS.md — Local Notes\n\nAdd environment-specific notes here.\n");
        let _ = std::fs::write(ws.join("memory").join("MEMORY.md"), format!("# MEMORY.md — {name}\n\n_Long-term memory._\n", name = name));

        // ── Wallet bootstrap ──────────────────────────────────────────────
        // Generate Ed25519 keypair + seed starter credits so the agent can
        // transact on Agora immediately after onboarding.
        let zeus_dir = ws.parent().unwrap_or(ws);
        let wallet_dir = zeus_dir.join("wallet");
        match zeus_wallet::WalletKeypair::load_or_generate(
            &wallet_dir,
            name,
            "solana-devnet",
        ) {
            Ok(_kp) => {
                // Append [wallet] section to config.toml
                let config_path = zeus_dir.join("config.toml");
                if let Ok(existing) = std::fs::read_to_string(&config_path) {
                    if !existing.contains("[wallet]") {
                        let _ = std::fs::write(
                            &config_path,
                            format!("{}\n\n[wallet]\nenable_x402 = true\n", existing.trim_end()),
                        );
                    }
                }

                // Seed starter credits (100 ZEUS) via economy ledger
                let ledger_path = zeus_dir.join("economy").join("ledger.db");
                match zeus_economy::TokenLedger::new(&ledger_path) {
                    Ok(ledger) => {
                        let agent_id = name.to_lowercase().replace(' ', "-");
                        match ledger.mint(
                            &agent_id,
                            100,
                            zeus_economy::TransactionReason::SystemGrant,
                            "Starter credits — wallet bootstrap",
                        ) {
                            Ok(_) => info!("Seeded 100 starter credits for agent {}", agent_id),
                            Err(e) => warn!("Failed to seed starter credits: {}", e),
                        }
                    }
                    Err(e) => warn!("Failed to open economy ledger for starter credits: {}", e),
                }
            }
            Err(e) => warn!("Wallet keypair generation failed: {}", e),
        }

        self.workspace_generated = true;
    }

    /// Cycle allow_bots_mode: on → mentions → off → on
    pub fn cycle_allow_bots(&mut self) {
        self.allow_bots_mode = match self.allow_bots_mode.as_str() {
            "on" => "mentions".to_string(),
            "mentions" => "off".to_string(),
            _ => "on".to_string(),
        };
    }

    /// Toggle the currently focused skill on/off
    pub fn toggle_current_skill(&mut self) {
        // Flatten skill indices to find current position
        let mut flat_idx = 0usize;
        for (ci, cat) in self.skills.iter().enumerate() {
            for (si, _) in cat.items.iter().enumerate() {
                if flat_idx == self.sel {
                    let key = (ci, si);
                    let current = self.skill_selected.get(&key).copied().unwrap_or(false);
                    self.skill_selected.insert(key, !current);
                    return;
                }
                flat_idx += 1;
            }
        }
    }

    /// Move to next field in config steps
    pub fn next_field(&mut self) {
        match self.step {
            OnboardingStep::QuickStart => {
                self.quickstart_focus = (self.quickstart_focus + 1) % self.quickstart_fields.len().max(1);
            }
            OnboardingStep::ChanConfig => {
                // Only cycle through fields for toggled channels
                // Must match CHANNELS order: Discord(4), Telegram(2), IRC(4), Signal(3), X/Twitter(7), Pantheon(3), WhatsApp(2), Matrix(4), Slack(3), Email(4), MQTT(3), Mattermost(3)
                let field_counts: &[usize] = &[4, 2, 4, 3, 7, 3, 2, 4, 3, 4, 3, 3];
                let mut visible_indices: Vec<usize> = Vec::new();
                let mut idx = 0;
                for (chan_idx, &count) in field_counts.iter().enumerate() {
                    if self.channel_toggled.contains(&chan_idx) {
                        for i in 0..count {
                            visible_indices.push(idx + i);
                        }
                    }
                    idx += count;
                }
                if visible_indices.is_empty() { return; }
                // If policy is focused, Tab exits it (B key cycles the value)
                if self.bot_policy_focused {
                    self.bot_policy_focused = false;
                    // Move to first field of next non-Discord channel, or wrap to start
                    if !visible_indices.is_empty() {
                        self.chan_config_focus = visible_indices[0];
                    }
                    return;
                }
                let current_pos = visible_indices.iter().position(|&i| i == self.chan_config_focus);
                let next_pos = match current_pos {
                    Some(p) => (p + 1) % visible_indices.len(),
                    None => 0,
                };
                // After last Discord field, Tab goes to policy selector
                if next_pos == 0 && self.channel_toggled.contains(&0) && !self.bot_policy_focused {
                    self.bot_policy_focused = true;
                } else {
                    self.bot_policy_focused = false;
                    self.chan_config_focus = visible_indices[next_pos];
                }
            }
            OnboardingStep::Gateway => {
                self.gateway_focus = (self.gateway_focus + 1) % self.gateway_fields.len().max(1);
            }
            OnboardingStep::Voice => {
                self.voice_focus = (self.voice_focus + 1) % self.voice_fields.len().max(1);
            }
            OnboardingStep::Images => {
                self.image_focus = (self.image_focus + 1) % self.image_fields.len().max(1);
            }
            OnboardingStep::Orchestration => {
                self.orch_focus = (self.orch_focus + 1) % self.orch_fields.len().max(1);
            }
            OnboardingStep::Memory => {
                self.memory_focus = (self.memory_focus + 1) % self.memory_fields.len().max(1);
            }
            OnboardingStep::Workspace => {
                // Two path rows: 0=workspace, 1=sessions
                self.workspace_focus = (self.workspace_focus + 1) % 2;
            }
            _ => {}
        }
    }

    /// Move focus to the previous field (Shift+Tab) in config steps.
    pub fn prev_field(&mut self) {
        match self.step {
            OnboardingStep::QuickStart => {
                let len = self.quickstart_fields.len().max(1);
                self.quickstart_focus = (self.quickstart_focus + len - 1) % len;
            }
            OnboardingStep::ChanConfig => {
                let field_counts: &[usize] = &[4, 2, 4, 3, 7, 3, 2, 4, 3, 4, 3, 3];
                let mut visible_indices: Vec<usize> = Vec::new();
                let mut idx = 0;
                for (chan_idx, &count) in field_counts.iter().enumerate() {
                    if self.channel_toggled.contains(&chan_idx) {
                        for i in 0..count {
                            visible_indices.push(idx + i);
                        }
                    }
                    idx += count;
                }
                if visible_indices.is_empty() { return; }
                let discord_count = if self.channel_toggled.contains(&0) { 4usize } else { 0 };
                if self.bot_policy_focused {
                    // Shift+Tab from policy → back to last Discord field
                    self.bot_policy_focused = false;
                    if discord_count > 0 {
                        self.chan_config_focus = visible_indices[discord_count - 1];
                    }
                } else {
                    let current_pos = visible_indices.iter().position(|&i| i == self.chan_config_focus);
                    if current_pos == Some(0) && discord_count > 0 {
                        // Shift+Tab from first field → go to policy selector
                        self.bot_policy_focused = true;
                    } else {
                        let prev_pos = match current_pos {
                            Some(p) if p > 0 => p - 1,
                            _ => visible_indices.len() - 1,
                        };
                        self.bot_policy_focused = false;
                        self.chan_config_focus = visible_indices[prev_pos];
                    }
                }
            }
            OnboardingStep::Gateway => {
                let len = self.gateway_fields.len().max(1);
                self.gateway_focus = (self.gateway_focus + len - 1) % len;
            }
            OnboardingStep::Voice => {
                let len = self.voice_fields.len().max(1);
                self.voice_focus = (self.voice_focus + len - 1) % len;
            }
            OnboardingStep::Images => {
                let len = self.image_fields.len().max(1);
                self.image_focus = (self.image_focus + len - 1) % len;
            }
            OnboardingStep::Orchestration => {
                let len = self.orch_fields.len().max(1);
                self.orch_focus = (self.orch_focus + len - 1) % len;
            }
            OnboardingStep::Memory => {
                let len = self.memory_fields.len().max(1);
                self.memory_focus = (self.memory_focus + len - 1) % len;
            }
            _ => {}
        }
    }

    /// Type a character into the currently focused config field
    pub fn type_char_in_field(&mut self, c: char) {
        // Fallback step: no text input — navigation handled via Space toggle
        if self.step == OnboardingStep::Fallback { return; }
        // Agent step: sel==0 → agent_name, sel==1 → user_name, sel==2 → user_role, sel==3 → user_org, sel==4 → persona list
        if self.step == OnboardingStep::Agent {
            match self.sel {
                0 => { self.agent_name.push(c); }
                1 => { self.user_name.push(c); }
                2 => { self.user_role.push(c); }
                3 => { self.user_org.push(c); }
                _ => {}
            }
            return;
        }
        let field = match self.step {
            OnboardingStep::QuickStart => self.quickstart_fields.get_mut(self.quickstart_focus),
            OnboardingStep::ChanConfig => self.chan_config_fields.get_mut(self.chan_config_focus),
            OnboardingStep::Gateway => self.gateway_fields.get_mut(self.gateway_focus),
            OnboardingStep::Voice => self.voice_fields.get_mut(self.voice_focus),
            OnboardingStep::Images => self.image_fields.get_mut(self.image_focus),
            OnboardingStep::Orchestration => self.orch_fields.get_mut(self.orch_focus),
            OnboardingStep::Memory => self.memory_fields.get_mut(self.memory_focus),
            _ => None,
        };
        if let Some(f) = field { f.push(c); }
    }

    /// Delete last char from the currently focused config field
    pub fn delete_char_in_field(&mut self) {
        // Fallback step: Backspace removes the toggled model for the focused provider
        if self.step == OnboardingStep::Fallback {
            self.toggle_fallback_provider(); // acts as untoggle if already set
            return;
        }
        // Agent step: sel==0 → agent_name, sel==1 → user_name, sel==2 → user_role, sel==3 → user_org
        if self.step == OnboardingStep::Agent {
            match self.sel {
                0 => { self.agent_name.pop(); }
                1 => { self.user_name.pop(); }
                2 => { self.user_role.pop(); }
                3 => { self.user_org.pop(); }
                _ => {}
            }
            return;
        }
        let field = match self.step {
            OnboardingStep::QuickStart => self.quickstart_fields.get_mut(self.quickstart_focus),
            OnboardingStep::ChanConfig => self.chan_config_fields.get_mut(self.chan_config_focus),
            OnboardingStep::Gateway => self.gateway_fields.get_mut(self.gateway_focus),
            OnboardingStep::Voice => self.voice_fields.get_mut(self.voice_focus),
            OnboardingStep::Images => self.image_fields.get_mut(self.image_focus),
            OnboardingStep::Orchestration => self.orch_fields.get_mut(self.orch_focus),
            OnboardingStep::Memory => self.memory_fields.get_mut(self.memory_focus),
            _ => None,
        };
        if let Some(f) = field { f.pop(); }
    }
    /// Default backup model string for a provider.
    /// For cloud providers with known stable models, returns a sensible default.
    /// For Ollama, returns empty — user must select from locally installed models.
    fn fallback_default_model(provider_id: &str) -> &'static str {
        match provider_id {
            "anthropic"        => "anthropic/claude-haiku-4-5-20251001",
            "openai"           => "openai/gpt-4o",
            "google"           => "google/gemini-2.5-flash",
            "google-gemini-cli" => "google-gemini-cli/gemini-3-flash-preview",
            "moonshot"         => "moonshot/kimi-k2.6",
            "groq"             => "groq/llama-3.3-70b-versatile",
            "ollama"           => "", // Never hardcode — poll /api/tags for installed models
            "openrouter"       => "openrouter/anthropic/claude-3.5-haiku",
            "mistral"          => "mistral/mistral-small-latest",
            "together"         => "together/meta-llama/Meta-Llama-3.1-8B-Instruct-Turbo",
            "fireworks"        => "fireworks/accounts/fireworks/models/llama-v3p1-8b-instruct",
            "deepseek"         => "deepseek/deepseek-chat",
            "xai"              => "xai/grok-beta",
            "cerebras"         => "cerebras/llama3.1-8b",
            "zai"              => "zai/glm-4-flash",
            "qwen"             => "qwen/qwen3.5-plus",
            _                  => "", // No default — user selects from fetched models
        }
    }

    /// Toggle the currently focused non-primary provider into/out of fallback_models.
    pub fn toggle_fallback_provider(&mut self) {
        let primary_idx = self.selected_provider;
        let non_primary: Vec<usize> = (0..PROVIDERS.len())
            .filter(|&i| i != primary_idx)
            .collect();
        let Some(&prov_idx) = non_primary.get(self.fallback_focus) else { return };
        let Some(provider) = PROVIDERS.get(prov_idx) else { return };
        let prefix = format!("{}/", provider.provider_id);
        if self.fallback_models.iter().any(|m| m.starts_with(&prefix)) {
            // Remove it
            self.fallback_models.retain(|m| !m.starts_with(&prefix));
        } else {
            // Add with sensible default model
            let model = Self::fallback_default_model(provider.provider_id).to_string();
            self.fallback_models.push(model);
        }
    }
}

pub use render::render_onboarding;

/// Fetch models from a provider's API. Returns model IDs.
pub async fn fetch_models(provider_id: &str, api_key: &str) -> Result<Vec<String>, String> {
    let client = reqwest::Client::new();

    match provider_id {
        "anthropic" => {
            // OAuth setup tokens (sk-ant-oat*) require Bearer auth + anthropic-beta header.
            // Regular API keys use x-api-key header.
            let mut req = client.get("https://api.anthropic.com/v1/models")
                .header("anthropic-version", "2023-06-01");
            if api_key.starts_with("sk-ant-oat") {
                req = req.header("Authorization", format!("Bearer {}", api_key))
                    .header("anthropic-beta", "oauth-2025-04-20");
            } else {
                req = req.header("x-api-key", api_key);
            }
            let resp = req.send().await;
            match resp {
                Ok(r) if r.status().is_success() => {
                    let body: serde_json::Value = r.json().await.map_err(|e| e.to_string())?;
                    let models: Vec<String> = body["data"].as_array()
                        .map(|arr| arr.iter()
                            .filter_map(|m| m["id"].as_str().map(String::from))
                            .collect())
                        .unwrap_or_default();
                    if models.is_empty() {
                        Err("Anthropic API returned no models".into())
                    } else {
                        Ok(models)
                    }
                }
                _ => {
                    Err("Could not reach Anthropic API — check your API key".into())
                }
            }
        }
        "openai" => {
            let is_oauth_token = !api_key.starts_with("sk-");
            if is_oauth_token {
                // OAuth token — try Codex backend for model list, fall back to curated list
                let codex_resp = client.get("https://chatgpt.com/backend-api/models")
                    .bearer_auth(api_key)
                    .send().await;
                if let Ok(resp) = codex_resp {
                    if resp.status().is_success() {
                        if let Ok(body) = resp.json::<serde_json::Value>().await {
                            let models: Vec<String> = body["models"].as_array()
                                .map(|arr| arr.iter()
                                    .filter_map(|m| m["slug"].as_str().map(String::from))
                                    // Filter out web-UI-only and Codex-incompatible variants
                                    .filter(|s| {
                                        let is_model = s.starts_with("gpt-") || s.starts_with("o3") || s.starts_with("o4");
                                        let is_excluded = s.contains("thinking") || s.contains("research")
                                            || s.contains("agent-mode") || s.contains("instant")
                                            || s.contains("t-mini");
                                        is_model && !is_excluded
                                    })
                                    // Fix version format: API slugs use dashes for version numbers
                                    // (gpt-5-4) but Codex endpoint needs dots (gpt-5.4).
                                    // Only applies when char after "gpt-5-" is a digit.
                                    // gpt-5-mini stays gpt-5-mini (not a version number).
                                    .map(|s| {
                                        if let Some(rest) = s.strip_prefix("gpt-5-") {
                                            if rest.starts_with(|c: char| c.is_ascii_digit()) {
                                                if let Some((ver, suffix)) = rest.split_once('-') {
                                                    format!("gpt-5.{}-{}", ver, suffix)
                                                } else {
                                                    format!("gpt-5.{}", rest)
                                                }
                                            } else {
                                                s // gpt-5-mini, gpt-5-turbo etc — keep as-is
                                            }
                                        } else {
                                            s
                                        }
                                    })
                                    .collect())
                                .unwrap_or_default();
                            if !models.is_empty() {
                                // Ensure key models appear even if API omits them
                                let mut models = models;
                                let ensure = ["gpt-5.4", "gpt-5.4-mini"];
                                for m in ensure {
                                    if !models.iter().any(|x| x == m) {
                                        models.insert(0, m.to_string());
                                    }
                                }
                                return Ok(models);
                            }
                        }
                    }
                }
                // Codex backend doesn't expose model listing — use curated fallback.
                // These work via Codex OAuth (chatgpt.com/backend-api/codex)
                return Ok(vec![
                    "gpt-5.4".to_string(),
                    "gpt-5.4-mini".to_string(),
                    "gpt-5".to_string(),
                    "o3".to_string(),
                    "o4-mini".to_string(),
                ]);
            }
            // API key path — fetch from OpenAI models endpoint
            let resp = client.get("https://api.openai.com/v1/models")
                .bearer_auth(api_key)
                .send().await.map_err(|e| e.to_string())?;
            if !resp.status().is_success() {
                return Err(format!("OpenAI API error: {}", resp.status()));
            }
            let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
            let mut models: Vec<String> = body["data"].as_array()
                .map(|arr| arr.iter()
                    .filter_map(|m| m["id"].as_str().map(String::from))
                    .filter(|id| {
                        // Keep only chat-capable models
                        let is_chat = id.starts_with("gpt-") || id.starts_with("o1") || id.starts_with("o3") || id.starts_with("o4");
                        // Exclude non-chat and noisy variants
                        let is_excluded = id.contains("audio") || id.contains("realtime")
                            || id.contains("tts") || id.contains("transcribe")
                            || id.contains("image") || id.contains("search")
                            || id.contains("diarize") || id.contains("deep-research")
                            || id.contains("instruct") || id.contains("vision")
                            // Exclude pinned dated versions (e.g. gpt-4o-2024-05-13)
                            || id.contains("-20")
                            // Exclude very old series
                            || id.starts_with("gpt-3") || id.starts_with("gpt-4-");
                        is_chat && !is_excluded
                    })
                    .collect())
                .unwrap_or_default();
            // Sort: gpt-5.x first, then o-series, then gpt-4.x
            models.sort_by(|a, b| {
                let rank = |s: &str| -> u8 {
                    if s.starts_with("gpt-5") { 0 }
                    else if s.starts_with("o4") || s.starts_with("o3") { 1 }
                    else if s.starts_with("o1") { 2 }
                    else { 3 }
                };
                rank(a).cmp(&rank(b)).then(b.cmp(a)) // within tier: newest first
            });
            if models.is_empty() {
                Err("OpenAI API returned no models".into())
            } else {
                Ok(models)
            }
        }
        "ollama" => {
            // api_key holds the Ollama URL when provider is ollama
            let base = if !api_key.is_empty() && api_key.starts_with("http") {
                api_key.to_string()
            } else {
                std::env::var("OLLAMA_HOST").unwrap_or_else(|_| "http://localhost:11434".into())
            };
            // Detect remote endpoints: anything not on localhost/127.0.0.1
            let is_remote_host = !base.contains("localhost") && !base.contains("127.0.0.1");

            let resp = client.get(format!("{}/api/tags", base))
                .send().await.map_err(|e| e.to_string())?;
            if !resp.status().is_success() {
                let status = resp.status();
                if status.as_u16() == 401 && is_remote_host {
                    return Err(format!(
                        "Remote Ollama at {} requires sign-in — visit {} to authenticate.",
                        base, base
                    ));
                }
                return Err(format!("Ollama API error: {}", status));
            }
            let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;

            // Collect model names first, then enrich with /api/show capabilities
            let raw_models: Vec<(String, f64, String, String)> = body["models"].as_array()
                .map(|arr| arr.iter()
                    .filter_map(|m| {
                        let name = m["name"].as_str()?.to_string();
                        let size_bytes = m["size"].as_u64().unwrap_or(0);
                        let size_gb = size_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
                        let params = m["details"]["parameter_size"].as_str().unwrap_or("").to_string();
                        let quant = m["details"]["quantization_level"].as_str().unwrap_or("").to_string();
                        Some((name, size_gb, params, quant))
                    })
                    .collect())
                .unwrap_or_default();

            // Enrich with capabilities from /api/show — concurrent (up to 8 parallel),
            // 3s timeout per query, capped at 200 models (mirrors OpenClaw's approach).
            let enrich_client = std::sync::Arc::new(reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(3))
                .build()
                .unwrap_or_default());
            // Collect owned tuples (capped at 200) before spawning concurrent futures
            let models_to_enrich: Vec<(String, f64, String, String)> = raw_models
                .iter()
                .take(200)
                .cloned()
                .collect();
            let base_arc = std::sync::Arc::new(base.clone());

            use futures_util::stream::{self, StreamExt};
            let enriched: Vec<String> = stream::iter(models_to_enrich)
                .map(|(name, size_gb, params, quant)| {
                    let client = enrich_client.clone();
                    let base = base_arc.clone();
                    async move {
                        let caps = zeus_llm::ollama::get_cached_model_capabilities(
                            &client, &base, &name
                        ).await;
                        let mut flags = String::new();
                        if let Some(ref c) = caps {
                            if c.supports_tools { flags.push_str(" [tools]"); }
                            if c.supports_vision { flags.push_str(" [vision]"); }
                            if let Some(ctx) = c.context_length {
                                flags.push_str(&format!(" {}k", ctx / 1024));
                            }
                        }
                        format!("{} ({:.1}GB, {}, {}{})", name, size_gb, params, quant, flags)
                    }
                })
                .buffer_unordered(8)
                .collect()
                .await;

            let models: Vec<String> = enriched;
            if models.is_empty() {
                Err("Ollama returned no models — is it running?".into())
            } else {
                Ok(models)
            }
        }
        "google" => {
            // Try dynamic fetch — works with API keys, may work with some OAuth tokens
            let is_oauth = api_key.starts_with("ya29.") || api_key.starts_with("ey");
            let gemini_fallback = || vec![
                "gemini-3-flash-preview".to_string(),
                "gemini-3.1-pro-preview".to_string(),
                "gemini-2.5-flash".to_string(),
                "gemini-2.5-pro".to_string(),
            ];
            if is_oauth {
                // OAuth token — try Bearer auth against generativelanguage API
                let oauth_resp = client.get("https://generativelanguage.googleapis.com/v1beta/models")
                    .bearer_auth(api_key)
                    .send().await;
                if let Ok(resp) = oauth_resp {
                    if resp.status().is_success() {
                        if let Ok(body) = resp.json::<serde_json::Value>().await {
                            let models: Vec<String> = body["models"].as_array()
                                .map(|arr| arr.iter()
                                    .filter_map(|m| m["name"].as_str()
                                        .and_then(|n| n.strip_prefix("models/"))
                                        .map(String::from))
                                    .filter(|id| id.contains("gemini"))
                                    .collect())
                                .unwrap_or_default();
                            if !models.is_empty() {
                                return Ok(models);
                            }
                        }
                    }
                }
                // OAuth dynamic fetch failed — use curated fallback
                return Ok(gemini_fallback());
            }
            // API key path
            let resp = client.get(format!("https://generativelanguage.googleapis.com/v1beta/models?key={}", api_key))
                .send().await.map_err(|e| e.to_string())?;
            if !resp.status().is_success() {
                return Err(format!("Google API error: {}", resp.status()));
            }
            let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
            let models: Vec<String> = body["models"].as_array()
                .map(|arr| arr.iter()
                    .filter_map(|m| m["name"].as_str()
                        .and_then(|n| n.strip_prefix("models/"))
                        .map(String::from))
                    .filter(|id| id.contains("gemini"))
                    .collect())
                .unwrap_or_default();
            if models.is_empty() {
                Err("Google API returned no Gemini models".into())
            } else {
                Ok(models)
            }
        }

        "google-gemini-cli" => {
            // OAuth token — use Code Assist v1internal:listModels endpoint
            let project_id = match zeus_auth::discover_google_project(api_key).await {
                Ok(id) => id,
                // Static catalogue from gemini-cli models.ts VALID_GEMINI_MODELS
                Err(_) => return Ok(vec![
                    "gemini-3.1-pro-preview".to_string(),
                    "gemini-3.1-flash-lite-preview".to_string(),
                    "gemini-3-pro-preview".to_string(),
                    "gemini-3-flash-preview".to_string(),
                    "gemini-2.5-pro".to_string(),
                    "gemini-2.5-flash".to_string(),
                    "gemini-2.5-flash-lite".to_string(),
                ]),
            };
            let resp = client
                .post("https://cloudcode-pa.googleapis.com/v1internal:listModels")
                .bearer_auth(api_key)
                .header("Content-Type", "application/json")
                .json(&serde_json::json!({
                    "project": project_id,
                    "metadata": {
                        "ideType": "ANTIGRAVITY",
                        "platform": "PLATFORM_UNSPECIFIED",
                        "pluginType": "GEMINI",
                    }
                }))
                .send()
                .await
                .map_err(|e| e.to_string())?;
            // Static catalogue from gemini-cli models.ts VALID_GEMINI_MODELS
            let gemini_fallback = || vec![
                "gemini-3.1-pro-preview".to_string(),
                "gemini-3.1-flash-lite-preview".to_string(),
                "gemini-3-pro-preview".to_string(),
                "gemini-3-flash-preview".to_string(),
                "gemini-2.5-pro".to_string(),
                "gemini-2.5-flash".to_string(),
                "gemini-2.5-flash-lite".to_string(),
            ];
            if !resp.status().is_success() {
                return Ok(gemini_fallback());
            }
            let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
            // Code Assist may wrap in {"models": [...]} or {"response": {"models": [...]}}
            let models_val = body.get("models")
                .or_else(|| body.get("response").and_then(|r| r.get("models")));
            let models: Vec<String> = models_val
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter()
                    .filter_map(|m| {
                        m["name"].as_str()
                            .and_then(|n| n.strip_prefix("models/"))
                            .or_else(|| m["name"].as_str())
                            .or_else(|| m.as_str())
                            .map(String::from)
                    })
                    .filter(|id| id.contains("gemini"))
                    .collect())
                .unwrap_or_default();
            if models.is_empty() {
                Ok(gemini_fallback())
            } else {
                Ok(models)
            }
        }
        "groq" => {
            let resp = client.get("https://api.groq.com/openai/v1/models")
                .bearer_auth(api_key)
                .send().await.map_err(|e| e.to_string())?;
            if !resp.status().is_success() {
                return Err(format!("Groq API error: {}", resp.status()));
            }
            let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
            let models: Vec<String> = body["data"].as_array()
                .map(|arr| arr.iter()
                    .filter_map(|m| m["id"].as_str().map(String::from))
                    .collect())
                .unwrap_or_default();
            if models.is_empty() {
                Err("Groq API returned no models".into())
            } else {
                Ok(models)
            }
        }
        "openrouter" => {
            let resp = client.get("https://openrouter.ai/api/v1/models")
                .bearer_auth(api_key)
                .send().await.map_err(|e| e.to_string())?;
            if !resp.status().is_success() {
                return Err(format!("OpenRouter API error: {}", resp.status()));
            }
            let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
            let models: Vec<String> = body["data"].as_array()
                .map(|arr| arr.iter()
                    .filter_map(|m| m["id"].as_str().map(String::from))
                    .take(20) // Top 20, list is huge
                    .collect())
                .unwrap_or_default();
            if models.is_empty() {
                Err("OpenRouter API returned no models".into())
            } else {
                Ok(models)
            }
        }
        "mistral" => {
            let resp = client.get("https://api.mistral.ai/v1/models")
                .bearer_auth(api_key)
                .send().await.map_err(|e| e.to_string())?;
            if !resp.status().is_success() {
                return Err(format!("Mistral API error: {}", resp.status()));
            }
            let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
            let models: Vec<String> = body["data"].as_array()
                .map(|arr| arr.iter()
                    .filter_map(|m| m["id"].as_str().map(String::from))
                    .collect())
                .unwrap_or_default();
            if models.is_empty() {
                Err("Mistral API returned no models".into())
            } else {
                Ok(models)
            }
        }
        "together" => {
            let resp = client.get("https://api.together.xyz/v1/models")
                .bearer_auth(api_key)
                .send().await.map_err(|e| e.to_string())?;
            if !resp.status().is_success() {
                return Err(format!("Together API error: {}", resp.status()));
            }
            let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
            let models: Vec<String> = body.as_array()
                .map(|arr| arr.iter()
                    .filter_map(|m| m["id"].as_str().map(String::from))
                    .collect())
                .unwrap_or_default();
            if models.is_empty() {
                Err("Together API returned no models".into())
            } else {
                Ok(models)
            }
        }
        "fireworks" => {
            let resp = client.get("https://api.fireworks.ai/inference/v1/models")
                .bearer_auth(api_key)
                .send().await.map_err(|e| e.to_string())?;
            if !resp.status().is_success() {
                return Err(format!("Fireworks API error: {}", resp.status()));
            }
            let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
            let models: Vec<String> = body["data"].as_array()
                .map(|arr| arr.iter()
                    .filter_map(|m| m["id"].as_str().map(String::from))
                    .collect())
                .unwrap_or_default();
            if models.is_empty() {
                Err("Fireworks API returned no models".into())
            } else {
                Ok(models)
            }
        }
        "azure" => {
            // Azure OpenAI: endpoint is per-deployment, needs AZURE_OPENAI_ENDPOINT env var
            let endpoint = std::env::var("AZURE_OPENAI_ENDPOINT")
                .unwrap_or_default();
            if endpoint.is_empty() {
                return Err("Set AZURE_OPENAI_ENDPOINT to your Azure resource URL".into());
            }
            let url = format!("{}/openai/deployments?api-version=2024-02-01", endpoint.trim_end_matches('/'));
            let resp = client.get(&url)
                .header("api-key", api_key)
                .send().await.map_err(|e| e.to_string())?;
            if !resp.status().is_success() {
                return Err(format!("Azure API error: {}", resp.status()));
            }
            let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
            let models: Vec<String> = body["value"].as_array()
                .map(|arr| arr.iter()
                    .filter_map(|m| m["id"].as_str().map(String::from))
                    .collect())
                .unwrap_or_default();
            if models.is_empty() {
                Err("Azure returned no deployments — check your endpoint and key".into())
            } else {
                Ok(models)
            }
        }
        "bedrock" => {
            // AWS Bedrock: uses AWS SDK — fall back to known foundation models list
            // Full SDK auth is complex for a TUI; return curated list of common models.
            Ok(vec![
                "anthropic.claude-3-5-sonnet-20241022-v2:0".into(),
                "anthropic.claude-3-5-haiku-20241022-v1:0".into(),
                "anthropic.claude-3-opus-20240229-v1:0".into(),
                "anthropic.claude-3-sonnet-20240229-v1:0".into(),
                "amazon.nova-pro-v1:0".into(),
                "amazon.nova-lite-v1:0".into(),
                "amazon.nova-micro-v1:0".into(),
                "meta.llama3-70b-instruct-v1:0".into(),
                "meta.llama3-8b-instruct-v1:0".into(),
                "mistral.mistral-large-2402-v1:0".into(),
            ])
        }
        "deepseek" => {
            // DeepSeek — OpenAI-compatible models endpoint
            let resp = client.get("https://api.deepseek.com/v1/models")
                .bearer_auth(api_key)
                .send().await.map_err(|e| e.to_string())?;
            if !resp.status().is_success() {
                return Err(format!("DeepSeek API error: {}", resp.status()));
            }
            let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
            let models: Vec<String> = body["data"].as_array()
                .map(|arr| arr.iter()
                    .filter_map(|m| m["id"].as_str().map(String::from))
                    .collect())
                .unwrap_or_default();
            if models.is_empty() {
                Err("DeepSeek API returned no models".into())
            } else {
                Ok(models)
            }
        }
        "xai" => {
            // xAI / Grok — OpenAI-compatible models endpoint
            let resp = client.get("https://api.x.ai/v1/models")
                .bearer_auth(api_key)
                .send().await.map_err(|e| e.to_string())?;
            if !resp.status().is_success() {
                return Err(format!("xAI API error: {}", resp.status()));
            }
            let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
            let models: Vec<String> = body["data"].as_array()
                .map(|arr| arr.iter()
                    .filter_map(|m| m["id"].as_str().map(String::from))
                    .collect())
                .unwrap_or_default();
            if models.is_empty() {
                Err("xAI API returned no models".into())
            } else {
                Ok(models)
            }
        }
        "cerebras" => {
            // Cerebras — OpenAI-compatible models endpoint
            let resp = client.get("https://api.cerebras.ai/v1/models")
                .bearer_auth(api_key)
                .send().await.map_err(|e| e.to_string())?;
            if !resp.status().is_success() {
                return Err(format!("Cerebras API error: {}", resp.status()));
            }
            let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
            let models: Vec<String> = body["data"].as_array()
                .map(|arr| arr.iter()
                    .filter_map(|m| m["id"].as_str().map(String::from))
                    .collect())
                .unwrap_or_default();
            if models.is_empty() {
                Err("Cerebras API returned no models".into())
            } else {
                Ok(models)
            }
        }
        "moonshot" => {
            // Kimi/Moonshot — try live /v1/models, fall back to static catalog
            let resp = client
                .get("https://api.moonshot.cn/v1/models")
                .bearer_auth(api_key)
                .send()
                .await;
            if let Ok(r) = resp {
                if r.status().is_success() {
                    if let Ok(body) = r.json::<serde_json::Value>().await {
                        let live: Vec<String> = body["data"].as_array()
                            .map(|arr| arr.iter()
                                .filter_map(|m| m["id"].as_str().map(String::from))
                                .collect())
                            .unwrap_or_default();
                        if !live.is_empty() { return Ok(live); }
                    }
                }
            }
            Ok(vec![
                "kimi-k2.6".to_string(),
                "kimi-k2.5".to_string(),
                "kimi-k2-thinking".to_string(),
                "kimi-k2-turbo".to_string(),
            ])
        }
        "zai" => {
            // ZAI (Zhipu AI) GLM — OpenAI-compatible endpoint at BigModel.
            // Try /v4/models first; fall back to static catalog if unavailable.
            let resp = client
                .get("https://open.bigmodel.cn/api/paas/v4/models")
                .bearer_auth(api_key)
                .send()
                .await;
            if let Ok(r) = resp {
                if r.status().is_success() {
                    if let Ok(body) = r.json::<serde_json::Value>().await {
                        let live: Vec<String> = body["data"].as_array()
                            .map(|arr| arr.iter()
                                .filter_map(|m| m["id"].as_str().map(String::from))
                                .collect())
                            .unwrap_or_default();
                        if !live.is_empty() {
                            return Ok(live);
                        }
                    }
                }
            }
            // Static fallback catalog — GLM-5 series + legacy
            Ok(vec![
                "glm-5.1".to_string(),
                "glm-5".to_string(),
                "glm-5-turbo".to_string(),
                "glm-4.7-flash".to_string(),
                "glm-4.6".to_string(),
                "glm-4-plus".to_string(),
                "glm-4-flash".to_string(),
            ])
        }
        "qwen" => {
            // Qwen / Alibaba DashScope — bundled catalog from the OpenAI-compatible
            // endpoint. The LlmClient's `resolve_qwen_base_url` picks the correct
            // host (Standard/Coding × CN/Global) based on QWEN_REGION + QWEN_PLAN
            // env vars; we mirror its endpoint-aware filter here so the onboarding
            // dropdown only offers models that actually work on the selected plan.
            //
            // Accepts QWEN_API_KEY, DASHSCOPE_API_KEY, or MODELSTUDIO_API_KEY per
            // the 3-tier compat chain in resolve_qwen_api_key().
            let base_url = zeus_llm::resolve_qwen_base_url();
            let filtered = zeus_llm::qwen_filtered_catalog(&base_url);
            let models: Vec<String> = filtered.iter().map(|m| m.id.to_string()).collect();
            if models.is_empty() {
                // Safety net: if an unusual QWEN_BASE_URL override filters
                // out everything (shouldn't happen with the fail-open rule
                // in qwen_filtered_catalog, but belt-and-suspenders), fall
                // back to the flagship default.
                Ok(vec!["qwen3.5-plus".to_string()])
            } else {
                // Try the live /models endpoint first; the bundled catalog is
                // a fallback. DashScope's OpenAI-compat endpoint honors the
                // standard /v1/models listing.
                let url = format!("{}/models", base_url.trim_end_matches('/'));
                let resp = client.get(&url).bearer_auth(api_key).send().await;
                if let Ok(r) = resp {
                    if r.status().is_success() {
                        if let Ok(body) = r.json::<serde_json::Value>().await {
                            let live: Vec<String> = body["data"].as_array()
                                .map(|arr| arr.iter()
                                    .filter_map(|m| m["id"].as_str().map(String::from))
                                    .collect())
                                .unwrap_or_default();
                            if !live.is_empty() {
                                return Ok(live);
                            }
                        }
                    }
                }
                Ok(models)
            }
        }
        "minimax" => {
            // MiniMax — try model listing; fall back to static catalog.
            let resp = client
                .get("https://api.minimax.io/v1/models")
                .bearer_auth(api_key)
                .send()
                .await;
            if let Ok(r) = resp {
                if r.status().is_success() {
                    if let Ok(body) = r.json::<serde_json::Value>().await {
                        let live: Vec<String> = body["data"].as_array()
                            .map(|arr| arr.iter()
                                .filter_map(|m| m["id"].as_str().map(String::from))
                                .collect())
                            .unwrap_or_default();
                        if !live.is_empty() {
                            return Ok(live);
                        }
                    }
                }
            }
            // Static fallback — current MiniMax model family (per molty's docs check)
            Ok(vec![
                "MiniMax-M2.7".to_string(),
                "MiniMax-M2.7-highspeed".to_string(),
                "MiniMax-M2.5".to_string(),
                "MiniMax-M2.5-highspeed".to_string(),
                "MiniMax-M2.1".to_string(),
            ])
        }
        "xiaomimimo" => {
            // Xiaomi MiMo — no public models endpoint; static catalog of canonical IDs.
            Ok(vec![
                "mimo-v2.5-pro".to_string(),
                "mimo-v2.5".to_string(),
                "mimo-v2-pro".to_string(),
                "mimo-v2-omni".to_string(),
                "mimo-v2-flash".to_string(),
            ])
        }
        // No standard models endpoint for this provider
        _ => {
            Err(format!("No models endpoint available for {}. Enter model name manually.", provider_id))
        }
    }
}

// ── Signal QR pairing helper ──────────────────────────────────────────────────

/// Run `signal-cli link -n "zeus"` to get a tsdevice:// pairing URI.
/// This adds Zeus as a secondary (linked) device — safe, no registration needed,
/// does NOT displace Signal on the user's phone.
/// The URI is emitted immediately on stdout; the process stays running until
/// the phone scans it, so we read the first matching line and leave it running.
pub async fn fetch_signal_qr_uri(
    cli_path: &str,
    _phone: &str,
    _http_port: u16,
) -> Result<(String, tokio::process::Child), String> {
    use std::time::Duration;
    use tokio::io::{AsyncBufReadExt, BufReader};

    let mut child = tokio::process::Command::new(cli_path)
        .arg("link")
        .arg("-n")
        .arg("zeus")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        // Don't kill child when handle is dropped — it must outlive this function
        .kill_on_drop(false)
        .spawn()
        .map_err(|e| format!("Failed to run signal-cli: {}. Is signal-cli installed?", e))?;

    // Read stdout/stderr line by line looking for the tsdevice:// or sgnl:// URI.
    // We take() the pipes so we can read them, then return the child handle to the
    // caller so the process stays alive for the full phone-side handshake.
    let stdout = child.stdout.take().ok_or("no stdout from signal-cli")?;
    let stderr = child.stderr.take().ok_or("no stderr from signal-cli")?;

    let mut stdout_lines = BufReader::new(stdout).lines();
    let mut stderr_lines = BufReader::new(stderr).lines();

    // signal-cli link outputs the URI within a second or two
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        if tokio::time::Instant::now() >= deadline {
            let _ = child.kill().await;
            return Err("signal-cli did not output a pairing URI within 15s. Is it installed correctly?".to_string());
        }
        tokio::select! {
            line = stdout_lines.next_line() => {
                match line {
                    Ok(Some(l)) => {
                        let trimmed = l.trim().to_string();
                        if trimmed.starts_with("tsdevice://") || trimmed.starts_with("sgnl://") {
                            // Drain stdout in a background task so the pipe stays open.
                            // If we drop stdout_lines here, signal-cli gets SIGPIPE when it
                            // writes "Associated with..." after the phone scan completes.
                            tokio::spawn(async move {
                                while let Ok(Some(_)) = stdout_lines.next_line().await {}
                            });
                            tokio::spawn(async move {
                                while let Ok(Some(_)) = stderr_lines.next_line().await {}
                            });
                            // Return the child handle — caller MUST hold it until pairing is done.
                            return Ok((trimmed, child));
                        }
                        if trimmed.to_lowercase().contains("not found")
                            || trimmed.to_lowercase().contains("error")
                        {
                            let _ = child.kill().await;
                            return Err(format!("signal-cli error: {}", trimmed));
                        }
                    }
                    Ok(None) => break,
                    Err(_) => break,
                }
            }
            line = stderr_lines.next_line() => {
                if let Ok(Some(l)) = line {
                    let trimmed = l.trim().to_string();
                    if trimmed.starts_with("tsdevice://") || trimmed.starts_with("sgnl://") {
                        tokio::spawn(async move {
                            while let Ok(Some(_)) = stdout_lines.next_line().await {}
                        });
                        tokio::spawn(async move {
                            while let Ok(Some(_)) = stderr_lines.next_line().await {}
                        });
                        return Ok((trimmed, child));
                    }
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(100)) => {}
        }
    }

    let _ = child.kill().await;
    Err("signal-cli link exited without producing a pairing URI. Check that signal-cli is installed and up to date.".to_string())
}

// ── WhatsApp QR pairing helper ────────────────────────────────────────────────

/// Connect to the Baileys WebSocket bridge and wait for a { "type": "qr", "qr": "..." }
/// message. The bridge emits this when WhatsApp Web needs device linking.
/// Returns the raw QR string which can be rendered as a terminal QR code.
pub async fn fetch_whatsapp_qr(bridge_url: &str) -> Result<String, String> {
    use tokio_tungstenite::connect_async;
    use futures_util::StreamExt;
    use std::time::Duration;

    // Convert http(s):// to ws(s):// if needed
    let ws_url = if bridge_url.starts_with("http://") {
        bridge_url.replacen("http://", "ws://", 1)
    } else if bridge_url.starts_with("https://") {
        bridge_url.replacen("https://", "wss://", 1)
    } else {
        bridge_url.to_string()
    };

    let (ws_stream, _) = tokio::time::timeout(
        Duration::from_secs(15),
        connect_async(&ws_url),
    )
    .await
    .map_err(|_| format!("Timed out connecting to WhatsApp bridge at {}", ws_url))?
    .map_err(|e| format!("Could not connect to WhatsApp bridge: {}. Is the Baileys bridge running?", e))?;

    let (_, mut read) = ws_stream.split();

    // Wait up to 30s for the bridge to emit a QR message
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        if tokio::time::Instant::now() >= deadline {
            return Err("Timed out waiting for QR from WhatsApp bridge (30s). Bridge connected but no QR received.".to_string());
        }
        match tokio::time::timeout(Duration::from_secs(5), read.next()).await {
            Ok(Some(Ok(msg))) => {
                let text = match msg {
                    tokio_tungstenite::tungstenite::Message::Text(t) => t,
                    _ => continue,
                };
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&text) {
                    match val.get("type").and_then(|t| t.as_str()) {
                        Some("qr") => {
                            let qr = val.get("qr")
                                .or_else(|| val.get("data"))
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string())
                                .ok_or_else(|| "Bridge sent QR message but no qr/data field found".to_string())?;
                            return Ok(qr);
                        }
                        Some("ready") => {
                            return Err("Bridge is already linked — no QR needed. Press Enter to continue.".to_string());
                        }
                        _ => continue,
                    }
                }
            }
            Ok(Some(Err(e))) => return Err(format!("Bridge WebSocket error: {}", e)),
            Ok(None) => return Err("Bridge WebSocket closed before sending QR.".to_string()),
            Err(_) => continue, // timeout, loop back to check deadline
        }
    }
}
