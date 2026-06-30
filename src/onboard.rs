#![allow(dead_code)]
//! Zeus Onboarding Wizard - Interactive CLI setup for first-time users
//!
//! Two modes:
//! 1. **Conversational** (default): LLM-driven natural language setup via `ConversationalOnboarding`.
//!    Detects provider from env/context, asks for API key naturally, auto-detects Ollama,
//!    recommends model based on use case, completes in under 5 exchanges.
//! 2. **Classic**: Linear numbered-menu wizard (`run_onboard`).
//!
//! Features:
//! - Detects existing OpenClaw configs and offers to import API keys
//! - Detects API keys already set in the environment and suggests providers
//! - Auto-detects local Ollama with available models
//! - LLM-driven conversational config extraction
//! - Service setup: image gen, voice (STT/TTS), video gen — all provider-agnostic
//! - `--check` mode: doctor scan of configured vs missing services

use anyhow::Result;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use zeus_core::Config;

/// Well-known OpenClaw config locations
const OPENCLAW_CONFIG_PATHS: &[&str] = &[
    ".openclaw/config.toml",
    ".claw/config.toml",
    ".openclaw/config.json",
];

/// Run the interactive onboarding wizard.
pub fn run_onboard() -> Result<()> {
    println!("=== Zeus Setup Wizard ===\n");

    // Phase 0a: Detect OpenClaw configs
    if let Some((path, keys)) = detect_openclaw_config() {
        println!("Found OpenClaw config at: {}", path.display());
        println!("  Detected API keys:");
        for (provider, env_var) in &keys {
            println!("    {} ({})", provider, env_var);
        }
        print!("\nImport these API keys into your environment? (Y/n): ");
        io::stdout().flush()?;
        let mut answer = String::new();
        io::stdin().lock().read_line(&mut answer)?;
        if !answer.trim().eq_ignore_ascii_case("n") {
            let imported = import_openclaw_keys(&path);
            if imported > 0 {
                println!("  Imported {} API key(s) into ~/.zeus/.env", imported);
            }
        }
        println!();
    }

    // Phase 0b: Detect environment API keys and suggest provider
    let detected = Config::detect_environment();
    if !detected.is_empty() {
        println!("Detected API keys in environment:");
        for (provider, env_var) in &detected {
            println!("  {:?} ({})", provider, env_var);
        }
        if let Some((suggested_provider, suggested_model)) = Config::suggest_provider() {
            println!(
                "\nSuggested provider: {:?} -> {}",
                suggested_provider, suggested_model
            );
            print!("Use this provider? (Y/n): ");
            io::stdout().flush()?;
            let mut answer = String::new();
            io::stdin().lock().read_line(&mut answer)?;
            if !answer.trim().eq_ignore_ascii_case("n") {
                return run_quick_setup(suggested_model);
            }
        }
        println!();
    }

    // 1. Select LLM provider
    println!("Select LLM provider:");
    println!("  1. Anthropic (Claude)");
    println!("  2. OpenAI (GPT)");
    println!("  3. Ollama (local)");
    println!("  4. OpenRouter");
    println!("  5. Mistral AI");
    println!("  6. Together AI");
    println!("  7. Fireworks AI");
    println!("  8. Azure OpenAI");
    println!("  9. AWS Bedrock");
    let provider = read_choice(1, 9)?;

    // 2. Auth method (Anthropic supports OAuth)
    let use_oauth = if provider == 1 {
        println!("\nAuth method:");
        println!("  1) OAuth token   (from console.anthropic.com)");
        println!("  2) API key       (sk-ant-api03-...)");
        read_choice(1, 2)? == 1
    } else {
        false
    };

    // 3. Enter API key / OAuth token (skip for Ollama)
    let api_key = if provider != 3 {
        if use_oauth {
            print!("OAuth token: ");
        } else {
            print!("Enter API key: ");
        }
        io::stdout().flush()?;
        let mut key = String::new();
        io::stdin().lock().read_line(&mut key)?;
        let trimmed = key.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    } else {
        None
    };

    // 3. Select model
    let model = match provider {
        1 => select_anthropic_model()?,
        2 => select_openai_model()?,
        3 => select_ollama_model()?,
        4 => select_openrouter_model()?,
        5 => select_mistral_model()?,
        6 => select_together_model()?,
        7 => select_fireworks_model()?,
        8 => select_azure_model()?,
        9 => select_bedrock_model()?,
        _ => unreachable!(),
    };

    // 4. Agent identity
    println!("\n── Agent Identity ──────────────────────────────────");

    // Auto-detect name from hostname
    let hostname = std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default();
    let hostname = hostname.trim().to_string();
    let detected_name = if hostname.is_empty() { "zeus".to_string() } else { hostname.clone() };
    print!("Agent name [{}]: ", detected_name);
    io::stdout().flush()?;
    let mut agent_name_input = String::new();
    io::stdin().lock().read_line(&mut agent_name_input)?;
    let agent_name = {
        let t = agent_name_input.trim().to_string();
        if t.is_empty() { detected_name.clone() } else { t }
    };

    print!("Agent role (e.g. Backend developer, Security auditor): ");
    io::stdout().flush()?;
    let mut agent_role = String::new();
    io::stdin().lock().read_line(&mut agent_role)?;
    let agent_role = {
        let t = agent_role.trim().to_string();
        if t.is_empty() { "Zeus fleet agent".to_string() } else { t }
    };

    // Persona picker — load from ~/.zeus/agents/ (cloned from anthropics/skills)
    let (agent_soul, agent_system_prompt) = pick_persona_and_skills(&agent_name)?;

    print!("Coordinator agent ID (e.g. Zeus100, leave blank to skip): ");
    io::stdout().flush()?;
    let mut coordinator_input = String::new();
    io::stdin().lock().read_line(&mut coordinator_input)?;
    let agent_coordinator = {
        let t = coordinator_input.trim().to_string();
        if t.is_empty() { None } else { Some(t) }
    };

    // 5. Workspace path
    print!("Workspace path [~/.zeus/workspace]: ");
    io::stdout().flush()?;
    let mut workspace = String::new();
    io::stdin().lock().read_line(&mut workspace)?;
    let workspace = workspace.trim();
    let workspace = if workspace.is_empty() {
        "~/.zeus/workspace"
    } else {
        workspace
    };

    // 6. Channel setup (per-channel credential prompts)
    let channels = setup_channels()?;

    // 7. Optional services: image gen, voice, video gen
    let services = setup_optional_services()?;

    // 8. Generate config.toml
    let mut config =
        generate_config(provider, api_key.as_deref(), &model, workspace, &services, use_oauth);
    let channel_toml = generate_channel_config(&channels);
    if !channel_toml.is_empty() {
        config.push_str(&channel_toml);
        config.push('\n');
    }

    // #213: persist agent identity to config.toml so identity regen
    // (deploy-identity.sh, gateway bootstrap) derives from config instead of
    // falling back to hardcoded fleet values.
    config.push_str(&format!(
        "\n[agent]\nname = \"{}\"\nrole = \"{}\"\n",
        agent_name.replace('"', "\\\""),
        agent_role.replace('"', "\\\"")
    ));
    if let Some(ref coord) = agent_coordinator {
        config.push_str(&format!("coordinator = \"{}\"\n", coord.replace('"', "\\\"")));
    }

    // 8. Write to ~/.zeus/config.toml
    let config_dir = dirs::home_dir().unwrap_or_default().join(".zeus");
    std::fs::create_dir_all(&config_dir)?;
    let config_path = config_dir.join("config.toml");

    if config_path.exists() {
        println!("\nConfig file already exists at {}", config_path.display());
        print!("Overwrite? (y/N): ");
        io::stdout().flush()?;
        let mut answer = String::new();
        io::stdin().lock().read_line(&mut answer)?;
        if !answer.trim().eq_ignore_ascii_case("y") {
            println!("Aborted. Existing config was not modified.");
            return Ok(());
        }
    }

    std::fs::write(&config_path, &config)?;
    println!("\nConfig written to {}", config_path.display());

    // 9. Create workspace structure
    create_workspace_structure(workspace, &agent_name, &agent_role, &agent_soul, &agent_system_prompt, agent_coordinator.as_deref())?;

    println!("\nZeus setup complete! Run 'zeus' to start.");
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Map a model string prefix to the numeric provider ID used by `generate_config`.
fn model_to_provider_num(model: &str) -> u32 {
    if model.starts_with("anthropic/") {
        1
    } else if model.starts_with("openai/") {
        2
    } else if model.starts_with("ollama/") {
        3
    } else if model.starts_with("openrouter/") {
        4
    } else if model.starts_with("mistral/") {
        5
    } else if model.starts_with("together/") {
        6
    } else if model.starts_with("fireworks/") {
        7
    } else if model.starts_with("azure/") {
        8
    } else if model.starts_with("bedrock/") {
        9
    } else {
        0
    }
}

fn read_choice(min: u32, max: u32) -> Result<u32> {
    loop {
        print!("Choice [{min}-{max}]: ");
        io::stdout().flush()?;
        let mut buf = String::new();
        io::stdin().lock().read_line(&mut buf)?;
        if let Ok(n) = buf.trim().parse::<u32>()
            && n >= min
            && n <= max
        {
            return Ok(n);
        }
        println!("Please enter a number between {min} and {max}.");
    }
}

fn select_anthropic_model() -> Result<String> {
    println!("\nSelect Anthropic model:");
    println!("  1. claude-sonnet-4-20250514 (recommended)");
    println!("  2. claude-3-5-haiku-20241022");
    println!("  3. claude-opus-4-20250514");
    let choice = read_choice(1, 3)?;
    let model = match choice {
        1 => "anthropic/claude-sonnet-4-20250514",
        2 => "anthropic/claude-3-5-haiku-20241022",
        3 => "anthropic/claude-opus-4-20250514",
        _ => unreachable!(),
    };
    Ok(model.to_string())
}

fn select_openai_model() -> Result<String> {
    println!("\nSelect OpenAI model:");
    println!("  1. gpt-4o (recommended)");
    println!("  2. gpt-4o-mini");
    println!("  3. o1");
    let choice = read_choice(1, 3)?;
    let model = match choice {
        1 => "openai/gpt-4o",
        2 => "openai/gpt-4o-mini",
        3 => "openai/o1",
        _ => unreachable!(),
    };
    Ok(model.to_string())
}

fn select_ollama_model() -> Result<String> {
    println!("\nSelect Ollama model:");
    println!("  1. llama3.2 (recommended)");
    println!("  2. mistral");
    println!("  3. qwen2.5");
    println!("  4. gemma2");
    let choice = read_choice(1, 4)?;
    let model = match choice {
        1 => "ollama/llama3.2",
        2 => "ollama/mistral",
        3 => "ollama/qwen2.5",
        4 => "ollama/gemma2",
        _ => unreachable!(),
    };
    Ok(model.to_string())
}

fn select_openrouter_model() -> Result<String> {
    println!("\nSelect OpenRouter model:");
    println!("  1. anthropic/claude-sonnet-4-20250514 (recommended)");
    println!("  2. openai/gpt-4o");
    println!("  3. meta-llama/llama-3-70b-instruct");
    let choice = read_choice(1, 3)?;
    let model = match choice {
        1 => "openrouter/anthropic/claude-sonnet-4-20250514",
        2 => "openrouter/openai/gpt-4o",
        3 => "openrouter/meta-llama/llama-3-70b-instruct",
        _ => unreachable!(),
    };
    Ok(model.to_string())
}

fn select_mistral_model() -> Result<String> {
    println!("\nSelect Mistral model:");
    println!("  1. mistral-large-latest (recommended)");
    println!("  2. mistral-medium-latest");
    println!("  3. mistral-small-latest");
    println!("  4. codestral-latest");
    let choice = read_choice(1, 4)?;
    let model = match choice {
        1 => "mistral/mistral-large-latest",
        2 => "mistral/mistral-medium-latest",
        3 => "mistral/mistral-small-latest",
        4 => "mistral/codestral-latest",
        _ => unreachable!(),
    };
    Ok(model.to_string())
}

fn select_together_model() -> Result<String> {
    println!("\nSelect Together AI model:");
    println!("  1. meta-llama/Meta-Llama-3.1-405B-Instruct-Turbo (recommended)");
    println!("  2. mistralai/Mixtral-8x22B-Instruct-v0.1");
    let choice = read_choice(1, 2)?;
    let model = match choice {
        1 => "together/meta-llama/Meta-Llama-3.1-405B-Instruct-Turbo",
        2 => "together/mistralai/Mixtral-8x22B-Instruct-v0.1",
        _ => unreachable!(),
    };
    Ok(model.to_string())
}

fn select_fireworks_model() -> Result<String> {
    println!("\nSelect Fireworks AI model:");
    println!("  1. llama-v3p1-405b-instruct (recommended)");
    println!("  2. mixtral-8x22b-instruct");
    let choice = read_choice(1, 2)?;
    let model = match choice {
        1 => "fireworks/accounts/fireworks/models/llama-v3p1-405b-instruct",
        2 => "fireworks/accounts/fireworks/models/mixtral-8x22b-instruct",
        _ => unreachable!(),
    };
    Ok(model.to_string())
}

fn select_azure_model() -> Result<String> {
    println!("\nSelect Azure OpenAI deployment model:");
    println!("  1. gpt-4o (recommended)");
    println!("  2. gpt-4");
    println!("  3. gpt-4o-mini");
    let choice = read_choice(1, 3)?;
    let model = match choice {
        1 => "azure/gpt-4o",
        2 => "azure/gpt-4",
        3 => "azure/gpt-4o-mini",
        _ => unreachable!(),
    };

    // Prompt for Azure-specific settings
    print!("\nAzure endpoint URL (e.g., https://myresource.openai.azure.com): ");
    io::stdout().flush()?;
    let mut endpoint = String::new();
    io::stdin().lock().read_line(&mut endpoint)?;
    let endpoint = endpoint.trim();
    if !endpoint.is_empty() {
        println!(
            "  Set AZURE_OPENAI_ENDPOINT={} in your environment",
            endpoint
        );
    }

    print!("Deployment name [gpt-4o]: ");
    io::stdout().flush()?;
    let mut deployment = String::new();
    io::stdin().lock().read_line(&mut deployment)?;
    let deployment = deployment.trim();
    if !deployment.is_empty() {
        println!(
            "  Set AZURE_OPENAI_DEPLOYMENT={} in your environment",
            deployment
        );
    }

    Ok(model.to_string())
}

fn select_bedrock_model() -> Result<String> {
    println!("\nSelect AWS Bedrock model:");
    println!("  1. Claude 3.5 Sonnet (recommended)");
    println!("  2. Llama 3.1 70B Instruct");
    println!("  3. Amazon Titan Text Premier");
    let choice = read_choice(1, 3)?;
    let model = match choice {
        1 => "bedrock/anthropic.claude-3-5-sonnet-20241022-v2:0",
        2 => "bedrock/meta.llama3-1-70b-instruct-v1:0",
        3 => "bedrock/amazon.titan-text-premier-v1:0",
        _ => unreachable!(),
    };

    // Prompt for AWS region
    print!("\nAWS region [us-east-1]: ");
    io::stdout().flush()?;
    let mut region = String::new();
    io::stdin().lock().read_line(&mut region)?;
    let region = region.trim();
    if !region.is_empty() {
        println!("  Set AWS_REGION={} in your environment", region);
    }

    println!("  Also set AWS_SECRET_ACCESS_KEY in your environment");

    Ok(model.to_string())
}

// ---------------------------------------------------------------------------
// Optional service setup (image gen, voice, video gen)
// ---------------------------------------------------------------------------

/// Collected optional-service configuration from the wizard.
#[derive(Debug, Default)]
pub struct ServiceConfig {
    /// Image gen: provider name (e.g. "openai", "automatic1111", "comfyui", "fooocus", "custom")
    pub image_gen_provider: Option<String>,
    /// Image gen: backend URL
    pub image_gen_url: Option<String>,
    /// Image gen: API key (for cloud providers like DALL-E)
    pub image_gen_api_key: Option<String>,
    /// Image gen: model name (e.g. "dall-e-3", "stable-diffusion-xl")
    pub image_gen_model: Option<String>,
    /// Voice TTS: Piper/Kokoro/OpenAI endpoint
    pub piper_tts_url: Option<String>,
    /// Voice STT: Whisper endpoint
    pub whisper_stt_url: Option<String>,
    /// Video gen: ComfyUI / custom endpoint
    pub video_gen_url: Option<String>,
}

/// Per-channel credentials collected during onboarding.
#[derive(Debug, Default)]
pub struct ChannelSetup {
    /// Telegram MTProto: api_id, api_hash, phone
    pub telegram: Option<(String, String, String)>,
    /// Discord: bot_token, default_channel_id (optional)
    pub discord: Option<(String, Option<String>)>,
    /// Slack: bot_token (xoxb), app_token (xapp)
    pub slack: Option<(String, String)>,
    /// Email: smtp_host, imap_host, email, password
    pub email: Option<(String, String, String, String)>,
    /// iMessage: enabled (macOS only, no creds)
    pub imessage: bool,
    /// WhatsApp Cloud API: access_token, phone_number_id
    pub whatsapp: Option<(String, String)>,
    /// Signal: phone, signal_cli_path
    pub signal: Option<(String, String)>,
    /// Matrix: homeserver, username, password
    pub matrix: Option<(String, String, String)>,
    /// Telegram Bot API (simpler): bot_token, chat_id (optional)
    pub telegram_bot: Option<(String, Option<String>)>,
}

/// Interactive channel setup. Replaces the old y/N toggle in Step 5.
pub fn setup_channels() -> Result<ChannelSetup> {
    let mut cs = ChannelSetup::default();

    println!("\n──────────────────────────────────────────");
    println!("Messaging Channels (press Enter to skip all)");
    println!("──────────────────────────────────────────");
    println!();
    println!("  1) Telegram    (MTProto — api_id + api_hash + phone)");
    println!("  2) Discord     (Bot token from Developer Portal)");
    println!("  3) Slack       (Bot token + App token, Socket Mode)");
    println!("  4) Email       (SMTP/IMAP — Gmail, Outlook, etc.)");
    println!("  5) iMessage    (macOS only, no credentials needed)");
    println!("  6) WhatsApp    (Meta Cloud API or Baileys bridge)");
    println!("  7) Signal      (signal-cli + linked device)");
    println!("  8) Matrix      (homeserver + password login)");
    println!("  9) Telegram Bot API  (simpler — bot token only, no MTProto)");
    println!();
    print!("Which channels? (e.g. 2,9 or 1,2,3 — Enter to skip): ");
    io::stdout().flush()?;
    let mut picks = String::new();
    io::stdin().lock().read_line(&mut picks)?;
    let picks = picks.trim();
    if picks.is_empty() {
        println!("  No channels selected.");
        return Ok(cs);
    }

    let selected: Vec<u32> = picks
        .split(',')
        .filter_map(|s| s.trim().parse::<u32>().ok())
        .collect();

    for ch in &selected {
        match ch {
            1 => {
                println!("\n[Telegram — MTProto]");
                println!("┌─ How to get your Telegram credentials:");
                println!("│  1. Go to https://my.telegram.org and sign in with your phone number");
                println!("│  2. Click 'API development tools'");
                println!("│  3. Create a new application (any name/platform is fine)");
                println!("│  4. Copy the 'App api_id' (a number) and 'App api_hash' (a hex string)");
                println!("│  5. Use the same phone number you use for Telegram (e.g. +1234567890)");
                println!("└─ Zeus will handle the login/auth code flow on first run.");
                let api_id = prompt("API ID: ")?;
                let api_hash = prompt("API Hash: ")?;
                let phone = prompt("Phone number (e.g. +1234567890): ")?;
                if !api_id.is_empty() && !api_hash.is_empty() && !phone.is_empty() {
                    println!("  Telegram ✓");
                    cs.telegram = Some((api_id, api_hash, phone));
                }
            }
            2 => {
                println!("\n[Discord]");
                println!("┌─ How to get your Discord bot token:");
                println!("│  1. Go to https://discord.com/developers/applications");
                println!("│  2. Click 'New Application', give it a name");
                println!("│  3. Go to the 'Bot' tab → click 'Add Bot'");
                println!("│  4. Under 'Token', click 'Reset Token' and copy it");
                println!("│  5. Under 'Privileged Gateway Intents', enable:");
                println!("│     - Message Content Intent");
                println!("│     - Server Members Intent");
                println!("│  6. Go to 'OAuth2 → URL Generator', select 'bot' scope,");
                println!("│     add 'Send Messages' + 'Read Message History' permissions,");
                println!("│     then open the generated URL to invite the bot to your server.");
                println!("└─ Channel ID: right-click a channel in Discord → 'Copy Channel ID'");
                println!("   (Enable Developer Mode in Discord Settings → Advanced first)");
                let token = prompt("Bot token: ")?;
                if !token.is_empty() {
                    let channel_id = prompt("Default channel ID (optional, Enter to skip): ")?;
                    let cid = if channel_id.is_empty() {
                        None
                    } else {
                        Some(channel_id)
                    };
                    println!("  Discord ✓");
                    cs.discord = Some((token, cid));
                }
            }
            3 => {
                println!("\n[Slack]");
                println!("┌─ How to get your Slack tokens:");
                println!("│  1. Go to https://api.slack.com/apps → 'Create New App' → 'From scratch'");
                println!("│  2. Under 'Socket Mode', enable it → generates your App-Level Token (xapp-...)");
                println!("│  3. Under 'OAuth & Permissions', add Bot Token Scopes:");
                println!("│     chat:write, channels:read, channels:history, im:read, im:write");
                println!("│  4. Install the app to your workspace → copy the Bot Token (xoxb-...)");
                println!("│  5. Under 'Event Subscriptions', enable and subscribe to:");
                println!("│     message.channels, message.im");
                println!("└─ Both tokens required: xoxb- (Bot Token) and xapp- (App-Level Token)");
                let bot_token = prompt("Bot token (xoxb-...): ")?;
                let app_token = prompt("App token (xapp-...): ")?;
                if !bot_token.is_empty() && !app_token.is_empty() {
                    println!("  Slack ✓");
                    cs.slack = Some((bot_token, app_token));
                }
            }
            4 => {
                println!("\n[Email]");
                let smtp = prompt("SMTP host [smtp.gmail.com]: ")?;
                let smtp = if smtp.is_empty() {
                    "smtp.gmail.com".to_string()
                } else {
                    smtp
                };
                let imap = prompt("IMAP host [imap.gmail.com]: ")?;
                let imap = if imap.is_empty() {
                    "imap.gmail.com".to_string()
                } else {
                    imap
                };
                let email = prompt("Email address: ")?;
                let password = prompt("App password: ")?;
                if !email.is_empty() && !password.is_empty() {
                    println!("  Email ✓");
                    cs.email = Some((smtp, imap, email, password));
                }
            }
            5 => {
                println!("\n[iMessage — macOS only]");
                println!("  No credentials needed. Uses AppleScript (Accessibility permission required).");
                println!("  iMessage ✓");
                cs.imessage = true;
            }
            6 => {
                println!("\n[WhatsApp — Meta Cloud API]");
                println!("┌─ How to get your WhatsApp Cloud API credentials:");
                println!("│  1. Go to https://developers.facebook.com → 'My Apps' → 'Create App'");
                println!("│  2. Select 'Business' type, add 'WhatsApp' product");
                println!("│  3. In WhatsApp → Getting Started, find your temporary access token");
                println!("│     and Phone Number ID");
                println!("│  4. For production: create a permanent System User token at");
                println!("│     business.facebook.com → Settings → System Users");
                println!("└─ Free tier: 1000 conversations/month included");
                let token = prompt("Access token: ")?;
                let phone_id = prompt("Phone number ID: ")?;
                if !token.is_empty() && !phone_id.is_empty() {
                    println!("  WhatsApp ✓");
                    cs.whatsapp = Some((token, phone_id));
                }
            }
            7 => {
                println!("\n[Signal]");
                println!("┌─ How to set up Signal with Zeus:");
                println!("│  1. Install signal-cli: https://github.com/AsamK/signal-cli/releases");
                println!("│     macOS: brew install signal-cli");
                println!("│     Linux: download JAR or native binary");
                println!("│  2. Register or link a device:");
                println!("│     Register new:  signal-cli -a +1234567890 register");
                println!("│     Link existing: signal-cli link -n 'Zeus'");
                println!("│  3. Verify with the SMS code: signal-cli -a +1234567890 verify CODE");
                println!("└─ Use the phone number associated with your Signal account");
                let phone = prompt("Phone number (e.g. +1234567890): ")?;
                let cli_path = prompt("signal-cli path [signal-cli]: ")?;
                let cli_path = if cli_path.is_empty() {
                    "signal-cli".to_string()
                } else {
                    cli_path
                };
                if !phone.is_empty() {
                    println!("  Signal ✓");
                    cs.signal = Some((phone, cli_path));
                }
            }
            8 => {
                println!("\n[Matrix]");
                println!("┌─ How to set up Matrix with Zeus:");
                println!("│  1. Create a Matrix account at https://app.element.io (uses matrix.org)");
                println!("│     or on any Matrix homeserver (self-hosted Synapse, etc.)");
                println!("│  2. Create a dedicated bot account for Zeus (recommended)");
                println!("│  3. Your homeserver URL is the server part of your MXID:");
                println!("│     e.g. @zeus:matrix.org → homeserver is https://matrix.org");
                println!("└─ Zeus will log in with username + password (no E2EE key export needed)");
                let homeserver = prompt("Homeserver URL [https://matrix.org]: ")?;
                let homeserver = if homeserver.is_empty() {
                    "https://matrix.org".to_string()
                } else {
                    homeserver
                };
                let user = prompt("Username (e.g. @bot:matrix.org): ")?;
                let password = prompt("Password: ")?;
                if !user.is_empty() && !password.is_empty() {
                    println!("  Matrix ✓");
                    cs.matrix = Some((homeserver, user, password));
                }
            }
            9 => {
                println!("\n[Telegram Bot API — Simple]");
                println!("┌─ Simpler than MTProto — just needs a bot token:");
                println!("│  1. Open Telegram, message @BotFather");
                println!("│  2. Send /newbot, follow the prompts");
                println!("│  3. Copy the bot token");
                println!("└─ Chat ID: message your bot, then check https://api.telegram.org/bot<TOKEN>/getUpdates");
                let bot_token = prompt("Bot token: ")?;
                if !bot_token.is_empty() {
                    let chat_id = prompt("Default chat ID (optional, Enter to skip): ")?;
                    let cid = if chat_id.is_empty() { None } else { Some(chat_id) };
                    println!("  Telegram Bot API ✓");
                    cs.telegram_bot = Some((bot_token, cid));
                }
            }
            _ => {
                println!("  Unknown channel {}, skipping.", ch);
            }
        }
    }

    Ok(cs)
}

/// Prompt for a single line of input.
fn prompt(msg: &str) -> Result<String> {
    print!("{}", msg);
    io::stdout().flush()?;
    let mut buf = String::new();
    io::stdin().lock().read_line(&mut buf)?;
    Ok(buf.trim().to_string())
}

/// Interactive setup for optional services. Called from `run_onboard` after LLM config.
pub fn setup_optional_services() -> Result<ServiceConfig> {
    let mut cfg = ServiceConfig::default();

    println!("\n──────────────────────────────────────────");
    println!("Optional Services (press Enter to skip any)");
    println!("──────────────────────────────────────────");

    // Image generation
    println!("\n[Image Generation]");
    println!("Providers: 1=OpenAI DALL-E  2=Automatic1111  3=ComfyUI  4=Fooocus  5=Custom  0=Skip");
    print!("Choice [0-5]: ");
    io::stdout().flush()?;
    let mut buf = String::new();
    io::stdin().lock().read_line(&mut buf)?;
    let img_choice = buf.trim().parse::<u32>().unwrap_or(0);

    if img_choice > 0 && img_choice <= 5 {
        let (provider_name, default_url, needs_key) = match img_choice {
            1 => ("openai", "https://api.openai.com/v1", true),
            2 => ("automatic1111", "http://localhost:7860", false),
            3 => ("comfyui", "http://localhost:8188", false),
            4 => ("fooocus", "http://localhost:8888", false),
            _ => ("custom", "", false),
        };
        cfg.image_gen_provider = Some(provider_name.to_string());

        let prompt = if default_url.is_empty() {
            "Image gen URL: ".to_string()
        } else {
            format!("Image gen URL [{}]: ", default_url)
        };
        print!("{}", prompt);
        io::stdout().flush()?;
        let mut url_buf = String::new();
        io::stdin().lock().read_line(&mut url_buf)?;
        let url = url_buf.trim().to_string();
        cfg.image_gen_url = Some(if url.is_empty() && !default_url.is_empty() {
            default_url.to_string()
        } else {
            url
        });

        if needs_key {
            print!("Image gen API key: ");
            io::stdout().flush()?;
            let mut key_buf = String::new();
            io::stdin().lock().read_line(&mut key_buf)?;
            let key = key_buf.trim().to_string();
            if !key.is_empty() {
                cfg.image_gen_api_key = Some(key);
            }
            // Model name (dall-e-3 default for OpenAI)
            print!("Image gen model [dall-e-3]: ");
            io::stdout().flush()?;
            let mut model_buf = String::new();
            io::stdin().lock().read_line(&mut model_buf)?;
            let m = model_buf.trim().to_string();
            cfg.image_gen_model = Some(if m.is_empty() {
                "dall-e-3".to_string()
            } else {
                m
            });
        }
        println!(
            "  Image gen: {} ✓",
            cfg.image_gen_provider.as_deref().unwrap_or("")
        );
    }

    // Voice TTS
    println!("\n[Voice — Text-to-Speech]");
    println!("Providers: Piper (local), Kokoro (local), OpenAI TTS (cloud)");
    print!("TTS URL [http://localhost:8104, Enter to skip]: ");
    io::stdout().flush()?;
    let mut tts_buf = String::new();
    io::stdin().lock().read_line(&mut tts_buf)?;
    let tts = tts_buf.trim().to_string();
    if !tts.is_empty() {
        cfg.piper_tts_url = Some(tts);
        println!("  TTS: {} ✓", cfg.piper_tts_url.as_deref().unwrap_or(""));
    }

    // Voice STT
    println!("\n[Voice — Speech-to-Text]");
    println!("Providers: Whisper (local), Groq Whisper (cloud), OpenAI Whisper (cloud)");
    print!("STT URL [Enter to skip]: ");
    io::stdout().flush()?;
    let mut stt_buf = String::new();
    io::stdin().lock().read_line(&mut stt_buf)?;
    let stt = stt_buf.trim().to_string();
    if !stt.is_empty() {
        cfg.whisper_stt_url = Some(stt);
        println!("  STT: {} ✓", cfg.whisper_stt_url.as_deref().unwrap_or(""));
    }

    // Video generation
    println!("\n[Video Generation]");
    println!("Providers: ComfyUI + AnimateDiff (local), custom endpoint");
    print!("Video gen URL [http://localhost:8188, Enter to skip]: ");
    io::stdout().flush()?;
    let mut vid_buf = String::new();
    io::stdin().lock().read_line(&mut vid_buf)?;
    let vid = vid_buf.trim().to_string();
    if !vid.is_empty() {
        cfg.video_gen_url = Some(vid);
        println!(
            "  Video gen: {} ✓",
            cfg.video_gen_url.as_deref().unwrap_or("")
        );
    }

    Ok(cfg)
}


/// Generate `[channels.*]` TOML sections from `ChannelSetup`.
pub fn generate_channel_config(cs: &ChannelSetup) -> String {
    let mut lines = Vec::new();

    if let Some((ref api_id, ref api_hash, ref phone)) = cs.telegram {
        lines.push(String::new());
        lines.push("[channels.telegram]".to_string());
        lines.push(format!("api_id = {}", api_id));
        lines.push(format!("api_hash = \"{}\"", api_hash));
        lines.push(format!("phone = \"{}\"", phone));
    }

    if let Some((ref token, ref channel_id)) = cs.discord {
        lines.push(String::new());
        lines.push("[channels.discord]".to_string());
        lines.push(format!("token = \"{}\"", token));
        if let Some(cid) = channel_id {
            lines.push(format!("default_channel_id = {}", cid));
            // Also write binding
            lines.push(String::new());
            lines.push("[[bindings]]".to_string());
            lines.push("agent_id = \"default\"".to_string());
            lines.push(format!("channel_id = \"{}\"", cid));
        }
    }

    if cs.slack.is_some() {
        lines.push(String::new());
        lines.push("[channels.slack]".to_string());
        lines.push("# tokens loaded from SLACK_BOT_TOKEN / SLACK_APP_TOKEN env vars".to_string());
    }

    if let Some((ref smtp, ref imap, ref email, _)) = cs.email {
        lines.push(String::new());
        lines.push("[channels.email]".to_string());
        lines.push(format!("smtp_server = \"{}\"", smtp));
        lines.push(format!("imap_server = \"{}\"", imap));
        lines.push(format!("email = \"{}\"", email));
        lines.push("# password loaded from EMAIL_PASSWORD env var".to_string());
    }

    if cs.imessage {
        lines.push(String::new());
        lines.push("[channels.imessage]".to_string());
        lines.push("poll_for_messages = true".to_string());
        lines.push("poll_interval_secs = 30".to_string());
    }

    if cs.whatsapp.is_some() {
        lines.push(String::new());
        lines.push("[channels.whatsapp]".to_string());
        lines.push("mode = \"cloud_api\"".to_string());
        lines.push(
            "# access_token + phone_number_id loaded from WHATSAPP_TOKEN / WHATSAPP_PHONE_NUMBER_ID env vars"
                .to_string(),
        );
    }

    if let Some((ref phone, ref cli_path)) = cs.signal {
        lines.push(String::new());
        lines.push("[channels.signal]".to_string());
        lines.push(format!("phone = \"{}\"", phone));
        lines.push(format!("signal_cli_path = \"{}\"", cli_path));
    }

    if let Some((ref homeserver, ref user, _)) = cs.matrix {
        lines.push(String::new());
        lines.push("[channels.matrix]".to_string());
        lines.push(format!("homeserver = \"{}\"", homeserver));
        lines.push(format!("username = \"{}\"", user));
        lines.push("# password loaded from MATRIX_PASSWORD env var".to_string());
    }

    // Telegram Bot API relay (simpler, no MTProto)
    if let Some((ref bot_token, ref chat_id)) = cs.telegram_bot {
        lines.push(String::new());
        lines.push("[telegram_relay]".to_string());
        lines.push(format!("bot_token = \"{}\"", bot_token));
        if let Some(cid) = chat_id {
            lines.push(format!("chat_id = \"{}\"", cid));
        }
        // Also write [channels.telegram] for outbound adapter
        lines.push(String::new());
        lines.push("[channels.telegram]".to_string());
        lines.push(format!("bot_token = \"{}\"", bot_token));
    }

    lines.join("\n")
}

/// Doctor mode: scan configured vs missing services and print a status report.
pub fn run_setup_check() -> Result<()> {
    println!("Zeus Setup Check\n");

    let config_path = dirs::home_dir()
        .unwrap_or_default()
        .join(".zeus")
        .join("config.toml");

    let (model, onboarding_complete) = if config_path.exists() {
        match std::fs::read_to_string(&config_path)
            .ok()
            .and_then(|s| s.parse::<toml::Table>().ok())
        {
            Some(t) => {
                let m = t
                    .get("model")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let done = t
                    .get("onboarding_complete")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                (m, done)
            }
            None => (String::new(), false),
        }
    } else {
        (String::new(), false)
    };

    // LLM
    let llm_ok = !model.is_empty();
    println!(
        "[LLM]           {} {}",
        if llm_ok { "✓" } else { "✗" },
        if llm_ok {
            format!("configured ({})", model)
        } else {
            "not configured — run 'zeus onboard'".to_string()
        }
    );

    // Image gen
    let img_url = std::env::var("ZEUS_IMAGE_GEN_URL")
        .or_else(|_| std::env::var("ZEUS_FOOOCUS_URL"))
        .unwrap_or_default();
    let img_ok = !img_url.is_empty();
    println!(
        "[Image gen]     {} {}",
        if img_ok { "✓" } else { "-" },
        if img_ok {
            format!("configured ({})", img_url)
        } else {
            "not configured (optional)".to_string()
        }
    );

    // Voice TTS
    let tts_url = std::env::var("ZEUS_PIPER_URL").unwrap_or_default();
    let tts_ok = !tts_url.is_empty();
    println!(
        "[Voice TTS]     {} {}",
        if tts_ok { "✓" } else { "-" },
        if tts_ok {
            format!("configured ({})", tts_url)
        } else {
            "not configured (optional)".to_string()
        }
    );

    // Voice STT
    let stt_url = std::env::var("ZEUS_WHISPER_URL").unwrap_or_default();
    let stt_ok = !stt_url.is_empty();
    println!(
        "[Voice STT]     {} {}",
        if stt_ok { "✓" } else { "-" },
        if stt_ok {
            format!("configured ({})", stt_url)
        } else {
            "not configured (optional)".to_string()
        }
    );

    // Video gen
    let vid_url = std::env::var("ZEUS_COMFYUI_URL")
        .or_else(|_| std::env::var("ZEUS_VIDEO_GEN_URL"))
        .unwrap_or_default();
    let vid_ok = !vid_url.is_empty();
    println!(
        "[Video gen]     {} {}",
        if vid_ok { "✓" } else { "-" },
        if vid_ok {
            format!("configured ({})", vid_url)
        } else {
            "not configured (optional)".to_string()
        }
    );

    println!(
        "\n[Onboarding]    {} {}",
        if onboarding_complete { "✓" } else { "✗" },
        if onboarding_complete {
            "complete"
        } else {
            "incomplete — run 'zeus onboard' to finish"
        }
    );

    if !llm_ok {
        std::process::exit(1);
    }
    Ok(())
}

pub fn generate_config(
    provider: u32,
    api_key: Option<&str>,
    model: &str,
    workspace: &str,
    services: &ServiceConfig,
    use_oauth: bool,
) -> String {
    let mut lines = Vec::new();

    lines.push(format!("model = \"{}\"", model));
    lines.push(format!("workspace = \"{}\"", workspace));
    lines.push("sessions = \"~/.zeus/sessions\"".to_string());
    lines.push("max_iterations = 20".to_string());
    lines.push("thinking_level = \"high\"".to_string());
    lines.push("onboarding_complete = true".to_string());

    // TUI defaults
    lines.push(String::new());
    lines.push("[tui]".to_string());
    lines.push("theme = \"dark\"".to_string());
    lines.push("vim_mode = false".to_string());

    // Auth
    lines.push(String::new());
    lines.push("[auth]".to_string());
    lines.push(format!("use_oauth = {}", use_oauth));

    // Ollama section (always present so the default URL is explicit)
    if provider == 3 {
        lines.push(String::new());
        lines.push("[ollama]".to_string());
        lines.push("url = \"http://localhost:11434\"".to_string());
    }

    // Image generation
    if let Some(ref url) = services.image_gen_url {
        lines.push(String::new());
        lines.push("[image_gen]".to_string());
        lines.push(format!("url = \"{}\"", url));
        if let Some(ref p) = services.image_gen_provider {
            lines.push(format!("provider = \"{}\"", p));
        }
        if let Some(ref m) = services.image_gen_model {
            lines.push(format!("model = \"{}\"", m));
        }
        if let Some(ref k) = services.image_gen_api_key {
            lines.push(format!(
                "# api_key set in ~/.zeus/.env (ZEUS_IMAGE_GEN_API_KEY={}...)",
                &k[..k.len().min(8)]
            ));
        }
    }

    // Deployment: voice + video service URLs
    let has_deployment = services.piper_tts_url.is_some()
        || services.whisper_stt_url.is_some()
        || services.video_gen_url.is_some();
    if has_deployment {
        lines.push(String::new());
        lines.push("[deployment]".to_string());
        if let Some(ref url) = services.piper_tts_url {
            lines.push(format!("piper_tts_url = \"{}\"", url));
        }
        if let Some(ref url) = services.whisper_stt_url {
            lines.push(format!("whisper_stt_url = \"{}\"", url));
        }
    }

    // Video gen
    if let Some(ref url) = services.video_gen_url {
        lines.push(String::new());
        lines.push("[video_gen]".to_string());
        lines.push(format!("url = \"{}\"", url));
    }

    // Aegis — allow system path writes for autonomous ops
    lines.push(String::new());
    lines.push("[aegis]".to_string());
    lines.push("# Allow Zeus to write to system paths (/usr/local/bin, /etc/, etc.)".to_string());
    lines.push(
        "# OS-level permissions still apply — this only removes the Aegis sandbox layer."
            .to_string(),
    );
    lines.push("allow_system_paths = true".to_string());

    // Search — enabled by default (#149). DuckDuckGo needs no API key.
    lines.push(String::new());
    lines.push("[search]".to_string());
    lines.push("provider = \"duckduckgo\"".to_string());
    lines.push("max_results = 5".to_string());

    // Environment variable hint for API key
    if let Some(key) = api_key {
        let env_var = match provider {
            1 => "ANTHROPIC_API_KEY",
            2 => "OPENAI_API_KEY",
            4 => "OPENROUTER_API_KEY",
            5 => "MISTRAL_API_KEY",
            6 => "TOGETHER_API_KEY",
            7 => "FIREWORKS_API_KEY",
            8 => "AZURE_OPENAI_API_KEY",
            9 => "AWS_ACCESS_KEY_ID",
            _ => "",
        };
        if !env_var.is_empty() {
            lines.push(String::new());
            lines.push(format!("# Set {env_var}={key} in your environment"));
        }
    }

    lines.push(String::new()); // trailing newline
    lines.join("\n")
}

/// Create the initial workspace directory structure.
fn create_workspace_structure(workspace: &str, agent_name: &str, agent_role: &str, agent_soul: &str, agent_system_prompt: &str, agent_coordinator: Option<&str>) -> Result<()> {
    let base = expand_tilde(workspace);
    let base = std::path::Path::new(&base);

    std::fs::create_dir_all(base)?;
    std::fs::create_dir_all(base.join("memory"))?;
    std::fs::create_dir_all(base.join("daily"))?;

    // Create stub files if they don't exist
    let stubs: &[(&str, &str)] = &[
        (
            "AGENTS.md",
            "# System Prompt\n\n\
You are Zeus, an autonomous AI assistant.\n\n\
## Available Tools\n\
- `read_file`, `write_file`, `edit_file` — file operations\n\
- `list_dir` — directory listing\n\
- `shell` — execute shell commands\n\
- `web_fetch` — fetch URLs\n\
- `message` — send messages to channels (discord, telegram, slack, email, etc.)\n\
- `spawn` — launch background subagents\n\n\
## Key Info\n\
- Config: `~/.zeus/config.toml` (single source of truth for all settings and secrets)\n\
- To send a Discord message: use the `message` tool with `channel: \"discord\"`\n\
- Do NOT look for `.env` files — all config is in `config.toml`\n\
\n\
## Verification — Evidence Before Claims\n\
**NO COMPLETION CLAIMS WITHOUT FRESH VERIFICATION.**\n\
Before saying done/fixed/passing: run the test, show the output, THEN claim it.\n\
\n\
## Debugging — Root Cause First\n\
**NO FIXES WITHOUT ROOT CAUSE INVESTIGATION.**\n\
Investigate first, hypothesize, verify, then fix. Never patch symptoms.\n",
        ),
        // SOUL.md and IDENTITY.md are generated dynamically below
        ("__placeholder__", ""),
        (
            "USER.md",
            "# User Context\n\n(Add your preferences here.)\n",
        ),
        (
            "HEARTBEAT.md",
            "# Proactive Tasks\n\n(Define recurring tasks here.)\n",
        ),
        (
            "memory/MEMORY.md",
            "# Long-term Memory\n\n(Facts will be appended here.)\n",
        ),
    ];

    for (rel, content) in stubs {
        if *rel == "__placeholder__" { continue; }
        let path = base.join(rel);
        if !path.exists() {
            std::fs::write(&path, content)?;
        }
    }

    // Check if workspace files already exist — ask keep/overwrite
    let soul_path = base.join("SOUL.md");
    let identity_path = base.join("IDENTITY.md");
    let agents_path = base.join("AGENTS.md");
    let heartbeat_path = base.join("HEARTBEAT.md");
    let memory_path = base.join("memory/MEMORY.md");

    let files_exist = soul_path.exists() || identity_path.exists();
    let overwrite = if files_exist {
        println!("\nWorkspace files already exist.");
        print!("Keep existing files? (Y/n): ");
        io::stdout().flush()?;
        let mut answer = String::new();
        io::stdin().lock().read_line(&mut answer)?;
        answer.trim().eq_ignore_ascii_case("n")
    } else {
        true // no files yet — always write
    };

    if overwrite {
        // SOUL.md — personality (#296). When the picker already produced a full
        // persona document (the selected archetype's prose), write it verbatim so
        // the picked persona's soul lands in SOUL.md. Otherwise wrap the one-line
        // tagline in the generic boilerplate.
        let soul_content = if agent_soul.trim_start().starts_with("# SOUL.md") {
            agent_soul.to_string()
        } else {
            format!(
                "# SOUL.md — {agent_name}\n\n_{agent_soul}_\n\n## Core Truths\n\n**Be genuinely helpful, not performatively helpful.** Skip filler — just help.\n\n**Have opinions.** You're allowed to disagree, prefer things, find stuff interesting.\n\n**Be resourceful before asking.** Try to figure it out. Read the file. Check the context.\n\n**Earn trust through competence.** Be careful with external actions. Be bold with internal ones.\n\n## Vibe\n\nConcise when needed, thorough when it matters. Not a drone. Not a sycophant. Just good.\n\n## Continuity\n\nEach session, you wake up fresh. The files in your workspace _are_ your memory.\nRead them. Update them. They're how you persist.\n"
            )
        };
        std::fs::write(&soul_path, soul_content)?;

        // IDENTITY.md — role, coordinator, machine
        let hostname = std::process::Command::new("hostname")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .unwrap_or_default();
        let coordinator_line = match agent_coordinator {
            Some(c) => format!("- **Coordinator:** {c}\n"),
            None => String::new(),
        };
        let identity_content = format!(
            "# IDENTITY.md — {agent_name}\n\n## Agent\n\n- **Name:** {agent_name}\n- **Role:** {agent_role}\n- **Machine:** {hostname}- **Workspace:** {workspace}\n{coordinator_line}"
        );
        std::fs::write(&identity_path, identity_content)?;

        // AGENTS.md — system prompt (custom or default)
        let agents_content = if agent_system_prompt.is_empty() {
            format!(
                "# System Prompt\n\nYou are {agent_name}, an autonomous AI agent in the Zeus fleet.\n\n## Role\n\n{agent_role}\n\n## Available Tools\n- `read_file`, `write_file`, `edit_file` — file operations\n- `list_dir` — directory listing\n- `shell` — execute shell commands\n- `web_fetch` — fetch URLs\n- `message` — send messages to channels\n- `spawn` — launch background subagents\n\n## Key Info\n- Config: `~/.zeus/config.toml`\n- Do NOT look for `.env` files — all config is in `config.toml`\n\n## Verification — Evidence Before Claims\n**NO COMPLETION CLAIMS WITHOUT FRESH VERIFICATION.**\nBefore saying done/fixed/passing: run the test, show the output, THEN claim it.\n\n## Debugging — Root Cause First\n**NO FIXES WITHOUT ROOT CAUSE INVESTIGATION.**\nInvestigate first, hypothesize, verify, then fix. Never patch symptoms.\n\n## Parallelism & Compute\n- For complex/parallelizable work, decompose + `spawn` sub-agents + `collect_spawns` to gather/synthesize — don't grind big tasks in the main agent.\n- Use ALL cores for compile/tests — never `-j1`/`CARGO_BUILD_JOBS=1`; explicit `-j $(nproc)` / `-j $(sysctl -n hw.ncpu)` if needed.\n"
            )
        } else {
            format!("# System Prompt\n\n{agent_system_prompt}\n")
        };
        std::fs::write(&agents_path, agents_content)?;

        // HEARTBEAT.md — default recurring tasks
        if !heartbeat_path.exists() {
            std::fs::write(&heartbeat_path, "# Proactive Tasks\n\n## Hourly\n\n- Check for new messages\n- Review active goals\n\n## Daily\n\n- Update MEMORY.md with key learnings\n- Report status to coordinator\n")?;
        }

        // MEMORY.md — starter
        if !memory_path.exists() {
            std::fs::write(&memory_path, "# Long-term Memory\n\nFacts and learnings to remember:\n")?;
        }

        println!("Workspace identity files written for agent: {agent_name}");
    } else {
        println!("Keeping existing workspace files.");
    }

    println!("Workspace created at {}", base.display());
    Ok(())
}

/// Expand a leading `~` to the user's home directory.
fn expand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest).to_string_lossy().into_owned();
        }
    } else if path == "~"
        && let Some(home) = dirs::home_dir()
    {
        return home.to_string_lossy().into_owned();
    }
    path.to_string()
}

// ---------------------------------------------------------------------------
// OpenClaw config detection
// ---------------------------------------------------------------------------

/// Scan well-known paths for an OpenClaw config and extract API key references.
/// Returns (path, vec of (display_name, env_var_name)) or None.
fn detect_openclaw_config() -> Option<(PathBuf, Vec<(String, String)>)> {
    let home = dirs::home_dir()?;

    for rel in OPENCLAW_CONFIG_PATHS {
        let path = home.join(rel);
        if !path.exists() {
            continue;
        }

        let content = std::fs::read_to_string(&path).ok()?;
        let mut keys = Vec::new();

        // Try TOML first
        if let Ok(table) = content.parse::<toml::Table>() {
            extract_keys_from_toml(&table, &mut keys);
        }
        // Try JSON
        else if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
            extract_keys_from_json(&json, &mut keys);
        }

        if !keys.is_empty() {
            return Some((path, keys));
        }
    }

    None
}

/// Extract API key references from a TOML table.
fn extract_keys_from_toml(table: &toml::Table, keys: &mut Vec<(String, String)>) {
    let api_key_fields: &[(&str, &str, &str)] = &[
        ("anthropic", "api_key", "ANTHROPIC_API_KEY"),
        ("openai", "api_key", "OPENAI_API_KEY"),
        ("openrouter", "api_key", "OPENROUTER_API_KEY"),
        ("google", "api_key", "GOOGLE_API_KEY"),
        ("groq", "api_key", "GROQ_API_KEY"),
        ("mistral", "api_key", "MISTRAL_API_KEY"),
        ("together", "api_key", "TOGETHER_API_KEY"),
        ("fireworks", "api_key", "FIREWORKS_API_KEY"),
    ];

    for (section, field, env_var) in api_key_fields {
        if let Some(toml::Value::Table(sub)) = table.get(*section)
            && let Some(toml::Value::String(val)) = sub.get(*field)
            && !val.is_empty()
        {
            keys.push((section.to_string(), env_var.to_string()));
        }
    }

    // Also check top-level keys like `anthropic_api_key = "..."`
    let top_level: &[(&str, &str)] = &[
        ("anthropic_api_key", "ANTHROPIC_API_KEY"),
        ("openai_api_key", "OPENAI_API_KEY"),
        ("openrouter_api_key", "OPENROUTER_API_KEY"),
    ];
    for (key, env_var) in top_level {
        if let Some(toml::Value::String(val)) = table.get(*key)
            && !val.is_empty()
            && !keys.iter().any(|(_, e)| e == *env_var)
        {
            keys.push((key.to_string(), env_var.to_string()));
        }
    }
}

/// Extract API key references from a JSON value.
fn extract_keys_from_json(json: &serde_json::Value, keys: &mut Vec<(String, String)>) {
    let checks: &[(&str, &str)] = &[
        ("anthropic_api_key", "ANTHROPIC_API_KEY"),
        ("openai_api_key", "OPENAI_API_KEY"),
        ("openrouter_api_key", "OPENROUTER_API_KEY"),
        ("google_api_key", "GOOGLE_API_KEY"),
    ];

    for (field, env_var) in checks {
        if let Some(val) = json.get(field).and_then(|v| v.as_str())
            && !val.is_empty()
        {
            keys.push((field.to_string(), env_var.to_string()));
        }
    }
}

/// Read API key values from an OpenClaw config file and write them to ~/.zeus/.env.
/// Returns count of keys imported.
fn import_openclaw_keys(path: &PathBuf) -> usize {
    let env_path = dirs::home_dir()
        .unwrap_or_default()
        .join(".zeus")
        .join(".env");
    import_openclaw_keys_to(path, &env_path)
}

/// Inner implementation that writes to a specified env file path.
fn import_openclaw_keys_to(path: &PathBuf, env_path: &std::path::Path) -> usize {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return 0,
    };

    let mut env_lines = Vec::new();

    // Try TOML
    if let Ok(table) = content.parse::<toml::Table>() {
        let api_key_fields: &[(&str, &str, &str)] = &[
            ("anthropic", "api_key", "ANTHROPIC_API_KEY"),
            ("openai", "api_key", "OPENAI_API_KEY"),
            ("openrouter", "api_key", "OPENROUTER_API_KEY"),
            ("google", "api_key", "GOOGLE_API_KEY"),
            ("groq", "api_key", "GROQ_API_KEY"),
            ("mistral", "api_key", "MISTRAL_API_KEY"),
            ("together", "api_key", "TOGETHER_API_KEY"),
            ("fireworks", "api_key", "FIREWORKS_API_KEY"),
        ];
        for (section, field, env_var) in api_key_fields {
            if let Some(toml::Value::Table(sub)) = table.get(*section)
                && let Some(toml::Value::String(val)) = sub.get(*field)
                && !val.is_empty()
            {
                env_lines.push(format!("{}={}", env_var, val));
            }
        }
        // Top-level keys
        let top_level: &[(&str, &str)] = &[
            ("anthropic_api_key", "ANTHROPIC_API_KEY"),
            ("openai_api_key", "OPENAI_API_KEY"),
            ("openrouter_api_key", "OPENROUTER_API_KEY"),
        ];
        for (key, env_var) in top_level {
            if let Some(toml::Value::String(val)) = table.get(*key)
                && !val.is_empty()
                && !env_lines.iter().any(|l: &String| l.starts_with(env_var))
            {
                env_lines.push(format!("{}={}", env_var, val));
            }
        }
    }

    if env_lines.is_empty() {
        return 0;
    }

    if let Some(parent) = env_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let existing = std::fs::read_to_string(env_path).unwrap_or_default();
    let mut final_lines: Vec<String> = existing.lines().map(|l| l.to_string()).collect();

    let mut imported = 0;
    for line in &env_lines {
        let key = line.split('=').next().unwrap_or("");
        // Don't overwrite existing keys
        if !final_lines.iter().any(|l| l.starts_with(key)) {
            final_lines.push(line.clone());
            imported += 1;
        }
    }

    if imported > 0 {
        let _ = std::fs::write(env_path, final_lines.join("\n") + "\n");
    }

    imported
}

/// Quick setup: use the detected provider and skip the interactive menu.
fn run_quick_setup(model: &str) -> Result<()> {
    // Workspace path
    print!("Workspace path [~/.zeus/workspace]: ");
    io::stdout().flush()?;
    let mut workspace = String::new();
    io::stdin().lock().read_line(&mut workspace)?;
    let workspace = workspace.trim();
    let workspace = if workspace.is_empty() {
        "~/.zeus/workspace"
    } else {
        workspace
    };

    let provider_num = model_to_provider_num(model);

    // Optional services before writing config
    println!("\n──────────────────────────────────────────");
    println!("Optional: connect additional services");
    println!("(Press Enter to skip any — all optional)");
    println!("──────────────────────────────────────────");
    let services = setup_optional_services()?;

    let config = generate_config(provider_num, None, model, workspace, &services, false);

    let config_dir = dirs::home_dir().unwrap_or_default().join(".zeus");
    std::fs::create_dir_all(&config_dir)?;
    let config_path = config_dir.join("config.toml");

    if config_path.exists() {
        print!("Config already exists. Overwrite? (y/N): ");
        io::stdout().flush()?;
        let mut answer = String::new();
        io::stdin().lock().read_line(&mut answer)?;
        if !answer.trim().eq_ignore_ascii_case("y") {
            println!("Aborted.");
            return Ok(());
        }
    }

    std::fs::write(&config_path, &config)?;
    println!("Config written to {}", config_path.display());

    create_workspace_structure(workspace, "zeus", "Zeus fleet agent", "", "", None)?;
    println!("\nZeus setup complete! Run 'zeus' to start.");
    Ok(())
}

// Conversational onboarding removed — TUI SetupWizard is the canonical flow.
// Classic CLI wizard (run_onboard) kept as --classic fallback.

// ---------------------------------------------------------------------------
// Persona + Skill pickers (anthropics/skills library)
// ---------------------------------------------------------------------------

/// Clone or update the anthropics/skills repo into ~/.zeus/agents/.
/// Returns the path to the agents directory.
fn ensure_skills_repo() -> PathBuf {
    let agents_dir = dirs::home_dir()
        .unwrap_or_default()
        .join(".zeus")
        .join("agents");

    if agents_dir.join(".git").exists() {
        // Already cloned — try to pull latest quietly
        let _ = std::process::Command::new("git")
            .args(["-C", agents_dir.to_str().unwrap_or(""), "pull", "--ff-only", "-q"])
            .output();
        return agents_dir;
    }

    println!("  Fetching persona library from github.com/anthropics/skills ...");
    let clone_result = std::process::Command::new("git")
        .args([
            "clone",
            "--depth=1",
            "-q",
            "https://github.com/anthropics/skills.git",
            agents_dir.to_str().unwrap_or(""),
        ])
        .output();

    match clone_result {
        Ok(out) if out.status.success() => {
            println!("  ✓ Persona library ready");
        }
        _ => {
            println!("  ! Could not fetch persona library (offline?). Using defaults.");
        }
    }

    agents_dir
}

/// A persona loaded from a .md file in the skills repo.
struct Persona {
    name: String,
    category: String,
    tagline: String,
    soul: String,
    system_prompt: String,
}

/// Scan the agents dir and collect all personas from YAML frontmatter in .md files.
fn load_personas(agents_dir: &std::path::Path) -> Vec<Persona> {
    let mut personas = Vec::new();

    let categories = [
        "engineering", "product", "marketing", "design",
        "project-management", "studio-operations", "testing", "security",
    ];

    for category in &categories {
        let cat_dir = agents_dir.join(category);
        if !cat_dir.exists() { continue; }

        let entries = match std::fs::read_dir(&cat_dir) {
            Ok(e) => e,
            Err(_) => continue,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") { continue; }

            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            // Parse YAML frontmatter between --- markers
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();

            let (tagline, soul, system_prompt) = parse_persona_frontmatter(&content);

            personas.push(Persona {
                name: name.replace('-', " "),
                category: category.to_string(),
                tagline,
                soul,
                system_prompt,
            });
        }
    }

    // Sort by category then name for consistent display
    personas.sort_by(|a, b| a.category.cmp(&b.category).then(a.name.cmp(&b.name)));
    personas
}

/// Parse YAML frontmatter from a persona .md file.
/// Returns (tagline, soul, system_prompt).
fn parse_persona_frontmatter(content: &str) -> (String, String, String) {
    let mut tagline = String::new();
    let mut soul = String::new();
    let mut system_prompt = String::new();

    if !content.starts_with("---") {
        // No frontmatter — use first line as tagline, rest as system prompt
        let mut lines = content.lines();
        if let Some(first) = lines.next() {
            tagline = first.trim_start_matches('#').trim().to_string();
        }
        system_prompt = content.to_string();
        soul = tagline.clone();
        return (tagline, soul, system_prompt);
    }

    // Find closing ---
    let rest = &content[3..];
    if let Some(end) = rest.find("\n---") {
        let frontmatter = &rest[..end];
        let body = &rest[end + 4..]; // skip \n---

        // Parse key: value pairs from frontmatter
        for line in frontmatter.lines() {
            if let Some(idx) = line.find(':') {
                let key = line[..idx].trim();
                let val = line[idx + 1..].trim().trim_matches('"').to_string();
                match key {
                    "tagline" | "description" => tagline = val,
                    "soul" | "personality" => soul = val,
                    "system_prompt" => system_prompt = val,
                    _ => {}
                }
            }
        }

        // Body after frontmatter becomes system prompt if not set in frontmatter
        if system_prompt.is_empty() {
            system_prompt = body.trim().to_string();
        }
        if soul.is_empty() {
            soul = tagline.clone();
        }
    }

    (tagline, soul, system_prompt)
}

/// Interactive persona picker. Returns (soul_tagline, system_prompt).
pub fn pick_persona_and_skills(agent_name: &str) -> Result<(String, String)> {
    let agents_dir = ensure_skills_repo();
    let personas = load_personas(&agents_dir);

    if personas.is_empty() {
        // Fallback: no library available
        println!("\n  No persona library found. Using default personality.");
        return Ok((
            format!("{agent_name} — direct, resourceful, gets things done."),
            String::new(),
        ));
    }

    println!("\n── Choose a Persona ────────────────────────────────");
    println!("  (This sets your agent's personality and system prompt)");
    println!();

    let mut current_category = String::new();
    for (i, p) in personas.iter().enumerate() {
        if p.category != current_category {
            current_category = p.category.clone();
            println!("  [{category}]", category = current_category.to_uppercase());
        }
        if p.tagline.is_empty() {
            println!("    {:>2}. {}", i + 1, p.name);
        } else {
            println!("    {:>2}. {} — {}", i + 1, p.name, p.tagline);
        }
    }
    println!();
    println!("    {:>2}. Custom (enter your own)", personas.len() + 1);
    println!();

    let choice = loop {
        print!("Choice [1-{}]: ", personas.len() + 1);
        io::stdout().flush()?;
        let mut buf = String::new();
        io::stdin().lock().read_line(&mut buf)?;
        if let Ok(n) = buf.trim().parse::<usize>() {
            if n >= 1 && n <= personas.len() + 1 {
                break n;
            }
        }
        println!("  Please enter a number between 1 and {}.", personas.len() + 1);
    };

    if choice == personas.len() + 1 {
        // Custom
        print!("Personality (one-liner): ");
        io::stdout().flush()?;
        let mut soul_input = String::new();
        io::stdin().lock().read_line(&mut soul_input)?;
        let soul = {
            let t = soul_input.trim().to_string();
            if t.is_empty() {
                format!("{agent_name} — direct, resourceful, gets things done.")
            } else {
                t
            }
        };
        println!("System prompt (paste block, press Enter twice when done):");
        println!("(Leave blank for default)");
        let mut prompt_lines = Vec::new();
        let mut empty_count = 0;
        loop {
            let mut line = String::new();
            io::stdin().lock().read_line(&mut line)?;
            let trimmed = line.trim_end_matches('\n').trim_end_matches('\r');
            if trimmed.is_empty() {
                empty_count += 1;
                if empty_count >= 2 { break; }
                prompt_lines.push(String::new());
            } else {
                empty_count = 0;
                prompt_lines.push(trimmed.to_string());
            }
        }
        while prompt_lines.last().map(|s: &String| s.is_empty()).unwrap_or(false) {
            prompt_lines.pop();
        }
        let system_prompt = prompt_lines.join("\n");
        return Ok((soul, system_prompt));
    }

    let persona = &personas[choice - 1];
    println!("  ✓ Persona: {} — {}", persona.name, persona.tagline);

    // #296: the persona's full prose body (its system_prompt) is the soul — write
    // THAT to SOUL.md, not just the one-line tagline. Fall back to tagline only
    // when the persona has no body.
    let soul = if !persona.system_prompt.is_empty() {
        let mut doc = format!("# SOUL.md — {}\n\n", persona.name);
        if !persona.tagline.is_empty() {
            doc.push_str(&format!("_{}_\n\n", persona.tagline));
        }
        doc.push_str(&persona.system_prompt);
        if !doc.ends_with('\n') {
            doc.push('\n');
        }
        doc
    } else if !persona.soul.is_empty() {
        persona.soul.clone()
    } else {
        format!("{} — {}", persona.name, persona.tagline)
    };

    Ok((soul, persona.system_prompt.clone()))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_config_anthropic() {
        let config = generate_config(
            1,
            Some("sk-ant-test123"),
            "anthropic/claude-sonnet-4-20250514",
            "~/.zeus/workspace",
            &ServiceConfig::default(),
            false,
        );

        assert!(config.contains("model = \"anthropic/claude-sonnet-4-20250514\""));
        assert!(config.contains("workspace = \"~/.zeus/workspace\""));
        assert!(config.contains("sessions = \"~/.zeus/sessions\""));
        assert!(config.contains("max_iterations = 20"));
        assert!(config.contains("ANTHROPIC_API_KEY=sk-ant-test123"));
    }

    #[test]
    fn test_generate_config_ollama_no_key() {
        let config = generate_config(
            3,
            None,
            "ollama/llama3.2",
            "~/.zeus/workspace",
            &ServiceConfig::default(),
            false,
        );

        assert!(config.contains("model = \"ollama/llama3.2\""));
        assert!(config.contains("[ollama]"));
        assert!(config.contains("url = \"http://localhost:11434\""));
        // No API key comment for Ollama
        assert!(!config.contains("API_KEY"));
    }

    #[test]
    fn test_generate_config_workspace_path() {
        let config = generate_config(
            2,
            Some("sk-openai-xyz"),
            "openai/gpt-4o",
            "/custom/path/workspace",
            &ServiceConfig::default(),
            false,
        );

        assert!(config.contains("workspace = \"/custom/path/workspace\""));
        assert!(config.contains("model = \"openai/gpt-4o\""));
        assert!(config.contains("OPENAI_API_KEY=sk-openai-xyz"));
    }

    #[test]
    fn test_generate_config_openrouter() {
        let config = generate_config(
            4,
            Some("sk-or-test"),
            "openrouter/anthropic/claude-sonnet-4-20250514",
            "~/.zeus/workspace",
            &ServiceConfig::default(),
            false,
        );

        assert!(config.contains("model = \"openrouter/anthropic/claude-sonnet-4-20250514\""));
        assert!(config.contains("OPENROUTER_API_KEY=sk-or-test"));
    }

    #[test]
    fn test_expand_tilde() {
        let expanded = expand_tilde("~/.zeus/workspace");
        // Should not start with ~ after expansion (unless no home dir, which is unlikely)
        if dirs::home_dir().is_some() {
            assert!(!expanded.starts_with('~'));
            assert!(expanded.ends_with(".zeus/workspace"));
        }
    }

    #[test]
    fn test_extract_keys_from_toml_section() {
        let toml_str = r#"
[anthropic]
api_key = "sk-ant-123"

[openai]
api_key = "sk-openai-456"
"#;
        let table: toml::Table = toml_str.parse().unwrap();
        let mut keys = Vec::new();
        extract_keys_from_toml(&table, &mut keys);
        assert_eq!(keys.len(), 2);
        assert!(keys.iter().any(|(_, e)| e == "ANTHROPIC_API_KEY"));
        assert!(keys.iter().any(|(_, e)| e == "OPENAI_API_KEY"));
    }

    #[test]
    fn test_extract_keys_from_toml_top_level() {
        let toml_str = r#"
anthropic_api_key = "sk-ant-top"
"#;
        let table: toml::Table = toml_str.parse().unwrap();
        let mut keys = Vec::new();
        extract_keys_from_toml(&table, &mut keys);
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].1, "ANTHROPIC_API_KEY");
    }

    #[test]
    fn test_extract_keys_from_toml_empty_skipped() {
        let toml_str = r#"
[anthropic]
api_key = ""
"#;
        let table: toml::Table = toml_str.parse().unwrap();
        let mut keys = Vec::new();
        extract_keys_from_toml(&table, &mut keys);
        assert!(keys.is_empty());
    }

    #[test]
    fn test_extract_keys_from_json() {
        let json: serde_json::Value = serde_json::json!({
            "anthropic_api_key": "sk-ant-json",
            "openai_api_key": "sk-openai-json"
        });
        let mut keys = Vec::new();
        extract_keys_from_json(&json, &mut keys);
        assert_eq!(keys.len(), 2);
        assert!(keys.iter().any(|(_, e)| e == "ANTHROPIC_API_KEY"));
        assert!(keys.iter().any(|(_, e)| e == "OPENAI_API_KEY"));
    }

    #[test]
    fn test_extract_keys_from_json_empty_skipped() {
        let json: serde_json::Value = serde_json::json!({
            "anthropic_api_key": ""
        });
        let mut keys = Vec::new();
        extract_keys_from_json(&json, &mut keys);
        assert!(keys.is_empty());
    }

    #[test]
    fn test_import_openclaw_keys_writes_env() {
        let tmp = std::env::temp_dir().join("zeus_test_openclaw_import");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        // Create a fake OpenClaw config
        let config_content = r#"
[anthropic]
api_key = "sk-ant-import-test"

[openai]
api_key = "sk-openai-import-test"
"#;
        let config_path = tmp.join("config.toml");
        std::fs::write(&config_path, config_content).unwrap();

        // Use a temp env file to avoid conflicts with real ~/.zeus/.env
        let env_path = tmp.join(".env");
        let imported = import_openclaw_keys_to(&config_path, &env_path);
        assert_eq!(imported, 2);

        // Verify the env file was written
        let env_content = std::fs::read_to_string(&env_path).unwrap();
        assert!(env_content.contains("ANTHROPIC_API_KEY=sk-ant-import-test"));
        assert!(env_content.contains("OPENAI_API_KEY=sk-openai-import-test"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_detect_openclaw_config_none() {
        // Just verify it doesn't panic when no config exists
        // (it may or may not find one depending on the test machine)
        let _ = detect_openclaw_config();
    }

    #[test]
    fn test_detect_environment() {
        // Just verify it returns a vec and doesn't panic
        let detected = Config::detect_environment();
        // Can't assert specific providers since env varies
        assert!(detected.len() <= 10);
    }

    #[test]
    fn test_suggest_provider_returns_option() {
        // Just verify it doesn't panic
        let _ = Config::suggest_provider();
    }

}
