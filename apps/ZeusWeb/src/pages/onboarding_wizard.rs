// ═══════════════════════════════════════════════════════════
// ZEUS — Onboarding Wizard (8-Step)
// Pixel-perfect translation from zeus-onboarding.jsx
// Phase 2: Wired to API (complete_onboarding)
// ═══════════════════════════════════════════════════════════

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;
use crate::api;
use crate::components::sentient_orb::SentientOrb;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;

/// Render a QR code string as an inline SVG (no renderer feature needed — pure matrix).
fn qr_svg(data: &str) -> String {
    use qrcode::QrCode;
    use qrcode::types::Color;
    let code = match QrCode::new(data.as_bytes()) {
        Ok(c) => c,
        Err(_) => return String::new(),
    };
    let colors = code.to_colors();
    let width = code.width();
    // Each cell is 4px; add 8px quiet-zone padding on each side
    let cell = 4usize;
    let pad = 8usize;
    let size = width * cell + pad * 2;
    let mut svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {s} {s}" width="{s}" height="{s}"><rect width="{s}" height="{s}" fill="#0a0a0f"/>"##,
        s = size
    );
    for (i, color) in colors.iter().enumerate() {
        if *color == Color::Dark {
            let x = (i % width) * cell + pad;
            let y = (i / width) * cell + pad;
            svg.push_str(&format!(
                r##"<rect x="{x}" y="{y}" width="{c}" height="{c}" fill="rgba(255,60,20,1)"/>"##,
                x = x, y = y, c = cell
            ));
        }
    }
    svg.push_str("</svg>");
    svg
}

/// Encode a QR code as a base64 SVG data URL for use in <img src=...>.
fn qr_img_src(data: &str) -> String {
    use base64::Engine;
    let svg = qr_svg(data);
    let b64 = base64::engine::general_purpose::STANDARD.encode(svg.as_bytes());
    format!("data:image/svg+xml;base64,{}", b64)
}

// ─── TYPEWRITER COMPONENT ────────────────────────────────
// Character-by-character text reveal with blinking cursor (from JSX TypeWriter)

#[component]
fn TypeWriter(
    text: &'static str,
    #[prop(default = 40)] speed_ms: u32,
    #[prop(default = 0)] delay_ms: u32,
    #[prop(default = "")] style: &'static str,
) -> impl IntoView {
    let displayed = RwSignal::new(0usize);
    let started = RwSignal::new(false);
    let len = text.len();

    // Delay before starting
    Effect::new(move || {
        let win = web_sys::window().unwrap();
        let cb = Closure::once(move || started.set(true));
        let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(
            cb.as_ref().unchecked_ref(), delay_ms as i32
        );
        cb.forget();
    });

    // Character-by-character interval with cleanup
    let interval_handle = RwSignal::new(0i32);
    Effect::new(move || {
        if !started.get() { return; }
        let win = web_sys::window().unwrap();
        let cb = Closure::wrap(Box::new(move || {
            let cur = displayed.get_untracked();
            if cur < len {
                displayed.set(cur + 1);
            } else {
                let win = web_sys::window().unwrap();
                win.clear_interval_with_handle(interval_handle.get_untracked());
            }
        }) as Box<dyn FnMut()>);
        let id = win.set_interval_with_callback_and_timeout_and_arguments_0(
            cb.as_ref().unchecked_ref(), speed_ms as i32
        ).unwrap_or(0);
        interval_handle.set(id);
        cb.forget();
    });
    // Cleanup on unmount
    on_cleanup(move || {
        if let Some(win) = web_sys::window() {
            win.clear_interval_with_handle(interval_handle.get_untracked());
        }
    });

    view! {
        <span style=style>
            {move || &text[..displayed.get().min(len)]}
            <span style={move || format!("opacity: {}; transition: opacity 0.3s; font-weight: 400;",
                if displayed.get() < len { "1" } else { "0" }
            )}>
                "\u{2588}"
            </span>
        </span>
    }
}

// ─── FADEIN COMPONENT ────────────────────────────────────
// Smooth opacity + translateY transition with configurable delay (from JSX FadeIn)

#[component]
fn FadeIn(
    children: Children,
    #[prop(default = 0)] delay_ms: u32,
    #[prop(default = "")] style: &'static str,
) -> impl IntoView {
    let visible = RwSignal::new(false);

    Effect::new(move || {
        let win = web_sys::window().unwrap();
        let cb = Closure::once(move || visible.set(true));
        let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(
            cb.as_ref().unchecked_ref(), delay_ms as i32
        );
        cb.forget();
    });

    view! {
        <div style={move || format!(
            "opacity: {}; transform: translateY({}px); transition: all 1.2s cubic-bezier(0.16, 1, 0.3, 1); {}",
            if visible.get() { 1 } else { 0 },
            if visible.get() { 0 } else { 16 },
            style
        )}>
            {children()}
        </div>
    }
}

// ─── STEP DATA ───────────────────────────────────────────

#[allow(dead_code)]
struct StepInfo {
    id: &'static str,
    title: &'static str,
    orb_mode: &'static str,
    orb_intensity: f64,
}

const STEPS: &[StepInfo] = &[
    StepInfo { id: "awaken",        title: "AWAKENING",          orb_mode: "dormant",   orb_intensity: 0.3 },
    StepInfo { id: "mode",          title: "AWAKENING PROTOCOL", orb_mode: "waking",    orb_intensity: 0.4 },
    StepInfo { id: "config-preview",title: "CONFIGURATION",      orb_mode: "waking",    orb_intensity: 0.5 },
    StepInfo { id: "identity",      title: "IDENTITY",           orb_mode: "waking",    orb_intensity: 0.5 },
    StepInfo { id: "intelligence", title: "INTELLIGENCE", orb_mode: "listening", orb_intensity: 0.65 },
    StepInfo { id: "model", title: "MODEL SELECT", orb_mode: "thinking", orb_intensity: 0.75 },
    StepInfo { id: "channels", title: "SENSES", orb_mode: "active", orb_intensity: 0.85 },
    StepInfo { id: "security", title: "ARMOR", orb_mode: "active", orb_intensity: 0.9 },
    StepInfo { id: "features", title: "ABILITIES", orb_mode: "speaking", orb_intensity: 0.95 },
    StepInfo { id: "services", title: "SERVICES", orb_mode: "speaking", orb_intensity: 0.95 },
    StepInfo { id: "orchestration", title: "ORCHESTRATION", orb_mode: "thinking", orb_intensity: 0.96 },
    StepInfo { id: "memory", title: "MEMORY", orb_mode: "thinking", orb_intensity: 0.97 },
    StepInfo { id: "skills", title: "SKILLS", orb_mode: "speaking", orb_intensity: 0.98 },
    StepInfo { id: "launch", title: "IGNITION", orb_mode: "surge", orb_intensity: 1.0 },
];

struct Provider {
    id: &'static str,
    name: &'static str,
    desc: &'static str,
    models: &'static [&'static str],
    color: &'static str,
    hot: bool,
    local: bool,
}

// Provider metadata only — NO hardcoded model lists.
// Models are fetched from the provider API after credentials are configured.
const PROVIDERS: &[Provider] = &[
    Provider { id: "anthropic", name: "Anthropic", desc: "Claude AI models", models: &[], color: "#d4a574", hot: true, local: false },
    Provider { id: "openai", name: "OpenAI", desc: "GPT and reasoning models", models: &[], color: "#74d4a5", hot: true, local: false },
    Provider { id: "google", name: "Google", desc: "Gemini API models", models: &[], color: "#4285f4", hot: true, local: false },
    Provider { id: "ollama", name: "Ollama", desc: "Local models — no API key needed", models: &[], color: "#74a5d4", hot: false, local: true },
    Provider { id: "google-gemini-cli", name: "Gemini CLI", desc: "Code assist via Gemini CLI OAuth", models: &[], color: "#0f9d58", hot: false, local: false },
    Provider { id: "moonshot", name: "Kimi", desc: "Moonshot AI — K2.5 series", models: &[], color: "#ff6b35", hot: false, local: false },
    Provider { id: "zai", name: "GLM", desc: "ZAI — GLM series models", models: &[], color: "#e74c3c", hot: false, local: false },
    Provider { id: "qwen", name: "Qwen", desc: "Alibaba — device code OAuth", models: &[], color: "#6c5ce7", hot: false, local: false },
    Provider { id: "minimax", name: "MiniMax", desc: "Portal OAuth — Anthropic Messages API", models: &[], color: "#fdcb6e", hot: false, local: false },
    Provider { id: "xiaomimimo", name: "MiMo", desc: "Xiaomi — MiMo models", models: &[], color: "#ff8800", hot: false, local: false },
];

struct Channel {
    id: &'static str,
    name: &'static str,
    desc: &'static str,
    icon: &'static str,
}

// Launch channels: Discord, Telegram, IRC, Signal
const CHANNELS: &[Channel] = &[
    Channel { id: "discord", name: "Discord", desc: "Guilds, channels, embeds", icon: "D" },
    Channel { id: "telegram", name: "Telegram", desc: "MTProto — groups, media, voice", icon: "T" },
    Channel { id: "irc", name: "IRC", desc: "Traditional IRC with TLS", icon: "IR" },
    Channel { id: "signal", name: "Signal", desc: "signal-cli JSON-RPC", icon: "Sg" },
    Channel { id: "matrix", name: "Matrix", desc: "Federation, E2EE, rooms", icon: "M" },
    Channel { id: "twitter", name: "X / Twitter", desc: "Twitter API v2 — DMs, mentions, posts", icon: "X" },
    Channel { id: "slack", name: "Slack", desc: "Socket Mode WebSocket — bot & app tokens", icon: "Sl" },
    Channel { id: "email", name: "Email", desc: "IMAP polling + SMTP replies", icon: "Em" },
    Channel { id: "whatsapp", name: "WhatsApp", desc: "Baileys bridge or Cloud API", icon: "Wa" },
    Channel { id: "mqtt", name: "MQTT", desc: "Subscribe + publish via rumqttc", icon: "Mq" },
    Channel { id: "mattermost", name: "Mattermost", desc: "Self-hosted Slack alternative, WebSocket API", icon: "Mm" },
    Channel { id: "pantheon", name: "Pantheon", desc: "Multi-agent war rooms + missions", icon: "P" },
];

// ─── DYNAMIC PROVIDER/CHANNEL (owned types for API data) ─
#[derive(Clone, Debug)]
struct DynProvider {
    id: String,
    name: String,
    desc: String,
    models: Vec<String>,
    color: String,
    hot: bool,
    local: bool,
}

#[derive(Clone, Debug)]
struct DynChannel {
    id: String,
    name: String,
    desc: String,
    icon: String,
}

fn fallback_providers() -> Vec<DynProvider> {
    PROVIDERS.iter().map(|p| DynProvider {
        id: p.id.to_string(),
        name: p.name.to_string(),
        desc: p.desc.to_string(),
        models: p.models.iter().map(|m| m.to_string()).collect(),
        color: p.color.to_string(),
        hot: p.hot,
        local: p.local,
    }).collect()
}

fn fallback_channels() -> Vec<DynChannel> {
    CHANNELS.iter().map(|ch| DynChannel {
        id: ch.id.to_string(),
        name: ch.name.to_string(),
        desc: ch.desc.to_string(),
        icon: ch.icon.to_string(),
    }).collect()
}

struct Feature {
    id: &'static str,
    name: &'static str,
    desc: &'static str,
    default: bool,
}

// Feature ids are the TUI canonical set (crates/zeus-tui/src/onboarding/mod.rs:1043-1055)
// so toggles map 1:1 to real config sections — TUI is the source of truth (#216).
// Defaults mirror the TUI's initial toggle map.
const FEATURES: &[Feature] = &[
    Feature { id: "nous", name: "Nous Cognitive Engine", desc: "Intent recognition, reasoning chains, meta-cognition", default: true },
    Feature { id: "mnemosyne", name: "Mnemosyne Memory", desc: "SQLite FTS5, vector embeddings, temporal versioning", default: true },
    Feature { id: "aegis", name: "Aegis Security", desc: "Seatbelt sandbox, command filtering, approvals", default: true },
    Feature { id: "athena", name: "Athena Docs", desc: "Obsidian vault integration — notes, docs, knowledge base", default: false },
    Feature { id: "hermes", name: "Hermes Notifications", desc: "Proactive notifications via console, Telegram, Discord", default: false },
    Feature { id: "prometheus", name: "Prometheus Orchestration", desc: "Task planner, cooking loop, heartbeat, cron", default: true },
    Feature { id: "browser", name: "Browser Automation", desc: "Chrome CDP — navigate, click, screenshot, JS", default: false },
    Feature { id: "talos", name: "macOS Automation", desc: "193 Talos tools — Calendar, Notes, Mail, UI", default: false },
    Feature { id: "mcp", name: "MCP Server", desc: "Model Context Protocol for Claude Code/Desktop", default: false },
];

struct SecurityLevel {
    id: &'static str,
    name: &'static str,
    desc: &'static str,
    risk: &'static str,
    color: &'static str,
    features: &'static [&'static str],
}

const SECURITY_LEVELS: &[SecurityLevel] = &[
    SecurityLevel { id: "minimal", name: "MINIMAL", desc: "No restrictions — full access to file system, shell, and network. Best for development and testing.", risk: "Low security", color: "#eab308", features: &["No sandboxing", "No command filtering", "No URL restrictions", "No approval workflow"] },
    SecurityLevel { id: "standard", name: "STANDARD", desc: "Command filtering, URL allowlisting, and path restrictions. Good balance of security and usability.", risk: "Recommended", color: "#22c55e", features: &["Command allowlist", "URL filtering", "Path restrictions", "Audit logging"] },
    SecurityLevel { id: "strict", name: "STRICT", desc: "Full Seatbelt sandbox, mandatory approvals for all shell and web operations. Maximum security.", risk: "Maximum security", color: "rgba(255,60,20,1)", features: &["macOS Seatbelt", "Mandatory approvals", "Process isolation", "Complete audit trail"] },
];

// ─── CONFIG STATE ────────────────────────────────────────

#[derive(Clone, Debug)]
struct OnboardConfig {
    /// Agent's own name (TUI parity: defaults to machine hostname; in the
    /// browser we default to the gateway host since wasm can't read the OS hostname).
    agent_name: String,
    user_name: String,
    user_role: String,
    user_org: String,
    personality: String,
    gateway_url: String,
    providers: Vec<String>,
    api_keys: std::collections::HashMap<String, String>,
    /// Per-provider auth type: "api_key" (default) or "oauth_token"
    auth_types: std::collections::HashMap<String, String>,
    ollama_url: String,
    default_model: String,
    channels: Vec<String>,
    /// Per-channel credential fields: channel_id -> { field_name -> value }
    channel_creds: std::collections::HashMap<String, std::collections::HashMap<String, String>>,
    security_level: String,
    features: Vec<String>,
    image_gen_provider: String,
    image_gen_url: String,
    image_gen_api_key: String,
    whisper_url: String,
    piper_url: String,
    elevenlabs_api_key: String,
    video_url: String,
    // Communication style
    communication_style: String,       // "professional" | "collaborative" | "minimal" | "autonomous"
    // LLM Council
    council_enabled: bool,
    // Orchestration (Prometheus + Nous)
    orchestration_mode: String,        // "enable_all" | "disable" | "custom"
    heartbeat_interval: String,        // seconds, default "300"
    nous_intent: bool,                 // default true
    nous_learning: bool,               // default true
    // Memory (Mnemosyne)
    memory_fts: bool,                  // FTS5 enabled, default true
    memory_embeddings: bool,           // vector embeddings, default false
    memory_db_path: String,            // mnemosyne db path (TUI parity)
    memory_embedding_provider: String, // none|ollama|openai|gemini|voyage (TUI parity)
    // Agent
    verbosity: String,                 // "normal" | "silent" | "barfly"
    fallback_models: Vec<String>,      // ordered fallback model list
    onboarding_mode: String,
    // QuickStart config fields (mirror TUI)
    qs_port: String,
    qs_bind: String,
    qs_workspace: String,
    qs_sessions: String,
    qs_max_iterations: String,
    // Rate limiting
    rate_limit_enabled: bool,
    rate_limit_rpm: String,
    // Session compaction
    compaction_max_tokens: String,
    compaction_threshold: String,
    // Hermes notifications
    hermes_channel: String,
    // Channel policies
    allow_bots_mode: String,
    // Gateway config
    gateway_timeout: String,
    gateway_mentions_only: bool,
}

impl Default for OnboardConfig {
    fn default() -> Self {
        Self {
            agent_name: web_sys::window()
                .and_then(|w| w.location().hostname().ok())
                .filter(|h| !h.is_empty() && h != "localhost" && h != "127.0.0.1")
                .unwrap_or_else(|| "Zeus".to_string()),
            user_name: String::new(),
            user_role: String::new(),
            user_org: String::new(),
            personality: "collaborative".to_string(),
            gateway_url: String::new(),
            // Single-active provider selection (see StepIntelligence on:click).
            providers: vec!["anthropic".to_string()],
            api_keys: std::collections::HashMap::new(),
            auth_types: std::collections::HashMap::new(),
            ollama_url: "http://localhost:11434".to_string(),
            default_model: String::new(), // No default — user must select from API-fetched list
            channels: vec!["telegram".to_string(), "discord".to_string()],
            channel_creds: std::collections::HashMap::new(),
            security_level: "standard".to_string(),
            features: FEATURES.iter().filter(|f| f.default).map(|f| f.id.to_string()).collect(),
            image_gen_provider: String::new(),
            image_gen_url: String::new(),
            image_gen_api_key: String::new(),
            whisper_url: String::new(),
            piper_url: String::new(),
            elevenlabs_api_key: String::new(),
            video_url: String::new(),
            communication_style: "professional".to_string(),
            council_enabled: false,
            orchestration_mode: "enable_all".to_string(),
            heartbeat_interval: "300".to_string(),
            nous_intent: true,
            nous_learning: true,
            memory_fts: true,
            memory_embeddings: false,
            memory_db_path: "~/.zeus/memory.db".to_string(),
            memory_embedding_provider: "none".to_string(),
            verbosity: "normal".to_string(),
            fallback_models: Vec::new(),
            onboarding_mode: "quickstart".to_string(),
            qs_port: option_env!("ZEUS_GATEWAY_PORT").unwrap_or("8080").to_string(),
            qs_bind: "0.0.0.0".to_string(),
            qs_workspace: "~/.zeus/workspace".to_string(),
            qs_sessions: "~/.zeus/sessions".to_string(),
            qs_max_iterations: "20".to_string(),
            rate_limit_enabled: true,
            rate_limit_rpm: "20".to_string(),
            compaction_max_tokens: "180000".to_string(),
            compaction_threshold: "0.8".to_string(),
            hermes_channel: "console".to_string(),
            allow_bots_mode: "mentions".to_string(),
            gateway_timeout: "1800".to_string(),
            gateway_mentions_only: false,
        }
    }
}

// ─── STATUS ENUMS ────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
enum KeyTestStatus {
    Untested,
    Testing,
    Valid,
    Invalid(String),
    InfoOnly,
}

#[derive(Clone, Debug, PartialEq)]
enum SaveStatus {
    Idle,
    Saving,
    Success,
    Error(String),
}

// ─── MAIN ONBOARDING PAGE ────────────────────────────────

#[component]
pub fn OnboardingWizardPage() -> impl IntoView {
    let step = RwSignal::new(0usize);

    // Background ambient sound — loops for entire onboarding lifetime
    // Starts on first user interaction (click anywhere) to satisfy autoplay policy,
    // then loops continuously. Stops when component is dropped (leave onboarding).
    Effect::new(move |_| {
        let _ = step.get(); // subscribe to keep effect alive
        let win = web_sys::window().unwrap();

        // Only create audio once — check if already exists
        let existing = js_sys::Reflect::get(&win, &"__zeus_ambient".into()).ok();
        if existing.as_ref().map(|v| !v.is_undefined() && !v.is_null()).unwrap_or(false) {
            return; // already playing
        }

        if let Ok(audio) = web_sys::HtmlAudioElement::new_with_src("audio/sfx_ambient_drone.mp3") {
            audio.set_volume(0.15);
            audio.set_loop(true);

            // Try to play immediately
            let _ = audio.play();

            // Also set up click listener as fallback for autoplay policy
            let audio_clone = audio.clone();
            let started = wasm_bindgen::closure::Closure::wrap(Box::new(move || {
                let _ = audio_clone.play();
            }) as Box<dyn FnMut()>);
            let doc = win.document().unwrap();
            let _ = doc.add_event_listener_with_callback("click", started.as_ref().unchecked_ref());
            started.forget();

            // Store globally so it persists and we can detect/stop it
            let _ = js_sys::Reflect::set(&win, &"__zeus_ambient".into(), &audio);
        }
    });

    // Cleanup: stop ambient when leaving onboarding
    on_cleanup(move || {
        if let Some(win) = web_sys::window()
            && let Ok(val) = js_sys::Reflect::get(&win, &"__zeus_ambient".into())
                && !val.is_undefined() && !val.is_null() {
                    if let Ok(audio) = val.dyn_into::<web_sys::HtmlAudioElement>() {
                        audio.pause().ok();
                        audio.set_src("");
                    }
                    let _ = js_sys::Reflect::set(&win, &"__zeus_ambient".into(), &wasm_bindgen::JsValue::NULL);
                }
    });
    let config = RwSignal::new(OnboardConfig::default());
    let is_rerun = RwSignal::new(false);

    // Check for existing config (re-run detection)
    spawn_local({
        let config = config;
        let is_rerun = is_rerun;
        async move {
            // Check localStorage first
            if let Some(win) = web_sys::window()
                && let Ok(Some(storage)) = win.local_storage()
                && storage.get_item("zeus_onboarding_complete").ok().flatten() == Some("true".into())
            {
                is_rerun.set(true);
            }
            // Also check gateway API
            if let Ok(status) = api::fetch_json::<serde_json::Value>("/v1/onboarding/status").await {
                if status.get("completed").and_then(|v| v.as_bool()).unwrap_or(false) {
                    is_rerun.set(true);
                }
                // Pre-fill config from existing if available
                if let Some(name) = status.get("name").and_then(|v| v.as_str())
                    && !name.is_empty()
                {
                    config.update(|c| c.agent_name = name.to_string());
                }
                if let Some(model) = status.get("model").and_then(|v| v.as_str()) {
                    config.update(|c| c.default_model = model.to_string());
                }
                if let Some(provider) = status.get("provider").and_then(|v| v.as_str()) {
                    // Single-active selection: an existing config's provider becomes
                    // the sole selected provider (replaces the default), not an addition.
                    config.update(|c| {
                        c.providers.clear();
                        c.providers.push(provider.to_string());
                    });
                }
            }
        }
    });

    // ─── DYNAMIC DATA SIGNALS (Phase 1: API with fallback) ──
    let providers_data: RwSignal<Vec<DynProvider>> = RwSignal::new(fallback_providers());
    let channels_data: RwSignal<Vec<DynChannel>> = RwSignal::new(fallback_channels());
    let _data_loaded = RwSignal::new(false);
    let key_status: RwSignal<std::collections::HashMap<String, KeyTestStatus>> = RwSignal::new(std::collections::HashMap::new());

    // Fetch providers + channels from API, fall back to hardcoded if gateway is offline
    spawn_local({
        async move {
            // Fetch providers
            if let Ok(resp) = api::fetch_providers_list().await
                && !resp.providers.is_empty() {
                    let dyn_provs: Vec<DynProvider> = resp.providers.iter().map(|p| {
                        DynProvider {
                            id: p.id.clone(),
                            name: p.name.clone(),
                            desc: p.tagline.clone(),
                            models: p.models.iter().map(|m| m.id.clone()).collect(),
                            color: if p.color.is_empty() { "#74a5d4".to_string() } else { p.color.clone() },
                            hot: false, // API doesn't have this field — could be added later
                            local: p.requires_url,
                        }
                    }).collect();
                    providers_data.set(dyn_provs);
                    web_sys::console::log_1(&"Zeus: Loaded providers from API".into());
                }
            // Fetch channels
            if let Ok(resp) = api::fetch_channels().await
                && !resp.channels.is_empty() {
                    let dyn_chs: Vec<DynChannel> = resp.channels.iter().map(|ch| {
                        DynChannel {
                            id: ch.id.clone(),
                            name: if ch.name.is_empty() { ch.platform.clone() } else { ch.name.clone() },
                            desc: ch.channel_type.clone(),
                            icon: ch.name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or("?".to_string()),
                        }
                    }).collect();
                    channels_data.set(dyn_chs);
                    web_sys::console::log_1(&"Zeus: Loaded channels from API".into());
                }
            _data_loaded.set(true);
        }
    });

    // ─── TTS NARRATION (Web Speech API only) ────────────────
    // Speaks step titles aloud using browser speech synthesis.
    // After name is entered (step >= 2), addresses user by name first.
    let prev_step = RwSignal::new(255usize); // sentinel: no previous step
    Effect::new(move |_| {
        let s = step.get();
        if s == prev_step.get_untracked() { return; }
        prev_step.set(s);

        // Cancel any current speech
        let _ = js_sys::eval("if(window.speechSynthesis) speechSynthesis.cancel();");

        // Get user name for personalized speech (untracked to avoid re-triggers)
        let name = config.get_untracked().user_name.trim().to_string();
        let has_name = !name.is_empty();

        // Descriptive titles for each step, with user name from step 2+
        let text = match s {
            0 => "Initializing Zeus".to_string(),
            1 => "Choose your awakening protocol".to_string(),
            2 => "Reviewing initial configuration".to_string(),
            3 => "Identity configuration".to_string(),
            4 => if has_name { format!("{}, choose your intelligence sources", name) } else { "Choose your intelligence sources".to_string() },
            5 => if has_name { format!("{}, select your default model", name) } else { "Select your default model".to_string() },
            6 => if has_name { format!("{}, configure your senses", name) } else { "Configure your senses".to_string() },
            7 => if has_name { format!("{}, choose your armor", name) } else { "Choose your armor".to_string() },
            8 => if has_name { format!("{}, enable your abilities", name) } else { "Enable your abilities".to_string() },
            9 => if has_name { format!("{}, connect your services", name) } else { "Connect your services".to_string() },
            10 => if has_name { format!("{}, configure orchestration", name) } else { "Configure orchestration".to_string() },
            11 => if has_name { format!("{}, configure memory systems", name) } else { "Configure memory systems".to_string() },
            12 => if has_name { format!("{}, select your skills", name) } else { "Select your skills".to_string() },
            13 => if has_name { format!("{}, Zeus is alive", name) } else { "Zeus is alive".to_string() },
            _ => return,
        };

        // Speak via Web Speech API — rate 0.85 for slow deliberate delivery
        let script = format!(
            "var u = new SpeechSynthesisUtterance('{}'); u.rate = 0.8; u.pitch = 0.85; u.volume = 0.8; speechSynthesis.speak(u);",
            text.replace(char::from(39), "")
        );
        let _ = js_sys::eval(&script);
    });

    // Cleanup: cancel speech when leaving onboarding
    on_cleanup(move || {
        let _ = js_sys::eval("if(window.speechSynthesis) speechSynthesis.cancel();");
    });

    let can_next = Memo::new(move |_| {
        let s = step.get();
        let c = config.get();
        match s {
            0 => true,
            1 => true,  // StepOnboardingMode — always can proceed (Skip handled in-component)
            2 => true,  // StepQuickStartConfig — read-only, always can proceed
            3 => !c.user_name.trim().is_empty(),
            4 => {
                // Require at least one selected provider to have credentials:
                // - OAuth providers: auth_type set to "oauth_token" (flow completed)
                // - Local providers (ollama): no key needed
                // - Cloud providers: non-empty API key entered or key_status is Valid/InfoOnly
                let ks = key_status.get();
                c.providers.iter().any(|pid| {
                    if c.auth_types.get(pid).map(|t| t == "oauth_token").unwrap_or(false) {
                        return true;
                    }
                    // #216c: device-code OAuth + Gemini CLI import count only once the
                    // flow completed (poll/import marks key_status Valid)
                    if c.auth_types.get(pid).map(|t| t == "oauth_device").unwrap_or(false) {
                        return matches!(ks.get(pid), Some(KeyTestStatus::Valid));
                    }
                    // #216b: browser OAuth counts only once the flow completed
                    // (poll loop marks key_status Valid on authenticated:true)
                    if c.auth_types.get(pid).map(|t| t == "oauth_browser").unwrap_or(false) {
                        return matches!(ks.get(pid), Some(KeyTestStatus::Valid));
                    }
                    if pid == "ollama" {
                        return true;
                    }
                    if c.api_keys.get(pid).map(|k| !k.trim().is_empty()).unwrap_or(false) {
                        return true;
                    }
                    matches!(ks.get(pid), Some(KeyTestStatus::Valid) | Some(KeyTestStatus::InfoOnly))
                })
            },
            5 => !c.default_model.is_empty(),
            6 => true,
            7 => true,
            8 => !c.features.is_empty(),
            9 => true,
            10 => true,
            11 => true,
            12 => true,
            13 => true,
            _ => true,
        }
    });

    view! {
        <div style="min-height: 100vh; background: #050508; color: rgba(255,245,240,0.9); font-family: 'Rajdhani', sans-serif; display: flex; flex-direction: column;">

            // Top bar (steps 1-6 only)
            {move || {
                let s = step.get();
                if s > 0 && s < 13 {
                    let step_info = &STEPS[s];
                    view! {
                        <div style="padding: 18px 32px; display: flex; align-items: center; justify-content: space-between; border-bottom: 1px solid rgba(255,60,20,0.1); flex-shrink: 0;">
                            <div class="flex items-center gap-3.5">
                                <SentientOrb size=32 mode={step_info.orb_mode.to_string()} intensity={step_info.orb_intensity} />
                                <div>
                                    <div style="font-family: 'Orbitron', monospace; font-size: 11px; letter-spacing: 4px; color: rgba(255,245,240,0.9); font-weight: 900;">
                                        "ZEUS"
                                    </div>
                                    <div style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 3px; color: rgba(255,245,240,0.5); font-weight: 700;">
                                        {step_info.title}
                                    </div>
                                </div>
                            </div>

                            // Step indicator dots
                            <div style="display: flex; align-items: center; gap: 6px;">
                                {(1..=12usize).map(|i| {
                                    let bg = if i < s { "rgba(255,60,20,1)" } else if i == s { "rgba(255,60,20,0.6)" } else { "rgba(255,60,20,0.15)" };
                                    let width = if i == s { "28px" } else { "8px" };
                                    let cursor = if i < s { "pointer" } else { "default" };
                                    let style_str = format!(
                                        "width: {}; height: 8px; border-radius: 4px; transition: all 0.5s cubic-bezier(0.16,1,0.3,1); background: {}; cursor: {};",
                                        width, bg, cursor
                                    );
                                    let can_click = i < s;
                                    view! {
                                        <div
                                            style={style_str}
                                            on:click=move |_| {
                                                if can_click { step.set(i); }
                                            }
                                        />
                                    }
                                }).collect::<Vec<_>>()}
                            </div>

                            // Progress ring
                            <ProgressRing step={s - 1} total=12 />
                        </div>
                    }.into_any()
                } else {
                    view! { <div></div> }.into_any()
                }
            }}

            // Content area
            <div style="flex: 1; display: flex; justify-content: center; overflow-y: auto;">
                <div style=move || {
                    let s = step.get();
                    if s == 0 || s == 13 {
                        "width: 100%; max-width: 800px; padding: 0 32px;"
                    } else {
                        "width: 100%; max-width: 680px; padding: 36px 32px 120px;"
                    }
                }>
                    {move || {
                        match step.get() {
                            0  => view! { <StepAwaken step=step /> }.into_any(),
                            1  => view! { <StepOnboardingMode config=config /> }.into_any(),
                            2  => view! { <StepQuickStartConfig config=config /> }.into_any(),
                            3  => view! { <StepIdentity config=config /> }.into_any(),
                            4  => view! { <StepIntelligence config=config providers_data=providers_data key_status=key_status /> }.into_any(),
                            5  => view! { <StepModel config=config providers_data=providers_data /> }.into_any(),
                            6  => view! { <StepChannels config=config channels_data=channels_data /> }.into_any(),
                            7  => view! { <StepSecurity config=config /> }.into_any(),
                            8  => view! { <StepFeatures config=config /> }.into_any(),
                            9  => view! { <StepServices config=config /> }.into_any(),
                            10 => view! { <StepOrchestration config=config /> }.into_any(),
                            11 => view! { <StepMemory config=config /> }.into_any(),
                            12 => view! { <StepSkills config=config /> }.into_any(),
                            13 => view! { <StepLaunch config=config /> }.into_any(),
                            _  => view! { <div /> }.into_any(),
                        }
                    }}
                </div>
            </div>

            // Navigation footer (steps 1-6 only)
            {move || {
                let s = step.get();
                if s > 0 && s < 13 {
                    view! {
                        <div style="padding: 18px 32px; border-top: 1px solid rgba(255,60,20,0.1); display: flex; justify-content: space-between; align-items: center; flex-shrink: 0; background: rgba(5,5,8,0.92); backdrop-filter: blur(16px);">
                            <button
                                style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 3px; text-transform: uppercase; background: transparent; border: 1px solid transparent; color: rgba(255,245,240,0.7); padding: 12px 28px; border-radius: 10px; cursor: pointer; transition: all 0.3s; display: inline-flex; align-items: center; gap: 8px;"
                                on:click=move |_| {
                                    let cur = step.get_untracked();
                                    // When going back from step 3 (identity), skip step 2 only in skip mode.
                                    // Manual mode KEEPS step 2 — it's the only place workspace/
                                    // sessions/max_iterations are editable (TUI parity, #216 #10/#14).
                                    if cur == 3 && config.get_untracked().onboarding_mode == "skip" {
                                        step.set(1);
                                    } else {
                                        step.update(|v| *v = v.saturating_sub(1));
                                    }
                                }
                            >
                                "← Back"
                            </button>
                            <div style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 3px; color: rgba(255,245,240,0.2);">
                                {move || format!("STEP {} OF 12", step.get())}
                            </div>
                            <button
                                style=move || {
                                    let disabled = !can_next.get();
                                    format!(
                                        "font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 3px; text-transform: uppercase; background: rgba(255,60,20,0.12); border: 1px solid rgba(255,60,20,0.25); color: rgba(255,60,20,1); padding: 12px 28px; border-radius: 10px; cursor: {}; transition: all 0.3s; display: inline-flex; align-items: center; gap: 8px; opacity: {}; box-shadow: 0 0 30px rgba(255,60,20,0.15);",
                                        if disabled { "not-allowed" } else { "pointer" },
                                        if disabled { "0.4" } else { "1" }
                                    )
                                }
                                prop:disabled=move || !can_next.get()
                                on:click=move |_| {
                                    let cur = step.get_untracked();
                                    // When advancing from step 1 (mode), skip step 2 (gateway/workspace
                                    // config) only in skip mode — manual mode needs it for workspace/
                                    // sessions/max_iterations (TUI parity, #216 #10/#14).
                                    if cur == 1 && config.get_untracked().onboarding_mode == "skip" {
                                        step.set(3);
                                    } else {
                                        step.update(|v| *v += 1);
                                    }
                                }
                            >
                                {move || if step.get() == 12 { "Ignite Zeus →" } else { "Continue →" }}
                            </button>
                        </div>
                    }.into_any()
                } else {
                    view! { <div></div> }.into_any()
                }
            }}
        </div>
    }
}

// ─── PROGRESS RING ───────────────────────────────────────

#[component]
fn ProgressRing(step: usize, total: usize) -> impl IntoView {
    let pct = ((step + 1) as f64 / total as f64) * 100.0;
    let dash = pct * 1.131;
    let gap = 113.1 - dash;
    let label = format!("{}/{}", step + 1, total);
    let dash_str = format!("{} {}", dash, gap);

    view! {
        <div style="position: relative; width: 42px; height: 42px;">
            <svg width="42" height="42" viewBox="0 0 42 42">
                <circle cx="21" cy="21" r="18" fill="none" stroke="rgba(255,60,20,0.1)" stroke-width="2.5" />
                <circle cx="21" cy="21" r="18" fill="none" stroke="rgba(255,60,20,1)" stroke-width="2.5"
                    stroke-dasharray={dash_str}
                    stroke-dashoffset="28.27"
                    stroke-linecap="round"
                    style="transition: stroke-dasharray 0.8s cubic-bezier(0.16,1,0.3,1);"
                />
            </svg>
            <div style="position: absolute; inset: 0; display: flex; align-items: center; justify-content: center; font-family: 'Orbitron', monospace; font-size: 9px; color: rgba(255,245,240,0.9); font-weight: 700;">
                {label}
            </div>
        </div>
    }
}

// ─── STEP 0: AWAKENING ──────────────────────────────────

#[component]
fn StepAwaken(step: RwSignal<usize>) -> impl IntoView {
    let phase = RwSignal::new(0u8);

    Effect::new(move |_| {
        // Play cymbal sound at awakening (70% volume, converted from WAV)
        if let Ok(audio) = web_sys::HtmlAudioElement::new_with_src("audio/sfx_cymbal_awaken.mp3") {
            audio.set_volume(0.7);
            let _ = audio.play();
        }

        let win = web_sys::window().unwrap();
        let phase_c = phase;
        let cb1 = Closure::once(move || phase_c.set(1));
        let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(cb1.as_ref().unchecked_ref(), 1200);
        cb1.forget();

        let phase_c = phase;
        let cb2 = Closure::once(move || phase_c.set(2));
        let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(cb2.as_ref().unchecked_ref(), 3000);
        cb2.forget();

        let phase_c = phase;
        let cb3 = Closure::once(move || phase_c.set(3));
        let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(cb3.as_ref().unchecked_ref(), 5000);
        cb3.forget();
    });

    view! {
        <div style="display: flex; flex-direction: column; align-items: center; justify-content: center; min-height: 80vh; text-align: center;">
            {move || view! { <SentientOrb
                size=240
                mode={if phase.get() >= 2 { "waking".to_string() } else { "dormant".to_string() }}
                intensity={0.3 + phase.get() as f64 * 0.15}
            /> }}
            <div style="margin-top: 36px; min-height: 140px;">
                <Show when=move || { phase.get() >= 1 }>
                    <FadeIn>
                        <div style="font-family: 'Orbitron', monospace; font-size: 36px; font-weight: 900; letter-spacing: 16px; color: rgba(255,245,240,0.9);">
                            "ZEUS"
                        </div>
                    </FadeIn>
                </Show>
                <Show when=move || { phase.get() >= 2 }>
                    <FadeIn delay_ms=400>
                        <div style="font-family: 'Rajdhani', sans-serif; font-size: 17px; color: rgba(255,245,240,0.7); margin-top: 12px; letter-spacing: 3px; font-weight: 500;">
                            "Autonomous Cognitive Platform"
                        </div>
                    </FadeIn>
                </Show>
                <Show when=move || { phase.get() >= 3 }>
                    <FadeIn delay_ms=1000>
                        <div style="margin-top: 24px; font-family: 'Orbitron', monospace; font-size: 11px; color: rgba(255,60,20,0.6); letter-spacing: 2px;">
                            <TypeWriter text="Initializing cognitive architecture..." speed_ms=45 delay_ms=600 />
                        </div>
                        // Security risk acknowledgement (OpenClaw parity)
                        <div style="margin-top: 20px; padding: 12px 20px; border-radius: 8px; background: rgba(234,179,8,0.06); border: 1px solid rgba(234,179,8,0.15); max-width: 460px; margin-left: auto; margin-right: auto;">
                            <div style="font-size: 12px; color: rgba(234,179,8,0.8); line-height: 1.6;">
                                "⚠ Zeus executes tools on your machine — shell commands, file operations, and network requests. By continuing, you acknowledge this risk."
                            </div>
                        </div>
                        <div style="margin-top: 24px;">
                            <button
                                style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 3px; text-transform: uppercase; background: rgba(255,60,20,0.12); border: 1px solid rgba(255,60,20,0.25); color: rgba(255,60,20,1); padding: 12px 28px; border-radius: 10px; cursor: pointer; transition: all 0.3s; display: inline-flex; align-items: center; gap: 8px; box-shadow: 0 0 30px rgba(255,60,20,0.15);"
                                on:click=move |_| step.set(1)
                            >
                                "I Understand — Begin Awakening"
                            </button>
                        </div>
                    </FadeIn>
                </Show>
            </div>
        </div>
    }
}

// ─── STEP 1: ONBOARDING MODE ───────────────────────────────────

#[component]
fn StepOnboardingMode(config: RwSignal<OnboardConfig>) -> impl IntoView {
    let modes = [
        ("quickstart", "⚡ QuickStart", "Configure basics now, start immediately", "Recommended — review gateway settings, then jump straight into provider & channel setup."),
        ("manual", "🔧 Manual", "Configure details later via config.toml", "Save defaults and edit config.toml manually. For advanced users who know what they want."),
        ("skip", "⏭ Skip", "Skip setup entirely", "Use existing configuration as-is. Only if you've already configured Zeus before."),
    ];

    view! {
        <div class="animate-fade-in">
            <div style="margin-bottom: 28px;">
                <div class="font-rajdhani text-[22px] font-semibold text-z-text">
                    <TypeWriter text="How would you like to set up Zeus?" speed_ms=40 />
                </div>
                <FadeIn delay_ms=800>
                    <p class="text-sm text-white/60 leading-relaxed">
                        "Choose your awakening protocol. QuickStart walks you through everything step by step."
                    </p>
                </FadeIn>
            </div>

            <div style="display: flex; flex-direction: column; gap: 12px;">
                {modes.into_iter().map(|(id, label, subtitle, detail)| {
                    let id_style = id.to_string();
                    let id_click = id.to_string();
                    let id_radio = id.to_string();
                    let id_inner = id.to_string();
                    let id_detail = id.to_string();
                    view! {
                        <div
                            style=move || {
                                let selected = config.get().onboarding_mode == id_style;
                                format!(
                                    "padding: 20px 24px; border-radius: 12px; cursor: pointer; transition: all 0.3s cubic-bezier(0.16,1,0.3,1); border: 1px solid {}; background: {};",
                                    if selected { "rgba(255,60,20,0.4)" } else { "rgba(255,60,20,0.08)" },
                                    if selected { "rgba(255,60,20,0.08)" } else { "rgba(255,255,255,0.015)" }
                                )
                            }
                            on:click=move |_| {
                                config.update(|c| c.onboarding_mode = id_click.clone());
                            }
                        >
                            <div class="flex items-center gap-3.5">
                                <div style=move || {
                                    let selected = config.get().onboarding_mode == id_radio;
                                    format!(
                                        "width: 18px; height: 18px; border-radius: 50%; border: 2px solid {}; display: flex; align-items: center; justify-content: center; flex-shrink: 0; transition: all 0.3s;",
                                        if selected { "rgba(255,60,20,1)" } else { "rgba(255,60,20,0.2)" }
                                    )
                                }>
                                    {move || {
                                        let selected = config.get().onboarding_mode == id_inner;
                                        if selected {
                                            view! { <div style="width: 8px; height: 8px; border-radius: 50%; background: rgba(255,60,20,1);" /> }.into_any()
                                        } else {
                                            view! { <div /> }.into_any()
                                        }
                                    }}
                                </div>
                                <div>
                                    <div style="font-family: 'Rajdhani', sans-serif; font-size: 17px; font-weight: 600; color: rgba(255,245,240,0.95);">
                                        {label}
                                    </div>
                                    <div style="font-size: 13px; color: rgba(255,245,240,0.6); margin-top: 2px;">
                                        {subtitle}
                                    </div>
                                </div>
                            </div>
                            {move || {
                                let selected = config.get().onboarding_mode == id_detail;
                                if selected {
                                    view! {
                                        <div style="margin-top: 12px; margin-left: 32px; padding: 10px 14px; background: rgba(255,255,255,0.02); border-radius: 8px; font-size: 12px; color: rgba(255,245,240,0.5); line-height: 1.6;">
                                            {detail}
                                        </div>
                                    }.into_any()
                                } else {
                                    view! { <div /> }.into_any()
                                }
                            }}
                        </div>
                    }
                }).collect_view()}
            </div>
        </div>
    }
}

// ─── STEP 2: QUICKSTART CONFIG ──────────────────────────────────

#[component]
fn StepQuickStartConfig(config: RwSignal<OnboardConfig>) -> impl IntoView {
    let input_style = "width: 100%; padding: 12px 16px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 10px; color: rgba(255,245,240,0.9); font-family: 'Rajdhani', sans-serif; font-size: 15px; outline: none; box-sizing: border-box; transition: all 0.3s;";
    let label_style = "font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 3px; color: rgba(255,245,240,0.7); display: block; margin-bottom: 6px; text-transform: uppercase;";
    let hint_style = "font-size: 11px; color: rgba(255,245,240,0.35); margin-top: 4px; margin-left: 4px; font-style: italic;";

    view! {
        <div class="animate-fade-in">
            <div style="margin-bottom: 28px;">
                <div class="font-rajdhani text-[22px] font-semibold text-z-text">
                    <TypeWriter text="Zeus Gateway — API & Web Server" speed_ms=40 />
                </div>
                <FadeIn delay_ms=800>
                    <p class="text-sm text-white/60 leading-relaxed">
                        "These settings control the Zeus HTTP API and web frontend. Review the defaults below, or edit as needed."
                    </p>
                </FadeIn>
            </div>

            <div style="display: flex; flex-direction: column; gap: 14px;">
                <div>
                    <label style=label_style>"API PORT"</label>
                    <input style=input_style type="text"
                        prop:value=move || config.get().qs_port.clone()
                        on:input=move |ev| config.update(|c| c.qs_port = event_target_value(&ev))
                    />
                    <div style=hint_style>"HTTP API & web frontend port"</div>
                </div>
                <div>
                    <label style=label_style>"LISTEN ADDRESS"</label>
                    <input style=input_style type="text"
                        prop:value=move || config.get().qs_bind.clone()
                        on:input=move |ev| config.update(|c| c.qs_bind = event_target_value(&ev))
                    />
                    <div style=hint_style>"Network interface (0.0.0.0 = all, 127.0.0.1 = local only)"</div>
                </div>
                <div>
                    <label style=label_style>"WORKSPACE"</label>
                    <input style=input_style type="text"
                        prop:value=move || config.get().qs_workspace.clone()
                        on:input=move |ev| config.update(|c| c.qs_workspace = event_target_value(&ev))
                    />
                    <div style=hint_style>"Where Zeus stores memory, notes, and agent files"</div>
                </div>
                <div>
                    <label style=label_style>"SESSIONS DIR"</label>
                    <input style=input_style type="text"
                        prop:value=move || config.get().qs_sessions.clone()
                        on:input=move |ev| config.update(|c| c.qs_sessions = event_target_value(&ev))
                    />
                    <div style=hint_style>"Where chat sessions are saved"</div>
                </div>
                <div>
                    <label style=label_style>"MAX ITERATIONS"</label>
                    <input style=input_style type="text"
                        prop:value=move || config.get().qs_max_iterations.clone()
                        on:input=move |ev| config.update(|c| c.qs_max_iterations = event_target_value(&ev))
                    />
                    <div style=hint_style>"Max tool-call rounds per request"</div>
                </div>
            </div>

            // Rate Limiting
            <div style="margin-top: 28px; border-top: 1px solid rgba(255,60,20,0.1); padding-top: 20px;">
                <div style="font-family: 'Orbitron', monospace; font-size: 11px; letter-spacing: 2px; color: rgba(255,245,240,0.7); margin-bottom: 14px;">
                    "RATE LIMITING"
                </div>
                <div style="display: flex; flex-direction: column; gap: 14px;">
                    <div
                        style=move || {
                            let active = config.get().rate_limit_enabled;
                            format!("display: flex; align-items: center; gap: 16px; padding: 16px 20px; border-radius: 12px; cursor: pointer; transition: all 0.3s ease; background: rgba(255,255,255,0.03); border: 1px solid {};",
                                if active { "rgba(255,60,20,0.25)" } else { "rgba(255,60,20,0.1)" })
                        }
                        on:click=move |_| config.update(|c| c.rate_limit_enabled = !c.rate_limit_enabled)
                    >
                        <div style=move || format!(
                            "width: 44px; height: 24px; border-radius: 12px; padding: 2px; flex-shrink: 0; background: {}; transition: all 0.3s; cursor: pointer;",
                            if config.get().rate_limit_enabled { "rgba(255,60,20,0.4)" } else { "rgba(255,255,255,0.08)" }
                        )>
                            <div style=move || format!(
                                "width: 20px; height: 20px; border-radius: 10px; background: {}; transition: all 0.3s cubic-bezier(0.16,1,0.3,1); transform: {};",
                                if config.get().rate_limit_enabled { "#fff" } else { "rgba(255,255,255,0.2)" },
                                if config.get().rate_limit_enabled { "translateX(20px)" } else { "translateX(0)" }
                            ) />
                        </div>
                        <div style="flex: 1;">
                            <div style=move || format!("font-size: 15px; font-weight: 700; color: {};",
                                if config.get().rate_limit_enabled { "rgba(255,245,240,0.9)" } else { "rgba(255,245,240,0.7)" })>"Enable Rate Limiting"</div>
                            <div style=move || format!("font-size: 12px; color: {}; margin-top: 2px;",
                                if config.get().rate_limit_enabled { "rgba(255,245,240,0.7)" } else { "rgba(255,245,240,0.5)" })>"Throttle LLM requests to prevent runaway costs"</div>
                        </div>
                    </div>
                    <Show when=move || config.get().rate_limit_enabled>
                        <div>
                            <label style=label_style>"LLM REQUESTS/MIN"</label>
                            <input style=input_style type="text"
                                prop:value=move || config.get().rate_limit_rpm.clone()
                                on:input=move |ev| config.update(|c| c.rate_limit_rpm = event_target_value(&ev))
                            />
                            <div style=hint_style>"Maximum LLM API calls per minute"</div>
                        </div>
                    </Show>
                </div>
            </div>

            // Session Management
            <div style="margin-top: 28px; border-top: 1px solid rgba(255,60,20,0.1); padding-top: 20px;">
                <div style="font-family: 'Orbitron', monospace; font-size: 11px; letter-spacing: 2px; color: rgba(255,245,240,0.7); margin-bottom: 14px;">
                    "SESSION MANAGEMENT"
                </div>
                <div style="display: flex; flex-direction: column; gap: 14px;">
                    <div>
                        <label style=label_style>"MAX CONTEXT TOKENS"</label>
                        <input style=input_style type="text"
                            prop:value=move || config.get().compaction_max_tokens.clone()
                            on:input=move |ev| config.update(|c| c.compaction_max_tokens = event_target_value(&ev))
                        />
                        <div style=hint_style>"Maximum tokens before session compaction triggers"</div>
                    </div>
                    <div>
                        <label style=label_style>"COMPACTION THRESHOLD"</label>
                        <input style=input_style type="text"
                            prop:value=move || config.get().compaction_threshold.clone()
                            on:input=move |ev| config.update(|c| c.compaction_threshold = event_target_value(&ev))
                        />
                        <div style=hint_style>"Fraction of max tokens that triggers compaction (0.0 - 1.0)"</div>
                    </div>
                </div>
            </div>
        </div>
    }
}

// ─── STEP 3: IDENTITY ──────────────────────────────────────────

#[derive(Clone, serde::Deserialize, Default)]
struct PersonalityItem {
    id: String,
    name: String,
    description: String,
}

#[component]
fn StepIdentity(config: RwSignal<OnboardConfig>) -> impl IntoView {
    // Dynamic personalities fetched from API, fallback to hardcoded defaults
    let personalities: RwSignal<Vec<PersonalityItem>> = RwSignal::new(vec![
        PersonalityItem { id: "coordinator".to_string(), name: "The Coordinator".to_string(), description: "Fleet commander, sprint driver, orchestrator".to_string() },
        PersonalityItem { id: "professional".to_string(), name: "Professional".to_string(), description: "Formal, precise, efficient".to_string() },
        PersonalityItem { id: "collaborative".to_string(), name: "Collaborative".to_string(), description: "Friendly partner, proactive".to_string() },
        PersonalityItem { id: "minimal".to_string(), name: "Minimal".to_string(), description: "Terse, no fluff, results-only".to_string() },
        PersonalityItem { id: "autonomous".to_string(), name: "Autonomous".to_string(), description: "Acts first, reports after".to_string() },
    ]);

    // Fetch personalities from API
    spawn_local({
        async move {
            if let Ok(resp) = api::fetch_json::<serde_json::Value>("/v1/onboarding/personalities").await {
                if let Some(arr) = resp.as_array() {
                    let fetched: Vec<PersonalityItem> = arr.iter().filter_map(|v| {
                        Some(PersonalityItem {
                            id: v["id"].as_str()?.to_string(),
                            name: v["name"].as_str()?.to_string(),
                            description: v["description"].as_str().unwrap_or("").to_string(),
                        })
                    }).collect();
                    if !fetched.is_empty() {
                        personalities.set(fetched);
                    }
                }
            }
        }
    });

    view! {
        <div class="animate-fade-in">
            <div style="margin-bottom: 28px;">
                <div class="font-rajdhani text-[22px] font-semibold text-z-text">
                    <TypeWriter text="Who am I serving?" speed_ms=40 />
                </div>
                <FadeIn delay_ms=1000>
                    <p class="text-sm text-white/60 leading-relaxed">
                        "Zeus adapts to your identity. This shapes the system prompt, workspace structure, and how Zeus communicates with you across all channels."
                    </p>
                </FadeIn>
            </div>

            // Agent name input (TUI parity: first field, defaults to hostname)
            <div class="mb-4">
                <label class="font-orbitron text-[10px] tracking-[3px] text-white/70 block mb-2 uppercase">"Agent Name"</label>
                <input
                    style="width: 100%; padding: 12px 16px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 10px; color: rgba(255,245,240,0.9); font-family: 'Rajdhani', sans-serif; font-size: 15px; outline: none; box-sizing: border-box; transition: all 0.3s;"
                    type="text"
                    placeholder="e.g. Zeus112"
                    prop:value=move || config.get().agent_name.clone()
                    on:input=move |ev| config.update(|c| c.agent_name = event_target_value(&ev))
                />
                <div style="font-size: 11px; color: rgba(255,245,240,0.3); margin-top: 6px;">
                    "What this agent calls itself — defaults to the machine's hostname."
                </div>
            </div>

            // Name input
            <div class="mb-4">
                <label class="font-orbitron text-[10px] tracking-[3px] text-white/70 block mb-2 uppercase">"Your Name"</label>
                <input
                    style="width: 100%; padding: 12px 16px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 10px; color: rgba(255,245,240,0.9); font-family: 'Rajdhani', sans-serif; font-size: 15px; outline: none; box-sizing: border-box; transition: all 0.3s;"
                    type="text"
                    placeholder="e.g. Miguel"
                    prop:value=move || config.get().user_name.clone()
                    on:input=move |ev| config.update(|c| c.user_name = event_target_value(&ev))
                />
            </div>

            // Role input
            <div class="mb-4">
                <label class="onboarding-input-label">"Role / Title"</label>
                <input
                    class="onboarding-input"
                    type="text"
                    placeholder="e.g. Co-Founder & COO"
                    prop:value=move || config.get().user_role.clone()
                    on:input=move |ev| config.update(|c| c.user_role = event_target_value(&ev))
                />
            </div>

            // Org input
            <div class="mb-4">
                <label class="font-orbitron text-[10px] tracking-[3px] text-white/70 block mb-2 uppercase">"Organization"</label>
                <input
                    style="width: 100%; padding: 12px 16px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 10px; color: rgba(255,245,240,0.9); font-family: 'Rajdhani', sans-serif; font-size: 15px; outline: none; box-sizing: border-box; transition: all 0.3s;"
                    type="text"
                    placeholder="e.g. NovaXAI"
                    prop:value=move || config.get().user_org.clone()
                    on:input=move |ev| config.update(|c| c.user_org = event_target_value(&ev))
                />
            </div>

            // Gateway URL
            <div class="mb-4">
                <label class="font-orbitron text-[10px] tracking-[3px] text-white/70 block mb-2 uppercase">"Gateway URL"</label>
                <input
                    style="width: 100%; padding: 12px 16px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 10px; color: rgba(255,245,240,0.9); font-family: 'Rajdhani', sans-serif; font-size: 15px; outline: none; box-sizing: border-box; transition: all 0.3s;"
                    type="text"
                    placeholder="https://your-zeus-server.com:8080"
                    prop:value=move || config.get().gateway_url.clone()
                    on:input=move |ev| config.update(|c| c.gateway_url = event_target_value(&ev))
                />
                <div style="font-size: 11px; color: rgba(255,245,240,0.3); margin-top: 6px;">
                    "Your Zeus gateway server address. Examples: " <code style="color: rgba(255,60,20,0.7);">"http://localhost:8080"</code> " (local) or " <code style="color: rgba(255,60,20,0.7);">"https://zeus.yourcompany.com"</code> " (remote)"
                </div>
            </div>

            // Personality cards (2x2 grid)
            <div style="margin-top: 8px;">
                <label style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 3px; color: rgba(255,245,240,0.7); display: block; margin-bottom: 12px; text-transform: uppercase;">"Zeus Personality"</label>
                <div class="grid grid-cols-2 gap-2.5">
                    {move || personalities.get().into_iter().map(|p| {
                        let pid = p.id.clone();
                        let pid_c1 = pid.clone();
                        let pid_c2 = pid.clone();
                        let pid_c3 = pid.clone();
                        let pid_c4 = pid.clone();
                        let pid_c5 = pid.clone();
                        let label = p.name.clone();
                        let desc = p.description.clone();
                        view! {
                            <div
                                style=move || {
                                    let sel = config.get().personality == pid_c1;
                                    format!("padding: 18px; border-radius: 12px; cursor: pointer; transition: all 0.3s ease; background: rgba(255,255,255,0.03); border: 1px solid {}; box-shadow: {};",
                                        if sel { "rgba(255,60,20,0.25)" } else { "rgba(255,60,20,0.1)" },
                                        if sel { "0 0 30px rgba(255,60,20,0.15)" } else { "none" }
                                    )
                                }
                                on:click={
                                    let pid = pid.clone();
                                    move |_| config.update(|c| c.personality = pid.clone())
                                }
                            >
                                <div style="display: flex; align-items: flex-start; gap: 12px;">
                                    <div style=move || format!(
                                        "width: 18px; height: 18px; border-radius: 9px; flex-shrink: 0; border: 1.5px solid {}; display: flex; align-items: center; justify-content: center; transition: all 0.3s;",
                                        if config.get().personality == pid_c2 { "rgba(255,60,20,1)" } else { "rgba(255,60,20,0.1)" }
                                    )>
                                        <Show when=move || config.get().personality == pid_c3>
                                            <div style="width: 6px; height: 6px; border-radius: 3px; background: rgba(255,60,20,1);" />
                                        </Show>
                                    </div>
                                    <div>
                                        <div style=move || format!("font-size: 15px; font-weight: 700; color: {}; margin-bottom: 4px;",
                                            if config.get().personality == pid_c4 { "rgba(255,245,240,0.9)" } else { "rgba(255,245,240,0.7)" })>
                                            {label.clone()}
                                        </div>
                                        <div style=move || format!("font-size: 12px; color: {};",
                                            if config.get().personality == pid_c5 { "rgba(255,245,240,0.7)" } else { "rgba(255,245,240,0.5)" })>
                                            {desc.clone()}
                                        </div>
                                    </div>
                                </div>
                            </div>
                        }
                    }).collect::<Vec<_>>()}
                </div>
            </div>
        </div>
    }
}

// ─── STEP 2: INTELLIGENCE (PROVIDERS) ───────────────────

#[component]
fn StepIntelligence(config: RwSignal<OnboardConfig>, providers_data: RwSignal<Vec<DynProvider>>, key_status: RwSignal<std::collections::HashMap<String, KeyTestStatus>>) -> impl IntoView {
    // Detect existing credentials on mount — auto-select providers with keys
    let detected = RwSignal::new(std::collections::HashSet::<String>::new());
    // #216b: browser OAuth flow state — "idle" | "waiting" | "done" | "error: …"
    let oauth_browser = RwSignal::new("idle".to_string());
    // Poll generation counter: bumping it invalidates any in-flight poll loop.
    let oauth_poll_gen = RwSignal::new(0u32);
    // #216c: device-code OAuth flow state (qwen/minimax) —
    // "idle" | "starting" | "waiting:<user_code>|<verification_uri>" | "done" | "error: …"
    let device_flow = RwSignal::new("idle".to_string());
    // Which provider the current device flow belongs to ("qwen" | "minimax")
    let device_provider = RwSignal::new(String::new());
    // Poll generation counter for device-code (mirrors oauth_poll_gen semantics)
    let device_poll_gen = RwSignal::new(0u32);
    // #216c: Gemini CLI creds import state — "unknown" | "found" | "not_found" | "imported" | "error: …"
    let gemini_cli = RwSignal::new("unknown".to_string());
    Effect::new(move |_| {
        spawn_local(async move {
            if let Ok(keys) = crate::api::fetch_keys().await {
                let mut found = std::collections::HashSet::new();
                for key in &keys.keys {
                    if key.configured {
                        // Map env var names to provider IDs
                        let pid = match key.env_var.as_str() {
                            "ANTHROPIC_API_KEY" => "anthropic",
                            "OPENAI_API_KEY" => "openai",
                            "GOOGLE_API_KEY" => "google",
                            "MOONSHOT_API_KEY" => "moonshot",
                            "ZAI_API_KEY" => "zai",
                            "QWEN_API_KEY" => "qwen",
                            "MINIMAX_API_KEY" => "minimax",
                            _ => continue,
                        };
                        // Single-active selection: auto-select only the FIRST detected
                        // provider (the rest are still recorded in `found`/key_status so
                        // their key-status badges render, but selection stays single).
                        let is_first = found.is_empty();
                        found.insert(pid.to_string());
                        if is_first {
                            config.update(|c| {
                                c.providers.clear();
                                c.providers.push(pid.to_string());
                            });
                        }
                        // Mark as detected (not tested yet, but key exists)
                        key_status.update(|m| {
                            if !m.contains_key(pid) {
                                m.insert(pid.to_string(), KeyTestStatus::InfoOnly);
                            }
                        });
                    }
                }
                detected.set(found);
            }
        });
    });

    view! {
        <div class="animate-fade-in">
            <div class="mb-5">
                <div class="font-orbitron text-[22px] font-semibold text-z-text">
                    <TypeWriter text="Choose your intelligence sources" speed_ms=40 />
                </div>
                <FadeIn delay_ms=1000>
                    <p class="text-sm text-white/60 leading-relaxed">
                        "Zeus connects to multiple LLM providers through a unified interface. Enable the ones you have API keys for."
                    </p>
                    <Show when=move || !detected.get().is_empty()>
                        <p style="font-family: 'Orbitron', monospace; font-size: 10px; color: rgba(100,255,100,0.8); letter-spacing: 1px; margin-top: 8px;">
                            {move || format!("⚡ {} credential(s) detected from environment", detected.get().len())}
                        </p>
                    </Show>
                </FadeIn>
            </div>

            <div class="grid grid-cols-2 gap-2.5">
                {providers_data.get().iter().map(|p| {
                    let pid = p.id.clone();
                    let pid_c1 = pid.clone();
                    let pid_c2 = pid.clone();
                    let pid_c3 = pid.clone();
                    let pid_c4 = pid.clone();
                    let pid_c5 = pid.clone();
                    let pid_c6 = pid.clone();
                    let pid_c7 = pid.clone();
                    let pid_c8 = pid.clone();
                    let pid_c9 = pid.clone();
                    let pid_c10 = pid.clone();
                    let pid_c11 = pid.clone();
                    let pid_c12 = pid.clone();
                    let pid_bedrock = pid.clone();
                    let name = p.name.clone();
                    let desc = p.desc.clone();
                    let color = p.color.clone();
                    let abbr = if p.name.len() >= 2 { p.name[..2].to_uppercase() } else { p.name.to_uppercase() };
                    let is_hot = p.hot;
                    let is_local = p.local;
                    let pid_auth1 = pid.clone();
                    let pid_auth2 = pid.clone();
                    let pid_auth3 = pid.clone();
                    let pid_auth4 = pid.clone();
                    let pid_auth5 = pid.clone();
                    let pid_auth6 = pid.clone();
                    // #216b: browser OAuth login (anthropic only)
                    let pid_browser1 = pid.clone();
                    let pid_browser2 = pid.clone();
                    let pid_browser3 = pid.clone();
                    let pid_browser4 = pid.clone();
                    // #216c: device-code OAuth (qwen/minimax) + Gemini CLI import
                    let pid_device1 = pid.clone();
                    let pid_device2 = pid.clone();
                    let pid_device3 = pid.clone();
                    let pid_device4 = pid.clone();
                    let pid_gemini1 = pid.clone();
                    let name_c = name.clone();
                    let name_c2 = name.clone();

                    view! {
                        <div
                            style=move || {
                                let active = config.get().providers.contains(&pid_c1);
                                format!("padding: 18px; border-radius: 12px; cursor: pointer; transition: all 0.3s ease; background: rgba(255,255,255,0.03); border: 1px solid {}; box-shadow: {};",
                                    if active { "rgba(255,60,20,0.25)" } else { "rgba(255,60,20,0.1)" },
                                    if active { "0 0 30px rgba(255,60,20,0.15)" } else { "none" })
                            }
                            on:click={
                                let pid = pid.clone();
                                // Single-active selection: clicking a provider makes it the
                                // sole selected provider (replaces any prior selection).
                                // Re-clicking the active provider is a no-op (keeps exactly
                                // one selected, which the step-4 credential gate requires).
                                move |_| config.update(|c| {
                                    if c.providers.len() == 1 && c.providers[0] == pid {
                                        return;
                                    }
                                    c.providers.clear();
                                    c.providers.push(pid.clone());
                                })
                            }
                        >
                            <div style="display: flex; align-items: center; gap: 10px;">
                                // Provider icon (2-letter)
                                <div style={format!("width: 36px; height: 36px; border-radius: 10px; background: {}12; border: 1px solid {}30; display: flex; align-items: center; justify-content: center; font-family: 'Orbitron', monospace; font-size: 10px; font-weight: 900; color: {};",  color, color, color)}>
                                    {abbr}
                                </div>
                                <div class="flex-1">
                                    <div style="display: flex; align-items: center; gap: 6px;">
                                        // Fix #3a: provider name color is dynamic (selected=bright, unselected=dim)
                                        <span style=move || format!("font-size: 14px; font-weight: 700; color: {};",
                                            if config.get().providers.contains(&pid_c4) { "rgba(255,245,240,0.9)" } else { "rgba(255,245,240,0.7)" })>
                                            {name}
                                        </span>
                                        {is_hot.then(|| view! {
                                            <span style="font-family: 'Orbitron', monospace; font-size: 8px; font-weight: 700; letter-spacing: 1.5px; color: rgba(255,60,20,1); background: rgba(255,60,20,0.20); padding: 3px 8px; border-radius: 4px;">"HOT"</span>
                                        })}
                                        {is_local.then(|| view! {
                                            <span style="font-family: 'Orbitron', monospace; font-size: 8px; font-weight: 700; letter-spacing: 1.5px; color: #22c55e; background: rgba(34,197,94,0.20); padding: 3px 8px; border-radius: 4px;">"LOCAL"</span>
                                        })}
                                    </div>
                                    <div style=move || format!("font-size: 11px; color: {}; margin-top: 2px;",
                                        if config.get().providers.contains(&pid_c8) { "rgba(255,245,240,0.7)" } else { "rgba(255,245,240,0.5)" })>{desc}</div>
                                </div>
                                // Checkbox
                                <div style=move || format!(
                                    "width: 22px; height: 22px; border-radius: 6px; flex-shrink: 0; border: 1.5px solid {}; background: {}; display: flex; align-items: center; justify-content: center; transition: all 0.3s;",
                                    if config.get().providers.contains(&pid_c2) { "rgba(255,60,20,1)" } else { "rgba(255,60,20,0.1)" },
                                    if config.get().providers.contains(&pid_c2) { "rgba(255,60,20,0.2)" } else { "transparent" }
                                )>
                                    <Show when=move || config.get().providers.contains(&pid_c3)>
                                        <div style="width: 10px; height: 10px; border-radius: 3px; background: rgba(255,60,20,1);" />
                                    </Show>
                                </div>
                            </div>
                            // API key input (cloud providers) or URL input (local providers like Ollama)
                            <div
                                on:click=|ev: web_sys::MouseEvent| ev.stop_propagation()
                                style=move || format!(
                                "margin-top: 14px; padding-top: 14px; border-top: 1px solid rgba(255,60,20,0.1); display: {};", 
                                if config.get().providers.contains(&pid_c5) && !is_local { "block" } else { "none" }
                            )>
                                // Auth type toggle: API Key vs Claude Token
                                <div style="display: flex; align-items: center; gap: 8px; margin-bottom: 10px;">
                                    <label style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 3px; color: rgba(255,245,240,0.7); text-transform: uppercase;">
                                        {move || {
                                            let auth = config.get().auth_types.get(&pid_auth1).cloned().unwrap_or_else(|| "api_key".to_string());
                                            if auth == "oauth_token" { format!("{} OAUTH TOKEN", name_c) } else { format!("{} API KEY", name_c) }
                                        }}
                                    </label>
                                    <div style="margin-left: auto; display: flex; gap: 4px;">
                                        <button
                                            style=move || format!(
                                                "padding: 3px 8px; border-radius: 4px; font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 1px; cursor: pointer; border: 1px solid rgba(255,60,20,0.2); transition: all 0.3s; background: {}; color: {};",
                                                if config.get().auth_types.get(&pid_auth2).cloned().unwrap_or_else(|| "api_key".to_string()) == "api_key" { "rgba(255,60,20,0.2)" } else { "transparent" },
                                                if config.get().auth_types.get(&pid_auth2).cloned().unwrap_or_else(|| "api_key".to_string()) == "api_key" { "rgba(255,60,20,1)" } else { "rgba(255,245,240,0.5)" }
                                            )
                                            on:click={
                                                let pid = pid_auth3.clone();
                                                move |_| config.update(|c| { c.auth_types.insert(pid.clone(), "api_key".to_string()); })
                                            }
                                        >"API KEY"</button>
                                        <button
                                            style=move || format!(
                                                "padding: 3px 8px; border-radius: 4px; font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 1px; cursor: pointer; border: 1px solid rgba(255,60,20,0.2); transition: all 0.3s; background: {}; color: {};",
                                                if config.get().auth_types.get(&pid_auth4).cloned().unwrap_or_else(|| "api_key".to_string()) == "oauth_token" { "rgba(255,60,20,0.2)" } else { "transparent" },
                                                if config.get().auth_types.get(&pid_auth4).cloned().unwrap_or_else(|| "api_key".to_string()) == "oauth_token" { "rgba(255,60,20,1)" } else { "rgba(255,245,240,0.5)" }
                                            )
                                            on:click={
                                                let pid = pid_auth6.clone();
                                                move |_| config.update(|c| { c.auth_types.insert(pid.clone(), "oauth_token".to_string()); })
                                            }
                                        >"CLAUDE TOKEN"</button>
                                        // #216b: browser OAuth — anthropic only (gateway PKCE flow)
                                        {(pid_browser1 == "anthropic").then(|| {
                                            let pid_b = pid_browser2.clone();
                                            view! {
                                                <button
                                                    style=move || format!(
                                                        "padding: 3px 8px; border-radius: 4px; font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 1px; cursor: pointer; border: 1px solid rgba(255,60,20,0.2); transition: all 0.3s; background: {}; color: {};",
                                                        if config.get().auth_types.get("anthropic").map(|t| t == "oauth_browser").unwrap_or(false) { "rgba(255,60,20,0.2)" } else { "transparent" },
                                                        if config.get().auth_types.get("anthropic").map(|t| t == "oauth_browser").unwrap_or(false) { "rgba(255,60,20,1)" } else { "rgba(255,245,240,0.5)" }
                                                    )
                                                    on:click=move |_| {
                                                        let pid = pid_b.clone();
                                                        config.update(|c| { c.auth_types.insert(pid, "oauth_browser".to_string()); });
                                                    }
                                                >"LOGIN WITH BROWSER"</button>
                                            }
                                        })}
                                        // #216c: device-code OAuth — qwen/minimax only
                                        {(pid_device1 == "qwen" || pid_device1 == "minimax").then(|| {
                                            let pid_d = pid_device2.clone();
                                            let pid_d2 = pid_device2.clone();
                                            view! {
                                                <button
                                                    style=move || format!(
                                                        "padding: 3px 8px; border-radius: 4px; font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 1px; cursor: pointer; border: 1px solid rgba(255,60,20,0.2); transition: all 0.3s; background: {}; color: {};",
                                                        if config.get().auth_types.get(&pid_d).map(|t| t == "oauth_device").unwrap_or(false) { "rgba(255,60,20,0.2)" } else { "transparent" },
                                                        if config.get().auth_types.get(&pid_d).map(|t| t == "oauth_device").unwrap_or(false) { "rgba(255,60,20,1)" } else { "rgba(255,245,240,0.5)" }
                                                    )
                                                    on:click=move |_| {
                                                        let pid = pid_d2.clone();
                                                        config.update(|c| { c.auth_types.insert(pid, "oauth_device".to_string()); });
                                                    }
                                                >"DEVICE CODE"</button>
                                            }
                                        })}
                                    </div>
                                </div>
                                // #216b: browser OAuth panel (anthropic + oauth_browser mode)
                                {(pid_browser3 == "anthropic").then(|| view! {
                                    <Show when=move || config.get().auth_types.get("anthropic").map(|t| t == "oauth_browser").unwrap_or(false)>
                                        <div style="margin-bottom: 10px; padding: 12px; background: rgba(255,60,20,0.04); border: 1px solid rgba(255,60,20,0.15); border-radius: 10px;">
                                            <Show when=move || oauth_browser.get() == "idle" || oauth_browser.get().starts_with("error")>
                                                <div style="font-size: 10px; color: rgba(255,245,240,0.55); margin-bottom: 8px; font-family: 'Orbitron', monospace; letter-spacing: 0.5px; line-height: 1.6;">
                                                    "Sign in with your Claude account. A new tab opens to console.anthropic.com; the gateway completes the PKCE exchange at /v1/auth/anthropic/callback and stores the credential."
                                                </div>
                                                <Show when=move || oauth_browser.get().starts_with("error")>
                                                    <div style="font-size: 10px; color: rgba(255,120,80,0.9); margin-bottom: 8px; font-family: 'Orbitron', monospace;">
                                                        {move || oauth_browser.get()}
                                                    </div>
                                                </Show>
                                                <button
                                                    style="padding: 10px 18px; border-radius: 8px; font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 2px; cursor: pointer; border: 1px solid rgba(255,60,20,0.4); background: rgba(255,60,20,0.15); color: rgba(255,245,240,0.95); transition: all 0.3s;"
                                                    on:click=move |_| {
                                                        // Open the gateway's OAuth login (302 → console.anthropic.com)
                                                        if let Some(win) = web_sys::window() {
                                                            let _ = win.open_with_url_and_target("/v1/auth/anthropic/login", "_blank");
                                                        }
                                                        oauth_browser.set("waiting".to_string());
                                                        // Invalidate any previous poll loop, start a new one
                                                        let my_gen = oauth_poll_gen.get_untracked() + 1;
                                                        oauth_poll_gen.set(my_gen);
                                                        spawn_local(async move {
                                                            // Poll status every 2s, up to 5 minutes
                                                            for _ in 0..150 {
                                                                gloo_timers::future::TimeoutFuture::new(2_000).await;
                                                                if oauth_poll_gen.get_untracked() != my_gen
                                                                    || oauth_browser.get_untracked() != "waiting" {
                                                                    return; // superseded or cancelled
                                                                }
                                                                if let Ok(st) = crate::api::fetch_anthropic_auth_status().await {
                                                                    if st.authenticated && st.method != "api_key" {
                                                                        config.update(|c| {
                                                                            c.auth_types.insert("anthropic".to_string(), "oauth_browser".to_string());
                                                                            if !c.providers.contains(&"anthropic".to_string()) {
                                                                                c.providers.clear();
                                                                                c.providers.push("anthropic".to_string());
                                                                            }
                                                                        });
                                                                        key_status.update(|m| { m.insert("anthropic".to_string(), KeyTestStatus::Valid); });
                                                                        oauth_browser.set("done".to_string());
                                                                        return;
                                                                    }
                                                                }
                                                            }
                                                            if oauth_browser.get_untracked() == "waiting" {
                                                                oauth_browser.set("error: timed out after 5 minutes — try again".to_string());
                                                            }
                                                        });
                                                    }
                                                >"⚡ LOGIN WITH BROWSER"</button>
                                            </Show>
                                            <Show when=move || oauth_browser.get() == "waiting">
                                                <div style="display: flex; align-items: center; gap: 10px;">
                                                    <div style="width: 10px; height: 10px; border-radius: 50%; background: rgba(255,180,0,0.9); animation: pulse 1.2s infinite;" />
                                                    <span style="font-size: 11px; color: rgba(255,210,80,0.9); font-family: 'Orbitron', monospace; letter-spacing: 1px;">
                                                        "WAITING FOR BROWSER SIGN-IN — complete the flow in the opened tab"
                                                    </span>
                                                    <button
                                                        style="margin-left: auto; padding: 3px 8px; border-radius: 4px; font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 1px; cursor: pointer; border: 1px solid rgba(255,60,20,0.2); background: transparent; color: rgba(255,245,240,0.5);"
                                                        on:click=move |_| {
                                                            oauth_poll_gen.update(|g| *g += 1);
                                                            oauth_browser.set("idle".to_string());
                                                        }
                                                    >"CANCEL"</button>
                                                </div>
                                            </Show>
                                            <Show when=move || oauth_browser.get() == "done">
                                                <div style="display: flex; align-items: center; gap: 10px;">
                                                    <span style="font-size: 13px;">"✅"</span>
                                                    <span style="font-size: 11px; color: rgba(100,255,100,0.9); font-family: 'Orbitron', monospace; letter-spacing: 1px;">
                                                        "AUTHENTICATED — Claude account connected via OAuth"
                                                    </span>
                                                </div>
                                            </Show>
                                        </div>
                                    </Show>
                                })}
                                // #216c: device-code OAuth panel (qwen/minimax + oauth_device mode)
                                {(pid_device3 == "qwen" || pid_device3 == "minimax").then(|| {
                                    let pid_sv = StoredValue::new(pid_device4.clone());
                                    let device_msg = StoredValue::new(format!("Device-code sign-in: a code and URL appear here; open the URL, enter the code, and the gateway completes the {} flow server-side.", pid_device4));
                                    view! {
                                    <Show when=move || config.get().auth_types.get(&pid_sv.get_value()).map(|t| t == "oauth_device").unwrap_or(false)>
                                        <div style="margin-bottom: 10px; padding: 12px; background: rgba(255,60,20,0.04); border: 1px solid rgba(255,60,20,0.15); border-radius: 10px;">
                                            <Show when=move || device_flow.get() == "idle" || device_flow.get().starts_with("error")>
                                                <div style="font-size: 10px; color: rgba(255,245,240,0.55); margin-bottom: 8px; font-family: 'Orbitron', monospace; letter-spacing: 0.5px; line-height: 1.6;">
                                                    {move || device_msg.get_value()}
                                                </div>
                                                <Show when=move || device_flow.get().starts_with("error")>
                                                    <div style="font-size: 10px; color: rgba(255,120,80,0.9); margin-bottom: 8px; font-family: 'Orbitron', monospace;">
                                                        {move || device_flow.get()}
                                                    </div>
                                                </Show>
                                                <button
                                                    style="padding: 10px 18px; border-radius: 8px; font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 2px; cursor: pointer; border: 1px solid rgba(255,60,20,0.4); background: rgba(255,60,20,0.15); color: rgba(255,245,240,0.95); transition: all 0.3s;"
                                                    on:click={
                                                        move |_| {
                                                        let pid = pid_sv.get_value();
                                                        device_flow.set("starting".to_string());
                                                        device_provider.set(pid.clone());
                                                        let my_gen = device_poll_gen.get_untracked() + 1;
                                                        device_poll_gen.set(my_gen);
                                                        spawn_local(async move {
                                                            // 1) Start the device-code flow
                                                            let body = serde_json::json!({ "provider": pid });
                                                            let started: Result<serde_json::Value, _> = crate::api::post_json("/v1/auth/device/start", &body).await;
                                                            let start = match started {
                                                                Ok(v) => v,
                                                                Err(e) => { device_flow.set(format!("error: {}", e)); return; }
                                                            };
                                                            let session = start.get("session_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                                            let user_code = start.get("user_code").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                                            // Prefer the complete URI (embeds the code) when present
                                                            let uri = start.get("verification_uri_complete")
                                                                .and_then(|v| v.as_str())
                                                                .filter(|s| !s.is_empty())
                                                                .or_else(|| start.get("verification_uri").and_then(|v| v.as_str()))
                                                                .unwrap_or("").to_string();
                                                            if session.is_empty() || uri.is_empty() {
                                                                device_flow.set("error: malformed response from /v1/auth/device/start".to_string());
                                                                return;
                                                            }
                                                            device_flow.set(format!("waiting:{}|{}", user_code, uri));
                                                            // 2) Poll every 3s, up to 5 minutes (server polls upstream itself)
                                                            for _ in 0..100 {
                                                                gloo_timers::future::TimeoutFuture::new(3_000).await;
                                                                if device_poll_gen.get_untracked() != my_gen
                                                                    || !device_flow.get_untracked().starts_with("waiting") {
                                                                    return; // superseded or cancelled
                                                                }
                                                                let url = format!("/v1/auth/device/poll?session={}", session);
                                                                if let Ok(st) = crate::api::fetch_json::<serde_json::Value>(&url).await {
                                                                    match st.get("status").and_then(|v| v.as_str()) {
                                                                        Some("complete") => {
                                                                            let pid_done = device_provider.get_untracked();
                                                                            config.update(|c| {
                                                                                c.auth_types.insert(pid_done.clone(), "oauth_device".to_string());
                                                                                if !c.providers.contains(&pid_done) {
                                                                                    c.providers.clear();
                                                                                    c.providers.push(pid_done.clone());
                                                                                }
                                                                            });
                                                                            key_status.update(|m| { m.insert(pid_done, KeyTestStatus::Valid); });
                                                                            device_flow.set("done".to_string());
                                                                            return;
                                                                        }
                                                                        Some("error") => {
                                                                            let msg = st.get("error").and_then(|v| v.as_str()).unwrap_or("device flow failed");
                                                                            device_flow.set(format!("error: {}", msg));
                                                                            return;
                                                                        }
                                                                        _ => {} // pending — keep polling
                                                                    }
                                                                }
                                                            }
                                                            if device_flow.get_untracked().starts_with("waiting") {
                                                                device_flow.set("error: timed out after 5 minutes — try again".to_string());
                                                            }
                                                        });
                                                    }}
                                                >"⚡ START DEVICE LOGIN"</button>
                                            </Show>
                                            <Show when=move || device_flow.get() == "starting">
                                                <div style="display: flex; align-items: center; gap: 10px;">
                                                    <div style="width: 10px; height: 10px; border-radius: 50%; background: rgba(255,180,0,0.9); animation: pulse 1.2s infinite;" />
                                                    <span style="font-size: 11px; color: rgba(255,210,80,0.9); font-family: 'Orbitron', monospace; letter-spacing: 1px;">
                                                        "REQUESTING DEVICE CODE…"
                                                    </span>
                                                </div>
                                            </Show>
                                            <Show when=move || device_flow.get().starts_with("waiting")>
                                                {move || {
                                                    let s = device_flow.get();
                                                    let rest = s.strip_prefix("waiting:").unwrap_or("");
                                                    let (code, uri) = rest.split_once('|').unwrap_or(("", ""));
                                                    let code = code.to_string();
                                                    let uri = uri.to_string();
                                                    let uri_href = uri.clone();
                                                    view! {
                                                        <div>
                                                            <div style="display: flex; align-items: center; gap: 12px; margin-bottom: 10px;">
                                                                <span style="font-family: 'Orbitron', monospace; font-size: 20px; font-weight: 700; letter-spacing: 4px; color: rgba(255,210,80,1); background: rgba(255,180,0,0.08); border: 1px solid rgba(255,180,0,0.35); border-radius: 8px; padding: 8px 14px;">
                                                                    {code}
                                                                </span>
                                                                <a href=uri_href target="_blank" rel="noopener"
                                                                   style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 1px; color: rgba(255,60,20,1); text-decoration: underline; word-break: break-all;">
                                                                    {uri}
                                                                </a>
                                                            </div>
                                                            <div style="display: flex; align-items: center; gap: 10px;">
                                                                <div style="width: 10px; height: 10px; border-radius: 50%; background: rgba(255,180,0,0.9); animation: pulse 1.2s infinite;" />
                                                                <span style="font-size: 11px; color: rgba(255,210,80,0.9); font-family: 'Orbitron', monospace; letter-spacing: 1px;">
                                                                    "WAITING FOR AUTHORIZATION — open the link and enter the code"
                                                                </span>
                                                                <button
                                                                    style="margin-left: auto; padding: 3px 8px; border-radius: 4px; font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 1px; cursor: pointer; border: 1px solid rgba(255,60,20,0.2); background: transparent; color: rgba(255,245,240,0.5);"
                                                                    on:click=move |_| {
                                                                        device_poll_gen.set(device_poll_gen.get_untracked() + 1);
                                                                        device_flow.set("idle".to_string());
                                                                    }
                                                                >"CANCEL"</button>
                                                            </div>
                                                        </div>
                                                    }
                                                }}
                                            </Show>
                                            <Show when=move || device_flow.get() == "done">
                                                <div style="display: flex; align-items: center; gap: 10px;">
                                                    <span style="font-size: 13px;">"✅"</span>
                                                    <span style="font-size: 11px; color: rgba(100,255,100,0.9); font-family: 'Orbitron', monospace; letter-spacing: 1px;">
                                                        "AUTHENTICATED — device-code credential stored"
                                                    </span>
                                                </div>
                                            </Show>
                                        </div>
                                    </Show>
                                    }
                                })}
                                // #216c: Gemini CLI creds detect → import (google-gemini-cli)
                                {(pid_gemini1 == "google-gemini-cli").then(|| view! {
                                    <div style="margin-bottom: 10px; padding: 12px; background: rgba(15,157,88,0.05); border: 1px solid rgba(15,157,88,0.25); border-radius: 10px;">
                                        <Show when=move || gemini_cli.get() == "unknown">
                                            <button
                                                style="padding: 8px 14px; border-radius: 8px; font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; cursor: pointer; border: 1px solid rgba(15,157,88,0.5); background: rgba(15,157,88,0.15); color: rgba(255,245,240,0.95); transition: all 0.3s;"
                                                on:click=move |_| {
                                                    spawn_local(async move {
                                                        match crate::api::fetch_json::<serde_json::Value>("/v1/auth/cli-creds").await {
                                                            Ok(v) => {
                                                                let found = v.get("gemini").and_then(|g| g.get("found")).and_then(|f| f.as_bool()).unwrap_or(false);
                                                                gemini_cli.set(if found { "found".to_string() } else { "not_found".to_string() });
                                                            }
                                                            Err(e) => gemini_cli.set(format!("error: {}", e)),
                                                        }
                                                    });
                                                }
                                            >"🔍 DETECT GEMINI CLI CREDENTIALS"</button>
                                        </Show>
                                        <Show when=move || gemini_cli.get() == "found">
                                            <div style="display: flex; align-items: center; gap: 10px;">
                                                <span style="font-size: 11px; color: rgba(100,255,140,0.9); font-family: 'Orbitron', monospace; letter-spacing: 1px;">
                                                    "GEMINI CLI CREDENTIALS FOUND ON THIS MACHINE"
                                                </span>
                                                <button
                                                    style="margin-left: auto; padding: 8px 14px; border-radius: 8px; font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; cursor: pointer; border: 1px solid rgba(15,157,88,0.5); background: rgba(15,157,88,0.2); color: rgba(255,245,240,0.95);"
                                                    on:click=move |_| {
                                                        spawn_local(async move {
                                                            let body = serde_json::json!({});
                                                            match crate::api::post_json::<serde_json::Value, serde_json::Value>("/v1/auth/cli-creds/import", &body).await {
                                                                Ok(v) if v.get("imported").and_then(|i| i.as_bool()).unwrap_or(false) => {
                                                                    config.update(|c| {
                                                                        c.auth_types.insert("google-gemini-cli".to_string(), "oauth_device".to_string());
                                                                        if !c.providers.contains(&"google-gemini-cli".to_string()) {
                                                                            c.providers.clear();
                                                                            c.providers.push("google-gemini-cli".to_string());
                                                                        }
                                                                    });
                                                                    key_status.update(|m| { m.insert("google-gemini-cli".to_string(), KeyTestStatus::Valid); });
                                                                    gemini_cli.set("imported".to_string());
                                                                }
                                                                Ok(_) => gemini_cli.set("error: import returned unexpected response".to_string()),
                                                                Err(e) => gemini_cli.set(format!("error: {}", e)),
                                                            }
                                                        });
                                                    }
                                                >"⚡ IMPORT"</button>
                                            </div>
                                        </Show>
                                        <Show when=move || gemini_cli.get() == "not_found">
                                            <div style="font-size: 10px; color: rgba(255,210,80,0.9); font-family: 'Orbitron', monospace; letter-spacing: 0.5px; line-height: 1.6;">
                                                "No Gemini CLI credentials found. Run `gemini` once to sign in, then re-detect — or enter an API key below."
                                            </div>
                                            <button
                                                style="margin-top: 8px; padding: 4px 10px; border-radius: 6px; font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 1px; cursor: pointer; border: 1px solid rgba(15,157,88,0.4); background: transparent; color: rgba(255,245,240,0.6);"
                                                on:click=move |_| gemini_cli.set("unknown".to_string())
                                            >"RE-DETECT"</button>
                                        </Show>
                                        <Show when=move || gemini_cli.get() == "imported">
                                            <div style="display: flex; align-items: center; gap: 10px;">
                                                <span style="font-size: 13px;">"✅"</span>
                                                <span style="font-size: 11px; color: rgba(100,255,100,0.9); font-family: 'Orbitron', monospace; letter-spacing: 1px;">
                                                    "IMPORTED — Gemini CLI credential stored in the gateway"
                                                </span>
                                            </div>
                                        </Show>
                                        <Show when=move || gemini_cli.get().starts_with("error")>
                                            <div style="font-size: 10px; color: rgba(255,120,80,0.9); font-family: 'Orbitron', monospace;">
                                                {move || gemini_cli.get()}
                                            </div>
                                            <button
                                                style="margin-top: 8px; padding: 4px 10px; border-radius: 6px; font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 1px; cursor: pointer; border: 1px solid rgba(15,157,88,0.4); background: transparent; color: rgba(255,245,240,0.6);"
                                                on:click=move |_| gemini_cli.set("unknown".to_string())
                                            >"RETRY"</button>
                                        </Show>
                                    </div>
                                })}
                                // Hint for OAuth tokens
                                <Show when=move || config.get().auth_types.get(&pid_auth5).cloned().unwrap_or_else(|| "api_key".to_string()) == "oauth_token">
                                    <div style="font-size: 10px; color: rgba(255,245,240,0.5); margin-bottom: 6px; font-family: 'Orbitron', monospace; letter-spacing: 0.5px;">
                                        "Run `claude setup-token` to generate an OAuth token (starts with sk-ant-oat01-)"
                                    </div>
                                </Show>
                                // Warning for AWS Bedrock multi-credential requirement
                                // Note: multi-field input for Bedrock would be ideal but warning is sufficient
                                //   AWS_ACCESS_KEY_ID, AWS_SECRET_ACCESS_KEY, and AWS_DEFAULT_REGION.
                                {(pid_bedrock == "bedrock").then(|| view! {
                                    <div style="margin-bottom: 10px; padding: 10px 12px; background: rgba(255,180,0,0.08); border: 1px solid rgba(255,180,0,0.35); border-radius: 8px; font-family: 'Orbitron', monospace; font-size: 10px; color: rgba(255,210,80,0.9); letter-spacing: 0.4px; line-height: 1.6;">
                                        <span style="font-size: 13px; vertical-align: middle; margin-right: 6px;">"⚠️"</span>
                                        "AWS Bedrock requires three credentials: Access Key ID, Secret Access Key, and Default Region. "
                                        "Enter your Access Key ID here. Then configure AWS_SECRET_ACCESS_KEY and AWS_DEFAULT_REGION manually in config.toml under [credentials] after setup."
                                    </div>
                                })}
                                <div style=move || format!("display: {}; align-items: center; gap: 8px;",
                                    // #216b: hide manual key input while browser-OAuth mode is active
                                    // #216c: same for device-code mode (qwen/minimax)
                                    if (pid_browser4 == "anthropic" && config.get().auth_types.get("anthropic").map(|t| t == "oauth_browser").unwrap_or(false))
                                        || config.get().auth_types.get(&pid_browser4).map(|t| t == "oauth_device").unwrap_or(false) { "none" } else { "flex" })>
                                    <input
                                        style="flex: 1; padding: 12px 16px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 10px; color: rgba(255,245,240,0.9); font-family: 'Orbitron', monospace; font-size: 12px; outline: none; box-sizing: border-box; transition: all 0.3s;"
                                        type="password"
                                        placeholder=move || {
                                            let auth = config.get().auth_types.get(&pid_c12).cloned().unwrap_or_else(|| "api_key".to_string());
                                            if auth == "oauth_token" || auth == "oauth_device" || auth == "oauth_browser" {
                                                format!("{} — OAuth (click button above)", name_c2)
                                            } else {
                                                match pid_c12.as_str() {
                                                    "anthropic" => format!("{} — sk-ant-api03-...", name_c2),
                                                    "openai" => format!("{} — sk-...", name_c2),
                                                    "google" => format!("{} — AIza...", name_c2),
                                                    "minimax" => format!("{} — sk-api-...", name_c2),
                                                    "openrouter" => format!("{} — sk-or-...", name_c2),
                                                    "xai" => format!("{} — xai-...", name_c2),
                                                    "sakana" => format!("{} — fish_...", name_c2),
                                                    _ => format!("{} — API key", name_c2),
                                                }
                                            }
                                        }
                                        prop:value=move || config.get().api_keys.get(&pid_c6).cloned().unwrap_or_default()
                                        on:input=move |ev| {
                                            let key = event_target_value(&ev);
                                            let pid = pid_c7.clone();
                                            // Auto-detect token type from prefix
                                            if key.starts_with("sk-ant-oat01-") {
                                                config.update(|c| {
                                                    c.auth_types.insert(pid.clone(), "oauth_token".to_string());
                                                    c.api_keys.insert(pid, key);
                                                });
                                            } else if key.starts_with("sk-ant-api") || key.starts_with("sk-") {
                                                config.update(|c| {
                                                    c.auth_types.insert(pid.clone(), "api_key".to_string());
                                                    c.api_keys.insert(pid, key);
                                                });
                                            } else {
                                                config.update(|c| { c.api_keys.insert(pid, key); });
                                            }
                                        }
                                        on:blur=move |_| {
                                            let pid = pid_c10.clone();
                                            let key = config.get_untracked().api_keys.get(&pid).cloned().unwrap_or_default();
                                            if key.trim().is_empty() {
                                                key_status.update(|m| { m.remove(&pid); });
                                                return;
                                            }
                                            let validatable = ["anthropic", "openai"];
                                            if !validatable.contains(&pid.as_str()) {
                                                key_status.update(|m| { m.insert(pid, KeyTestStatus::InfoOnly); });
                                                return;
                                            }
                                            key_status.update(|m| { m.insert(pid.clone(), KeyTestStatus::Testing); });
                                            let pid2 = pid.clone();
                                            spawn_local(async move {
                                                match api::test_provider_connection(&pid2, Some(&key), None).await {
                                                    Ok(r) if r.success => {
                                                        // Inject fetched models into providers_data so StepModel has them
                                                        if !r.models.is_empty() {
                                                            let fetched_models = r.models.clone();
                                                            let pid_for_models = pid2.clone();
                                                            providers_data.update(|provs| {
                                                                if let Some(p) = provs.iter_mut().find(|p| p.id == pid_for_models) {
                                                                    p.models = fetched_models;
                                                                }
                                                            });
                                                        }
                                                        key_status.update(|m| { m.insert(pid2, KeyTestStatus::Valid); });
                                                    }
                                                    Ok(r) => {
                                                        let msg = if r.error.is_empty() { r.status } else { r.error };
                                                        key_status.update(|m| { m.insert(pid2, KeyTestStatus::Invalid(msg)); });
                                                    }
                                                    Err(e) => {
                                                        key_status.update(|m| { m.insert(pid2, KeyTestStatus::Invalid(e)); });
                                                    }
                                                }
                                            });
                                        }
                                    />
                                    // Validation indicator
                                    <div style="width: 24px; flex-shrink: 0; text-align: center;">
                                        {move || {
                                            let status = key_status.get().get(&pid_c11).cloned().unwrap_or(KeyTestStatus::Untested);
                                            match status {
                                                KeyTestStatus::Untested => view! { <span></span> }.into_any(),
                                                KeyTestStatus::Testing => view! { <span style="color: rgba(255,245,240,0.7); font-size: 12px;">"..."</span> }.into_any(),
                                                KeyTestStatus::Valid => view! { <span style="color: #22c55e; font-size: 14px;" title="Valid key">"OK"</span> }.into_any(),
                                                KeyTestStatus::Invalid(ref msg) => view! { <span style="color: #ef4444; font-size: 11px; cursor: help;" title={msg.clone()}>"FAIL"</span> }.into_any(),
                                                KeyTestStatus::InfoOnly => view! { <span style="color: rgba(255,245,240,0.3); font-size: 9px;" title="Key saved">"SAVED"</span> }.into_any(),
                                            }
                                        }}
                                    </div>
                                </div>
                            </div>
                            // URL input for local providers (Ollama)
                            <div
                                on:click=|ev: web_sys::MouseEvent| ev.stop_propagation()
                                style=move || format!(
                                "margin-top: 14px; padding-top: 14px; border-top: 1px solid rgba(255,60,20,0.1); display: {};",
                                if config.get().providers.contains(&pid_c9) && is_local { "block" } else { "none" }
                            )>
                                <label class="font-orbitron text-[10px] tracking-[3px] text-white/70 block mb-2 uppercase">"OLLAMA ENDPOINT URL"</label>
                                <div style="display: flex; align-items: center; gap: 8px;">
                                    <input
                                        style="flex: 1; padding: 12px 16px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 10px; color: rgba(255,245,240,0.9); font-family: 'Orbitron', monospace; font-size: 12px; outline: none; box-sizing: border-box; transition: all 0.3s;"
                                        type="text"
                                        placeholder="http://localhost:11434"
                                        prop:value=move || config.get().ollama_url.clone()
                                        on:input=move |ev| {
                                            let url = event_target_value(&ev);
                                            config.update(|c| { c.ollama_url = url; });
                                        }
                                        on:blur=move |_| {
                                            let url = config.get_untracked().ollama_url.clone();
                                            if url.trim().is_empty() {
                                                key_status.update(|m| { m.remove("ollama"); });
                                                return;
                                            }
                                            key_status.update(|m| { m.insert("ollama".to_string(), KeyTestStatus::Testing); });
                                            spawn_local(async move {
                                                match api::test_provider_connection("ollama", None, Some(&url)).await {
                                                    Ok(r) if r.success => {
                                                        let info = if r.models.is_empty() {
                                                            "Connected".to_string()
                                                        } else {
                                                            format!("{} models", r.models.len())
                                                        };
                                                        key_status.update(|m| { m.insert("ollama".to_string(), KeyTestStatus::Valid); });
                                                        web_sys::console::log_1(&format!("Ollama: {}", info).into());
                                                    }
                                                    Ok(r) => {
                                                        let msg = if r.error.is_empty() { format!("Cannot reach Ollama at {}", url) } else { r.error };
                                                        key_status.update(|m| { m.insert("ollama".to_string(), KeyTestStatus::Invalid(msg)); });
                                                    }
                                                    Err(e) => {
                                                        key_status.update(|m| { m.insert("ollama".to_string(), KeyTestStatus::Invalid(e)); });
                                                    }
                                                }
                                            });
                                        }
                                    />
                                    <div style="width: 24px; flex-shrink: 0; text-align: center;">
                                        {move || {
                                            let status = key_status.get().get("ollama").cloned().unwrap_or(KeyTestStatus::Untested);
                                            match status {
                                                KeyTestStatus::Untested => view! { <span></span> }.into_any(),
                                                KeyTestStatus::Testing => view! { <span style="color: rgba(255,245,240,0.7); font-size: 12px;">"..."</span> }.into_any(),
                                                KeyTestStatus::Valid => view! { <span style="color: #22c55e; font-size: 14px;" title="Connected">"OK"</span> }.into_any(),
                                                KeyTestStatus::Invalid(ref msg) => view! { <span style="color: #ef4444; font-size: 11px; cursor: help;" title={msg.clone()}>"FAIL"</span> }.into_any(),
                                                _ => view! { <span></span> }.into_any(),
                                            }
                                        }}
                                    </div>
                                </div>
                            </div>
                        </div>
                    }
                }).collect::<Vec<_>>()}
            </div>

            // Counter
            <div style="margin-top: 14px; padding: 14px 18px; background: rgba(255,255,255,0.03); border-radius: 12px; border: 1px solid rgba(255,60,20,0.1); display: flex; align-items: center; gap: 14px; transition: all 0.3s ease;">
                <div style="font-family: 'Orbitron', monospace; font-size: 10px; color: rgba(255,60,20,1); letter-spacing: 3px; font-weight: 700;">
                    {move || config.get().providers.len().to_string()}
                </div>
                <span style="font-size: 13px; color: rgba(255,245,240,0.7);">
                    {move || { let n = config.get().providers.len(); format!("provider{} selected — keys stored locally in ~/.zeus/config.toml", if n != 1 { "s" } else { "" }) }}
                </span>
            </div>
        </div>
    }
}

// ─── STEP 3: MODEL SELECT ───────────────────────────────

#[component]
fn StepModel(config: RwSignal<OnboardConfig>, providers_data: RwSignal<Vec<DynProvider>>) -> impl IntoView {
    // If Anthropic is selected but no models loaded yet, fetch directly from Anthropic API
    // using the key the user just entered — bypasses the gateway entirely.
    leptos::prelude::Effect::new(move |_| {
        let cfg = config.get();
        let has_anthropic = cfg.providers.contains(&"anthropic".to_string());
        let api_key = cfg.api_keys.get("anthropic").cloned().unwrap_or_default();
        if !has_anthropic || api_key.is_empty() { return; }

        let already_loaded = providers_data.get()
            .iter()
            .find(|p| p.id == "anthropic")
            .map(|p| !p.models.is_empty())
            .unwrap_or(false);
        if already_loaded { return; }

        leptos::task::spawn_local(async move {
            use gloo_net::http::Request;
            let result = Request::get("https://api.anthropic.com/v1/models")
                .header("x-api-key", &api_key)
                .header("anthropic-version", "2023-06-01")
                .send()
                .await;
            if let Ok(resp) = result {
                if resp.ok() {
                    if let Ok(body) = resp.json::<serde_json::Value>().await {
                        let models: Vec<String> = body["data"]
                            .as_array()
                            .unwrap_or(&vec![])
                            .iter()
                            .filter_map(|m| m["id"].as_str().map(|s| s.to_string()))
                            .collect();
                        if !models.is_empty() {
                            providers_data.update(|provs| {
                                if let Some(p) = provs.iter_mut().find(|p| p.id == "anthropic") {
                                    p.models = models;
                                }
                            });
                        }
                    }
                }
            }
        });
    });

    view! {
        <div class="animate-fade-in">
            <div class="mb-5">
                <div class="font-orbitron text-[22px] font-semibold text-z-text">
                    <TypeWriter text="Select your default model" speed_ms=40 />
                </div>
                <p class="animate-fade-in" style="font-size: 13px; color: rgba(255,245,240,0.7); margin-top: 8px; line-height: 1.7;">
                    "This is the primary brain Zeus will use. Switch per-session or per-agent anytime via "
                    <code style="font-family: 'Orbitron', monospace; font-size: 11px; color: rgba(255,60,20,0.6); background: rgba(255,255,255,0.03); padding: 1px 6px; border-radius: 3px;">"/model"</code>
                </p>
            </div>

            <div class="animate-fade-in flex flex-col gap-1.5">
                {move || {
                    let enabled = config.get().providers.clone();
                    let provs = providers_data.get();
                    let models: Vec<(String, String, String, String)> = provs.iter()
                        .filter(|p| enabled.contains(&p.id))
                        .flat_map(|p| p.models.iter().map({
                            let pid = p.id.clone();
                            let pname = p.name.clone();
                            let pcolor = p.color.clone();
                            move |m| {
                                // Model ids from some sources already carry the provider
                                // prefix (e.g. the key-test injection) — don't double it.
                                let prefix = format!("{}/", pid);
                                let full = if m.starts_with(&prefix) { m.clone() } else { format!("{}{}", prefix, m) };
                                (full, pname.clone(), pcolor.clone(), pid.clone())
                            }
                        }))
                        .collect();

                    if models.is_empty() {
                        let manual_model = RwSignal::new(config.get().default_model.clone());
                        view! {
                            <div style="text-align: center; padding: 20px; color: rgba(255,245,240,0.7);">
                                <div style="font-size: 14px; margin-bottom: 8px;">
                                    "⚠ No models loaded — gateway may be offline"
                                </div>
                                <div style="font-size: 12px; color: rgba(255,245,240,0.5); margin-bottom: 16px;">
                                    "Enter your model manually (format: provider/model-name)"
                                </div>
                                <input
                                    type="text"
                                    placeholder="e.g. anthropic/claude-sonnet-4-6"
                                    prop:value=move || manual_model.get()
                                    on:input=move |e| {
                                        let val = leptos::prelude::event_target_value(&e);
                                        manual_model.set(val.clone());
                                        config.update(|c| c.default_model = val);
                                    }
                                    style="width: 100%; max-width: 400px; padding: 12px 16px; border-radius: 8px;\
                                        background: rgba(0,0,0,0.3); border: 1px solid rgba(255,60,20,0.15);\
                                        color: rgba(255,245,240,0.9); font-family: 'JetBrains Mono', monospace;\
                                        font-size: 13px; outline: none; text-align: center;"
                                />
                            </div>
                        }.into_any()
                    } else {
                        view! {
                            <div class="flex flex-col gap-1.5">
                                {models.into_iter().map(|(full, provider_name, color, _pid): (String, String, String, String)| {
                                    let full_c = full.clone();
                                    let full_c2 = full.clone();
                                    let full_c3 = full.clone();
                                    let full_c4 = full.clone();
                                    let color_c = color.clone();
                                    view! {
                                        <div
                                            class=move || if config.get().default_model == full_c { "zselect-card selected flex items-center gap-3.5" } else { "zselect-card flex items-center gap-3.5" }
                                            on:click={
                                                let full = full.clone();
                                                move |_| config.update(|c| c.default_model = full.clone())
                                            }
                                        >
                                            <div style={format!("width: 8px; height: 8px; border-radius: 50%; background: {}; box-shadow: 0 0 8px {}60; flex-shrink: 0;",  color_c, color_c)} />
                                            // Fix #5: model text color is dynamic
                                            <div style=move || format!("flex: 1; font-family: 'Orbitron', monospace; font-size: 12px; color: {}; letter-spacing: 1px;",
                                                if config.get().default_model == full_c3 { "rgba(255,245,240,0.9)" } else { "rgba(255,245,240,0.7)" })>
                                                {full_c2.clone()}
                                            </div>
                                            <span style="font-family: 'Orbitron', monospace; font-size: 9px; color: rgba(255,245,240,0.5); letter-spacing: 2px; font-weight: 700;">
                                                {provider_name.to_uppercase()}
                                            </span>
                                            <div style=move || format!(
                                                "width: 22px; height: 22px; border-radius: 11px; flex-shrink: 0; border: 2px solid {}; display: flex; align-items: center; justify-content: center; transition: all 0.3s;",
                                                if config.get().default_model == full_c4 { "rgba(255,60,20,1)" } else { "rgba(255,60,20,0.1)" }
                                            )>
                                                <Show when={
                                                    let full = full_c2.clone();
                                                    move || config.get().default_model == full
                                                }>
                                                    <div style="width: 10px; height: 10px; border-radius: 5px; background: rgba(255,60,20,1);" />
                                                </Show>
                                            </div>
                                        </div>
                                    }
                                }).collect::<Vec<_>>()}
                            </div>
                        }.into_any()
                    }
                }}
            </div>

            // Fallback models section
            <div style="margin-top: 28px; padding-top: 20px; border-top: 1px solid rgba(255,60,20,0.1);">
                <div class="font-orbitron text-[10px] tracking-[3px] text-z-accent">"FALLBACK MODELS"</div>
                <p style="font-size: 11px; color: rgba(255,245,240,0.5); margin-bottom: 12px; line-height: 1.5;">
                    "If the primary model fails, Zeus tries these in order. Format: " <code style="color: rgba(255,60,20,0.6);">"provider/model-name"</code>
                </p>
                // Current fallback list
                <div style="display: flex; flex-direction: column; gap: 6px; margin-bottom: 10px;">
                    {move || config.get().fallback_models.iter().enumerate().map(|(i, m)| {
                        let model = m.clone();
                        let idx = i;
                        view! {
                            <div style="display: flex; align-items: center; gap: 8px; padding: 8px 12px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 8px;">
                                <span style="font-family: 'Orbitron', monospace; font-size: 10px; color: rgba(255,60,20,0.5); min-width: 16px;">{format!("#{}", idx + 1)}</span>
                                <span style="flex: 1; font-family: 'Orbitron', monospace; font-size: 12px; color: rgba(255,245,240,0.8);">{model}</span>
                                <button
                                    style="padding: 2px 8px; background: rgba(255,60,20,0.1); border: 1px solid rgba(255,60,20,0.2); border-radius: 4px; color: rgba(255,60,20,0.7); font-family: 'Orbitron', monospace; font-size: 9px; cursor: pointer;"
                                    on:click=move |_| config.update(|c| { c.fallback_models.remove(idx); })
                                >"✕"</button>
                            </div>
                        }
                    }).collect::<Vec<_>>()}
                </div>
                // Add fallback input
                {
                    let fb_input = RwSignal::new(String::new());
                    view! {
                        <div style="display: flex; gap: 8px;">
                            <input
                                style="flex: 1; padding: 10px 14px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 8px; color: rgba(255,245,240,0.9); font-family: 'Orbitron', monospace; font-size: 12px; outline: none; box-sizing: border-box;"
                                placeholder="e.g. openai/gpt-4o or ollama/llama3.2"
                                prop:value=move || fb_input.get()
                                on:input=move |ev| fb_input.set(event_target_value(&ev))
                                on:keydown=move |ev: web_sys::KeyboardEvent| {
                                    if ev.key() == "Enter" {
                                        let val = fb_input.get();
                                        if !val.trim().is_empty() {
                                            config.update(|c| c.fallback_models.push(val.trim().to_string()));
                                            fb_input.set(String::new());
                                        }
                                    }
                                }
                            />
                            <button
                                style="padding: 10px 16px; background: rgba(255,60,20,0.15); border: 1px solid rgba(255,60,20,0.3); border-radius: 8px; color: rgba(255,60,20,1); font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 1px; cursor: pointer; white-space: nowrap;"
                                on:click=move |_| {
                                    let val = fb_input.get();
                                    if !val.trim().is_empty() {
                                        config.update(|c| c.fallback_models.push(val.trim().to_string()));
                                        fb_input.set(String::new());
                                    }
                                }
                            >"+ ADD"</button>
                        </div>
                    }
                }
            </div>
        </div>
    }
}

// ─── STEP 4: CHANNELS ───────────────────────────────────

#[component]
fn StepChannels(config: RwSignal<OnboardConfig>, channels_data: RwSignal<Vec<DynChannel>>) -> impl IntoView {
    // QR pairing state — stable signals outside reactive closures
    let signal_qr_uri: RwSignal<Option<String>> = RwSignal::new(None);
    let signal_qr_busy: RwSignal<bool> = RwSignal::new(false);
    let wa_qr: RwSignal<Option<String>> = RwSignal::new(None);
    let wa_qr_busy: RwSignal<bool> = RwSignal::new(false);
    view! {
        <div class="animate-fade-in">
            <div class="mb-5">
                <div class="font-orbitron text-[22px] font-semibold text-z-text">
                    <TypeWriter text="Extend your senses" speed_ms=40 />
                </div>
                <p class="text-sm text-white/60 leading-relaxed">
                    "Zeus can communicate across 8 messaging platforms simultaneously. Enable the ones you want now."
                </p>
            </div>

            <div class="grid grid-cols-2 gap-2.5">
                {channels_data.get().iter().map(|ch| {
                    let cid = ch.id.clone();
                    let cid_c1 = cid.clone();
                    let cid_c2 = cid.clone();
                    let cid_c3 = cid.clone();
                    let cid_c4 = cid.clone();
                    let cid_c5 = cid.clone();
                    let cid_c6 = cid.clone();
                    let icon_text = ch.icon.clone();
                    let name = ch.name.clone();
                    let desc = ch.desc.clone();

                    view! {
                        <div
                            style=move || {
                                let active = config.get().channels.contains(&cid_c1);
                                format!("padding: 20px; border-radius: 12px; cursor: pointer; transition: all 0.3s ease; background: rgba(255,255,255,0.03); border: 1px solid {}; box-shadow: {};",
                                    if active { "rgba(255,60,20,0.25)" } else { "rgba(255,60,20,0.1)" },
                                    if active { "0 0 30px rgba(255,60,20,0.15)" } else { "none" })
                            }
                            on:click={
                                let cid = cid.clone();
                                move |_| config.update(|c| {
                                    if c.channels.contains(&cid) {
                                        c.channels.retain(|x| x != &cid);
                                    } else {
                                        c.channels.push(cid.clone());
                                    }
                                })
                            }
                        >
                            <div class="flex items-center gap-3.5">
                                <div style=move || format!(
                                    "width: 46px; height: 46px; border-radius: 12px; flex-shrink: 0; display: flex; align-items: center; justify-content: center; font-family: 'Orbitron', monospace; font-size: 14px; font-weight: 900; transition: all 0.3s; border: 1px solid {}; background: {}; color: {};",
                                    if config.get().channels.contains(&cid_c2) { "rgba(255,60,20,0.25)" } else { "rgba(255,60,20,0.1)" },
                                    if config.get().channels.contains(&cid_c2) { "rgba(255,60,20,0.12)" } else { "rgba(255,255,255,0.03)" },
                                    if config.get().channels.contains(&cid_c2) { "rgba(255,60,20,1)" } else { "rgba(255,245,240,0.5)"
                                    }
                                )>
                                    {icon_text}
                                </div>
                                <div class="flex-1">
                                    // Fix #6: channel name color is dynamic
                                    <div style=move || format!("font-size: 15px; font-weight: 700; color: {};",
                                        if config.get().channels.contains(&cid_c5) { "rgba(255,245,240,0.9)" } else { "rgba(255,245,240,0.7)" })>
                                        {name}
                                    </div>
                                    <div style=move || format!("font-size: 12px; color: {}; margin-top: 2px;",
                                        if config.get().channels.contains(&cid_c6) { "rgba(255,245,240,0.7)" } else { "rgba(255,245,240,0.5)" })>{desc}</div>
                                </div>
                                <div style=move || format!(
                                    "width: 22px; height: 22px; border-radius: 6px; flex-shrink: 0; border: 1.5px solid {}; background: {}; display: flex; align-items: center; justify-content: center; transition: all 0.3s;",
                                    if config.get().channels.contains(&cid_c3) { "rgba(255,60,20,1)" } else { "rgba(255,60,20,0.1)" },
                                    if config.get().channels.contains(&cid_c3) { "rgba(255,60,20,0.2)" } else { "transparent" }
                                )>
                                    <Show when=move || config.get().channels.contains(&cid_c4)>
                                        <div style="width: 10px; height: 10px; border-radius: 3px; background: rgba(255,60,20,1);" />
                                    </Show>
                                </div>
                            </div>
                        </div>
                    }
                }).collect::<Vec<_>>()}
            </div>

            // Channel credential fields (T4) — expand when channel is enabled
            {move || {
                let cfg = config.get();
                let enabled: Vec<String> = cfg.channels.clone();
                if enabled.is_empty() { return view! { <div /> }.into_any(); }

                view! {
                    <div style="margin-top: 24px; border-top: 1px solid rgba(255,60,20,0.1); padding-top: 20px;">
                        <div style="font-family: 'Orbitron', monospace; font-size: 11px; letter-spacing: 2px; color: rgba(255,245,240,0.7); margin-bottom: 12px;">
                            "CHANNEL CREDENTIALS"
                        </div>
                        {enabled.into_iter().filter_map(|ch_id| {
                            let fields: Vec<(&str, &str, bool)> = match ch_id.as_str() {
                                "discord" => vec![
                                    ("token", "Bot Token", true),
                                    ("bot_name", "Bot Name", false),
                                    ("guild_id", "Guild (Server) ID", false),
                                    ("channel_id", "Channel ID", false),
                                    ("personality", "Personality (one-liner)", false),
                                ],
                                "telegram" => vec![
                                    ("bot_token", "Bot Token", true),
                                    ("chat_id", "Chat ID", false),
                                ],
                                "slack" => vec![
                                    ("bot_token", "Bot Token (xoxb-...)", true),
                                    ("app_token", "App Token (xapp-...)", true),
                                ],
                                "email" => vec![
                                    ("smtp_host", "SMTP Host", false),
                                    ("imap_host", "IMAP Host", false),
                                    ("username", "Email Address", false),
                                    ("password", "Password", true),
                                ],
                                "whatsapp" => vec![("token", "Cloud API Token", true), ("phone_id", "Phone Number ID", false)],
                                "matrix" => vec![("homeserver", "Homeserver URL", false), ("username", "Username", false), ("password", "Password", true)],
                                "signal" => vec![("phone", "Phone Number", false)],
                                "x_twitter" => vec![
                                    ("bearer_token", "Bearer Token", true),
                                    ("api_key", "API Key", true),
                                    ("api_secret", "API Secret", true),
                                    ("access_token", "Access Token", true),
                                    ("access_token_secret", "Access Token Secret", true),
                                ],
                                "mqtt" => vec![
                                    ("broker_url", "Broker URL (e.g. mqtt://localhost)", false),
                                    ("port", "Port (default: 1883)", false),
                                    ("topic_prefix", "Topic Prefix (e.g. zeus/)", false),
                                    ("client_id", "Client ID (optional)", false),
                                    ("username", "Username (optional)", false),
                                    ("password", "Password (optional)", true),
                                ],
                                "mattermost" => vec![
                                    ("server_url", "Server URL", false),
                                    ("token", "Bot Token", true),
                                    ("team_id", "Team ID (optional)", false),
                                ],
                                "irc" => vec![
                                    ("server", "Server (e.g. irc.libera.chat)", false),
                                    ("port", "Port (default: 6667)", false),
                                    ("nick", "Nickname", false),
                                    ("channels", "Channels (comma-separated, e.g. #zeus,#dev)", false),
                                    ("use_tls", "Use TLS? (Y/N, default: N)", false),
                                    ("nickserv_password", "NickServ Password (optional)", true),
                                ],
                                "twitter" => vec![
                                    ("api_key", "API Key (Consumer Key)", true),
                                    ("api_secret", "API Secret (Consumer Secret)", true),
                                    ("access_token", "Access Token", true),
                                    ("access_secret", "Access Token Secret", true),
                                    ("bearer_token", "Bearer Token (for v2 read)", true),
                                ],
                                "twitch" => vec![
                                    ("oauth_token", "OAuth Token", true),
                                    ("username", "Bot Username", false),
                                    ("channels", "Channels (comma-separated)", false),
                                    ("client_id", "Client ID (optional, for Helix API)", false),
                                ],
                                "imessage" => vec![
                                    ("_note", "macOS only — no credentials required. Zeus uses AppleScript bridge.", false),
                                ],
                                "pantheon" => vec![
                                    ("server", "Server (host:port)", false),
                                    ("channel_key", "Channel Key", false),
                                    ("nick", "Nick", false),
                                ],
                                _ => vec![],
                            };
                            if fields.is_empty() { return None; }
                            let ch_label = ch_id.clone();
                            let ch_id_outer = ch_id.clone();
                            Some(view! {
                                <div style="margin-bottom: 16px; padding: 14px; background: rgba(255,255,255,0.02); border-radius: 8px; border: 1px solid rgba(255,60,20,0.08);">
                                    <div style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,60,20,1); margin-bottom: 10px; text-transform: uppercase;">{ch_label}</div>
                                    {fields.into_iter().map(|(field_id, label, is_secret)| {
                                        let ch = ch_id_outer.clone();
                                        let fid = field_id.to_string();
                                        let fid2 = fid.clone();
                                        let ch2 = ch.clone();
                                        view! {
                                            <div style="margin-bottom: 8px;">
                                                <div style="font-size: 10px; color: rgba(255,245,240,0.5); margin-bottom: 3px;">{label}</div>
                                                <input
                                                    type={if is_secret { "password" } else { "text" }}
                                                    placeholder=label
                                                    style="width: 100%; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.12); border-radius: 6px; padding: 8px 10px; color: rgba(255,245,240,0.9); font-family: 'Rajdhani', sans-serif; font-size: 13px; box-sizing: border-box; outline: none;"
                                                    prop:value=move || {
                                                        config.get().channel_creds
                                                            .get(&ch).and_then(|m| m.get(&fid))
                                                            .cloned().unwrap_or_default()
                                                    }
                                                    on:input=move |ev| {
                                                        let val = event_target_value(&ev);
                                                        config.update(|c| {
                                                            c.channel_creds.entry(ch2.clone()).or_default().insert(fid2.clone(), val);
                                                        });
                                                    }
                                                />
                                            </div>
                                        }
                                    }).collect::<Vec<_>>()}
                                    // Signal: generate link QR for secondary-device pairing
                                    {if ch_id_outer == "signal" {
                                        view! {
                                            <div style="margin-top:10px; padding-top:10px; border-top:1px solid rgba(255,60,20,0.08);">
                                                <div style="font-size:10px; color:rgba(255,245,240,0.5); margin-bottom:8px; letter-spacing:1px; font-family:'Orbitron',monospace;">
                                                    "LINK AS SECONDARY DEVICE"
                                                </div>
                                                <button
                                                    style="padding:7px 14px; background:rgba(255,60,20,0.12); border:1px solid rgba(255,60,20,0.3); border-radius:6px; color:rgba(255,60,20,1); font-family:'Rajdhani',sans-serif; font-size:13px; cursor:pointer; transition:all 0.2s;"
                                                    on:click=move |_| {
                                                        signal_qr_busy.set(true);
                                                        signal_qr_uri.set(None);
                                                        spawn_local(async move {
                                                            if let Ok(v) = crate::api::post_json::<serde_json::Value, serde_json::Value>(
                                                                "/v1/channels/signal/link-uri", &serde_json::json!({})
                                                            ).await {
                                                                if let Some(u) = v.get("uri").and_then(|x| x.as_str()) {
                                                                    signal_qr_uri.set(Some(u.to_string()));
                                                                }
                                                            }
                                                            signal_qr_busy.set(false);
                                                        });
                                                    }
                                                >
                                                    {move || if signal_qr_busy.get() { "Generating…" } else { "Generate QR" }}
                                                </button>
                                                {move || signal_qr_uri.get().map(|uri| view! {
                                                    <div style="margin-top:12px; display:flex; justify-content:center;">
                                                        <img
                                                            src={qr_img_src(&uri)}
                                                            style="width:200px; height:200px; border-radius:8px; border:1px solid rgba(255,60,20,0.2);"
                                                        />
                                                    </div>
                                                })}
                                            </div>
                                        }.into_any()
                                    } else {
                                        view! { <div /> }.into_any()
                                    }}
                                    // WhatsApp: Baileys bridge QR pairing (optional, alongside Cloud API)
                                    {if ch_id_outer == "whatsapp" {
                                        view! {
                                            <div style="margin-top:10px; padding-top:10px; border-top:1px solid rgba(255,60,20,0.08);">
                                                <div style="font-size:10px; color:rgba(255,245,240,0.5); margin-bottom:6px; letter-spacing:1px; font-family:'Orbitron',monospace;">
                                                    "BAILEYS BRIDGE — PAIR VIA QR (OPTIONAL)"
                                                </div>
                                                <div style="margin-bottom:8px;">
                                                    <input
                                                        type="text"
                                                        placeholder="Bridge URL (e.g. http://localhost:3001)"
                                                        style="width:100%; background:rgba(255,255,255,0.03); border:1px solid rgba(255,60,20,0.12); border-radius:6px; padding:8px 10px; color:rgba(255,245,240,0.9); font-family:'Rajdhani',sans-serif; font-size:13px; box-sizing:border-box; outline:none;"
                                                        prop:value=move || config.get().channel_creds
                                                            .get("whatsapp").and_then(|m| m.get("bridge_url"))
                                                            .cloned().unwrap_or_default()
                                                        on:input=move |ev| {
                                                            let val = event_target_value(&ev);
                                                            config.update(|c| {
                                                                c.channel_creds.entry("whatsapp".to_string())
                                                                    .or_default()
                                                                    .insert("bridge_url".to_string(), val);
                                                            });
                                                        }
                                                    />
                                                </div>
                                                <button
                                                    style="padding:7px 14px; background:rgba(255,60,20,0.12); border:1px solid rgba(255,60,20,0.3); border-radius:6px; color:rgba(255,60,20,1); font-family:'Rajdhani',sans-serif; font-size:13px; cursor:pointer; transition:all 0.2s;"
                                                    on:click=move |_| {
                                                        let bridge = config.get().channel_creds
                                                            .get("whatsapp").and_then(|m| m.get("bridge_url"))
                                                            .cloned().unwrap_or_default();
                                                        if bridge.is_empty() { return; }
                                                        wa_qr_busy.set(true);
                                                        wa_qr.set(None);
                                                        spawn_local(async move {
                                                            let url = format!("{}/qr", bridge.trim_end_matches('/'));
                                                            if let Ok(v) = crate::api::fetch_json::<serde_json::Value>(&url).await {
                                                                if let Some(q) = v.get("qr").and_then(|x| x.as_str()) {
                                                                    wa_qr.set(Some(q.to_string()));
                                                                }
                                                            }
                                                            wa_qr_busy.set(false);
                                                        });
                                                    }
                                                >
                                                    {move || if wa_qr_busy.get() { "Loading…" } else { "Pair via QR" }}
                                                </button>
                                                {move || wa_qr.get().map(|qr| view! {
                                                    <div style="margin-top:12px; display:flex; justify-content:center;">
                                                        <img
                                                            src={qr_img_src(&qr)}
                                                            style="width:200px; height:200px; border-radius:8px; border:1px solid rgba(255,60,20,0.2);"
                                                        />
                                                    </div>
                                                })}
                                            </div>
                                        }.into_any()
                                    } else {
                                        view! { <div /> }.into_any()
                                    }}
                                </div>
                            })
                        }).collect::<Vec<_>>()}
                    </div>
                }.into_any()
            }}

            // Bot Policy
            <div style="margin-top: 24px; border-top: 1px solid rgba(255,60,20,0.1); padding-top: 20px;">
                <div style="font-family: 'Orbitron', monospace; font-size: 11px; letter-spacing: 2px; color: rgba(255,245,240,0.7); margin-bottom: 12px;">
                    "BOT POLICY"
                </div>
                <div style="display: flex; flex-direction: column; gap: 8px;">
                    {[("mentions", "Mentions Only", "Respond to @mentions from other bots"),
                      ("on", "All Bots", "Respond to all bot messages"),
                      ("off", "Ignore Bots", "Ignore all bot messages")].iter().map(|(id, name, desc)| {
                        let id_s = id.to_string();
                        let id_c1 = id_s.clone();
                        let id_c2 = id_s.clone();
                        let id_c3 = id_s.clone();
                        let id_c4 = id_s.clone();
                        view! {
                            <div
                                style=move || {
                                    let sel = config.get().allow_bots_mode == id_c1;
                                    format!("padding: 16px 20px; border-radius: 12px; cursor: pointer; transition: all 0.3s ease; background: rgba(255,255,255,0.03); border: 1px solid {};",
                                        if sel { "rgba(255,60,20,0.25)" } else { "rgba(255,60,20,0.1)" })
                                }
                                on:click={
                                    let id_click = id_s.clone();
                                    move |_| config.update(|c| c.allow_bots_mode = id_click.clone())
                                }
                            >
                                <div style="display: flex; align-items: center; gap: 14px;">
                                    <div style=move || format!(
                                        "width: 22px; height: 22px; border-radius: 11px; flex-shrink: 0; border: 2px solid {}; display: flex; align-items: center; justify-content: center; transition: all 0.3s;",
                                        if config.get().allow_bots_mode == id_c2 { "rgba(255,60,20,1)" } else { "rgba(255,60,20,0.1)" }
                                    )>
                                        <Show when={
                                            let id_show = id_c3.clone();
                                            move || config.get().allow_bots_mode == id_show
                                        }>
                                            <div style="width: 10px; height: 10px; border-radius: 5px; background: rgba(255,60,20,1);" />
                                        </Show>
                                    </div>
                                    <div style="flex: 1;">
                                        <div style=move || format!("font-size: 15px; font-weight: 700; color: {};",
                                            if config.get().allow_bots_mode == id_c4 { "rgba(255,245,240,0.9)" } else { "rgba(255,245,240,0.7)" })>
                                            {*name}
                                        </div>
                                        <div style="font-size: 12px; color: rgba(255,245,240,0.5); margin-top: 2px;">{*desc}</div>
                                    </div>
                                </div>
                            </div>
                        }
                    }).collect::<Vec<_>>()}
                </div>
            </div>

        </div>
    }
}

// ─── STEP 5: SECURITY ───────────────────────────────────

#[component]
fn StepSecurity(config: RwSignal<OnboardConfig>) -> impl IntoView {
    view! {
        <div class="animate-fade-in">
            <div class="mb-5">
                <div class="font-orbitron text-[22px] font-semibold text-z-text">
                    <TypeWriter text="Define your armor" speed_ms=40 />
                </div>
                <p class="text-sm text-white/60 leading-relaxed">
                    "Zeus executes tools on your machine. Choose a security level that matches your trust and risk tolerance."
                </p>
            </div>

            <div style="display: flex; flex-direction: column; gap: 12px;">
                {SECURITY_LEVELS.iter().map(|l| {
                    let lid = l.id.to_string();
                    let lid_c1 = lid.clone();
                    let lid_c2 = lid.clone();
                    let lid_c3 = lid.clone();
                    let lid_c4 = lid.clone();
                    let lid_c4b = lid.clone();
                    let lid_c5 = lid.clone();
                    let name = l.name;
                    let desc = l.desc;
                    let risk = l.risk;
                    let color = l.color;
                    let features = l.features;

                    view! {
                        <div
                            style=move || {
                                let sel = config.get().security_level == lid_c1;
                                format!("padding: 24px; border-radius: 12px; cursor: pointer; transition: all 0.3s ease; background: rgba(255,255,255,0.03); border: 1px solid {}; box-shadow: {};",
                                    if sel { "rgba(255,60,20,0.25)" } else { "rgba(255,60,20,0.1)" },
                                    if sel { "0 0 30px rgba(255,60,20,0.15)" } else { "none" })
                            }
                            on:click={
                                let lid = lid.clone();
                                move |_| config.update(|c| c.security_level = lid.clone())
                            }
                        >
                            <div style="display: flex; align-items: flex-start; gap: 16px;">
                                <div style=move || format!(
                                    "width: 22px; height: 22px; border-radius: 11px; flex-shrink: 0; border: 2px solid {}; display: flex; align-items: center; justify-content: center; transition: all 0.3s; margin-top: 2px;",
                                    if config.get().security_level == lid_c2 { "rgba(255,60,20,1)" } else { "rgba(255,60,20,0.1)" }
                                )>
                                    <Show when=move || config.get().security_level == lid_c3>
                                        <div style="width: 10px; height: 10px; border-radius: 5px; background: rgba(255,60,20,1);" />
                                    </Show>
                                </div>
                                <div class="flex-1">
                                    <div style="display: flex; align-items: center; gap: 10px; margin-bottom: 8px;">
                                        <span style=move || format!("font-family: 'Orbitron', monospace; font-size: 15px; font-weight: 900; letter-spacing: 5px; color: {};", 
                                            if config.get().security_level == lid_c4 { "rgba(255,245,240,0.9)" } else { "rgba(255,245,240,0.7)" })>
                                            {name}
                                        </span>
                                        <span style={format!("font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 2px; color: {}; background: {}18; padding: 2px 8px; border-radius: 4px;", color, color)}>
                                            {risk}
                                        </span>
                                    </div>
                                    <div style=move || format!("font-size: 14px; color: {}; line-height: 1.65; margin-bottom: 14px;",
                                            if config.get().security_level == lid_c4b { "rgba(255,245,240,0.7)" } else { "rgba(255,245,240,0.5)" })>
                                        {desc}
                                    </div>
                                    <div style="display: flex; gap: 6px; flex-wrap: wrap;">
                                        {features.iter().map(|f| {
                                            let lid_tag = lid_c5.clone();
                                            view! {
                                            <span style=move || format!("font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 1px; color: {}; background: rgba(255,255,255,0.03); padding: 4px 10px; border-radius: 5px; border: 1px solid rgba(255,60,20,0.1);",
                                                    if config.get().security_level == lid_tag { "rgba(255,245,240,0.7)" } else { "rgba(255,245,240,0.5)" })>
                                                {*f}
                                            </span>
                                            }
                                        }).collect::<Vec<_>>()}
                                    </div>
                                </div>
                            </div>
                        </div>
                    }
                }).collect::<Vec<_>>()}
            </div>
        </div>
    }
}

// ─── STEP 6: FEATURES ───────────────────────────────────

#[component]
fn StepFeatures(config: RwSignal<OnboardConfig>) -> impl IntoView {
    view! {
        <div class="animate-fade-in">
            <div class="mb-5">
                <div class="font-orbitron text-[22px] font-semibold text-z-text">
                    <TypeWriter text="Unlock abilities" speed_ms=40 />
                </div>
                <p class="text-sm text-white/60 leading-relaxed">
                    "Zeus ships with a modular subsystem architecture. Enable the capabilities you need."
                </p>
            </div>

            <div style="display: flex; flex-direction: column; gap: 8px;">
                {FEATURES.iter().map(|f| {
                    let fid = f.id.to_string();
                    let fid_c1 = fid.clone();
                    let fid_c2 = fid.clone();
                    let fid_c2b = fid.clone();
                    let fid_c3 = fid.clone();
                    let fid_c4 = fid.clone();
                    let name = f.name;
                    let desc = f.desc;
                    let is_default = f.default;

                    view! {
                        <div
                            style=move || {
                                let active = config.get().features.contains(&fid_c1);
                                format!("display: flex; align-items: center; gap: 16px; padding: 16px 20px; border-radius: 12px; cursor: pointer; transition: all 0.3s ease; background: rgba(255,255,255,0.03); border: 1px solid {}; box-shadow: {};",
                                    if active { "rgba(255,60,20,0.25)" } else { "rgba(255,60,20,0.1)" },
                                    if active { "0 0 30px rgba(255,60,20,0.15)" } else { "none" })
                            }
                            on:click={
                                let fid = fid.clone();
                                move |_| config.update(|c| {
                                    if c.features.contains(&fid) {
                                        c.features.retain(|x| x != &fid);
                                    } else {
                                        c.features.push(fid.clone());
                                    }
                                })
                            }
                        >
                            // Toggle switch
                            <div style=move || format!(
                                "width: 44px; height: 24px; border-radius: 12px; padding: 2px; flex-shrink: 0; background: {}; transition: all 0.3s; cursor: pointer;",
                                if config.get().features.contains(&fid_c2) { "rgba(255,60,20,0.4)" } else { "rgba(255,255,255,0.08)" }
                            )>
                                <div style=move || format!(
                                    "width: 20px; height: 20px; border-radius: 10px; background: {}; transition: all 0.3s cubic-bezier(0.16,1,0.3,1); transform: {};",
                                    if config.get().features.contains(&fid_c2b) { "#fff" } else { "rgba(255,255,255,0.2)" },
                                    if config.get().features.contains(&fid_c2b) { "translateX(20px)" } else { "translateX(0)" }
                                ) />
                            </div>
                            <div class="flex-1">
                                <div style=move || format!("font-size: 15px; font-weight: 700; color: {};",
                                    if config.get().features.contains(&fid_c3) { "rgba(255,245,240,0.9)" } else { "rgba(255,245,240,0.7)" })>{name}</div>
                                <div style=move || format!("font-size: 12px; color: {}; margin-top: 2px;",
                                    if config.get().features.contains(&fid_c4) { "rgba(255,245,240,0.7)" } else { "rgba(255,245,240,0.5)" })>{desc}</div>
                            </div>
                            {is_default.then(|| view! {
                                <span style="font-family: 'Orbitron', monospace; font-size: 8px; font-weight: 700; letter-spacing: 1.5px; color: rgba(255,245,240,0.5); background: rgba(255,245,240,0.05); padding: 3px 8px; border-radius: 4px;">
                                    "DEFAULT"
                                </span>
                            })}
                        </div>
                    }
                }).collect::<Vec<_>>()}
            </div>

            // Counter
            <div style="margin-top: 14px; padding: 14px 18px; background: rgba(255,255,255,0.03); border-radius: 12px; border: 1px solid rgba(255,60,20,0.1); transition: all 0.3s ease;">
                <span style="font-family: 'Orbitron', monospace; font-size: 10px; color: rgba(255,60,20,1); letter-spacing: 3px; font-weight: 700;">
                    {move || format!("{} ABILITIES ENABLED", config.get().features.len())}
                </span>
            </div>
        </div>
    }
}

// ─── STEP 7: SERVICES ───────────────────────────────────

#[component]
fn StepServices(config: RwSignal<OnboardConfig>) -> impl IntoView {
    view! {
        <div class="animate-fade-in">
            <div style="margin-bottom: 24px;">
                <div class="font-orbitron text-[22px] font-semibold text-z-text">
                    <TypeWriter text="Connect services" speed_ms=40 />
                </div>
                <p class="text-sm text-white/60 leading-relaxed">
                    "Optional: connect image generation, voice, and video services. All skippable — configure later in Settings."
                </p>
            </div>

            // Image Generation
            <div class="mb-5">
                <div class="font-orbitron text-[10px] tracking-[3px] text-z-accent">
                    "IMAGE GENERATION"
                </div>
                <div class="flex flex-col gap-1.5">
                    {["fooocus", "automatic1111", "comfyui", "openai", "openai_compatible"].iter().map(|pid| {
                        let (name, desc, def_url, needs_key) = match *pid {
                            "fooocus"            => ("Fooocus",           "Local Stable Diffusion UI (recommended)",  "http://localhost:8888", false),
                            "automatic1111"      => ("Automatic1111",     "Local SD WebUI — A1111 compatible",        "http://localhost:7860", false),
                            "comfyui"            => ("ComfyUI",           "Node-based local Stable Diffusion",        "http://localhost:8188", false),
                            "openai"             => ("OpenAI DALL-E",     "DALL-E 3 via OpenAI API",                  "",                      true),
                            "openai_compatible"  => ("OpenAI-compatible", "Any OpenAI-compatible image endpoint",     "http://localhost:8080", false),
                            _                    => ("Unknown",           "",                                         "",                      false),
                        };
                        let pid_s = pid.to_string();
                        let pid_c = pid_s.clone();
                        let pid_d = pid_s.clone();
                        let pid_r = pid_s.clone();
                        let def_url_s = def_url.to_string();
                        let def_url_c = def_url_s.clone();
                        view! {
                            <div>
                                <div
                                    style=move || {
                                        let active = config.get().image_gen_provider == pid_c;
                                        format!("padding: 14px 18px; border-radius: 10px; cursor: pointer; transition: all 0.3s; background: rgba(255,255,255,0.03); border: 1px solid {};",
                                            if active { "rgba(255,60,20,0.3)" } else { "rgba(255,60,20,0.1)" })
                                    }
                                    on:click={
                                        let ps = pid_s.clone();
                                        let du = def_url_s.clone();
                                        move |_| config.update(|c| {
                                            if c.image_gen_provider == ps {
                                                c.image_gen_provider = String::new();
                                                c.image_gen_url = String::new();
                                            } else {
                                                c.image_gen_provider = ps.clone();
                                                if c.image_gen_url.is_empty() {
                                                    c.image_gen_url = du.clone();
                                                }
                                            }
                                        })
                                    }
                                >
                                    <div style="display: flex; align-items: center; justify-content: space-between;">
                                        <div>
                                            <div style=move || format!("font-size: 14px; font-weight: 700; color: {};",
                                                if config.get().image_gen_provider == pid_d { "rgba(255,245,240,0.95)" } else { "rgba(255,245,240,0.7)" })>
                                                {name}
                                            </div>
                                            <div style="font-size: 12px; color: rgba(255,245,240,0.5); margin-top: 2px;">{desc}</div>
                                        </div>
                                        <div style=move || format!(
                                            "width: 18px; height: 18px; border-radius: 50%; border: 2px solid {}; background: {}; flex-shrink: 0; transition: all 0.3s;",
                                            if config.get().image_gen_provider == pid_r { "rgba(255,60,20,1)" } else { "rgba(255,60,20,0.2)" },
                                            if config.get().image_gen_provider == pid_r { "rgba(255,60,20,1)" } else { "transparent" }
                                        ) />
                                    </div>
                                </div>
                                {
                                    let pe = pid_s.clone();
                                    let pu = pid_s.clone();
                                    let pk = pid_s.clone();
                                    let du2 = def_url_c.clone();
                                    move || {
                                        if config.get().image_gen_provider == pe {
                                            view! {
                                                <div on:click=|ev: web_sys::MouseEvent| ev.stop_propagation() style="padding: 10px 18px 4px; display: flex; flex-direction: column; gap: 10px;">
                                                    {if !du2.is_empty() {
                                                        let pu2 = pu.clone();
                                                        let du3 = du2.clone();
                                                        view! {
                                                            <div>
                                                                <div style="font-size: 11px; color: rgba(255,245,240,0.5); margin-bottom: 4px; font-weight: 600;">"URL"</div>
                                                                <input
                                                                    type="text"
                                                                    placeholder={du3}
                                                                    prop:value=move || if config.get().image_gen_provider == pu2 { config.get().image_gen_url } else { String::new() }
                                                                    on:input=move |e| { let v = event_target_value(&e); config.update(|c| c.image_gen_url = v); }
                                                                    class="w-full box-border bg-white/4 border border-z-border rounded-lg px-3.5 py-2.5 text-sm text-z-text font-rajdhani outline-none focus:border-z-accent"
                                                                />
                                                            </div>
                                                        }.into_any()
                                                    } else {
                                                        view! { <div /> }.into_any()
                                                    }}
                                                    {if needs_key {
                                                        let pk2 = pk.clone();
                                                        view! {
                                                            <div>
                                                                <div style="font-size: 11px; color: rgba(255,245,240,0.5); margin-bottom: 4px; font-weight: 600;">"API KEY"</div>
                                                                <input
                                                                    type="password"
                                                                    placeholder="sk-..."
                                                                    prop:value=move || if config.get().image_gen_provider == pk2 { config.get().image_gen_api_key.clone() } else { String::new() }
                                                                    on:input=move |e| { let v = event_target_value(&e); config.update(|c| c.image_gen_api_key = v); }
                                                                    class="w-full box-border bg-white/4 border border-z-border rounded-lg px-3.5 py-2.5 text-sm text-z-text font-rajdhani outline-none focus:border-z-accent"
                                                                />
                                                            </div>
                                                        }.into_any()
                                                    } else {
                                                        view! { <div /> }.into_any()
                                                    }}
                                                </div>
                                            }.into_any()
                                        } else {
                                            view! { <div /> }.into_any()
                                        }
                                    }
                                }
                            </div>
                        }
                    }).collect::<Vec<_>>()}
                </div>
            </div>

            // Voice STT / TTS
            <div class="mb-5">
                <div class="font-orbitron text-[10px] tracking-[3px] text-z-accent">
                    "VOICE — STT / TTS"
                </div>
                <div style="display: flex; flex-direction: column; gap: 10px; padding: 16px 18px; border-radius: 12px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1);">
                    <div>
                        <div class="flex justify-between items-baseline">
                            <div class="text-[13px] font-bold text-white/80">"Piper TTS URL"</div>
                            <div class="text-[11px] text-white/35">"Text-to-speech engine"</div>
                        </div>
                        <input
                            type="text"
                            placeholder="http://localhost:8104"
                            prop:value=move || config.get().piper_url
                            on:input=move |e| { let v = event_target_value(&e); config.update(|c| c.piper_url = v); }
                            class="w-full box-border bg-white/4 border border-z-border rounded-lg px-3.5 py-2.5 text-sm text-z-text font-rajdhani outline-none focus:border-z-accent"
                        />
                    </div>
                    <div>
                        <div class="flex justify-between items-baseline">
                            <div class="text-[13px] font-bold text-white/80">"Whisper STT URL"</div>
                            <div class="text-[11px] text-white/35">"Speech-to-text engine"</div>
                        </div>
                        <input
                            type="text"
                            placeholder="http://localhost:9000"
                            prop:value=move || config.get().whisper_url
                            on:input=move |e| { let v = event_target_value(&e); config.update(|c| c.whisper_url = v); }
                            class="w-full box-border bg-white/4 border border-z-border rounded-lg px-3.5 py-2.5 text-sm text-z-text font-rajdhani outline-none focus:border-z-accent"
                        />
                    </div>
                    <div>
                        <div class="flex justify-between items-baseline">
                            <div class="text-[13px] font-bold text-white/80">"ElevenLabs API Key"</div>
                            <div class="text-[11px] text-white/35">"Premium cloud TTS (optional — overrides Piper)"</div>
                        </div>
                        <input
                            type="password"
                            placeholder="sk_..."
                            prop:value=move || config.get().elevenlabs_api_key
                            on:input=move |e| { let v = event_target_value(&e); config.update(|c| c.elevenlabs_api_key = v); }
                            class="w-full box-border bg-white/4 border border-z-border rounded-lg px-3.5 py-2.5 text-sm text-z-text font-rajdhani outline-none focus:border-z-accent"
                        />
                    </div>
                </div>
            </div>

            // Video Generation
            <div class="mb-4">
                <div class="font-orbitron text-[10px] tracking-[3px] text-z-accent">
                    "VIDEO GENERATION"
                </div>
                <div style="padding: 16px 18px; border-radius: 12px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1);">
                    <div class="flex justify-between items-baseline">
                        <div class="text-[13px] font-bold text-white/80">"ComfyUI / Video Gen URL"</div>
                        <div class="text-[11px] text-white/35">"Wan2.1, CogVideo, AnimateDiff..."</div>
                    </div>
                    <input
                        type="text"
                        placeholder="http://localhost:8188"
                        prop:value=move || config.get().video_url
                        on:input=move |e| { let v = event_target_value(&e); config.update(|c| c.video_url = v); }
                        class="w-full box-border bg-white/4 border border-z-border rounded-lg px-3.5 py-2.5 text-sm text-z-text font-rajdhani outline-none focus:border-z-accent"
                    />
                </div>
            </div>

            // Summary badge
            <div style="padding: 12px 18px; background: rgba(255,255,255,0.03); border-radius: 12px; border: 1px solid rgba(255,60,20,0.1);">
                <span style="font-family: 'Orbitron', monospace; font-size: 10px; color: rgba(255,245,240,0.5); letter-spacing: 2px; font-weight: 700;">
                    {move || {
                        let c = config.get();
                        let mut parts: Vec<String> = Vec::new();
                        if !c.image_gen_provider.is_empty() { parts.push(format!("IMG: {}", c.image_gen_provider.to_uppercase())); }
                        // TTS: prefer ElevenLabs if key set, else Piper (matches TUI auto-select)
                        if !c.elevenlabs_api_key.is_empty() { parts.push("TTS: ELEVENLABS".to_string()); }
                        else if !c.piper_url.is_empty() { parts.push("TTS: PIPER".to_string()); }
                        if !c.whisper_url.is_empty() { parts.push("STT: WHISPER".to_string()); }
                        if !c.video_url.is_empty() { parts.push("VIDEO: COMFYUI".to_string()); }
                        if parts.is_empty() { "NO SERVICES CONFIGURED — SKIPPABLE".to_string() }
                        else { parts.join(" · ") }
                    }}
                </span>
            </div>
        </div>
    }
}

// ─── STEP 8: ORCHESTRATION ───────────────────────────────

#[component]
fn StepOrchestration(config: RwSignal<OnboardConfig>) -> impl IntoView {
    view! {
        <div class="animate-fade-in">
            <div class="mb-5">
                <div class="font-orbitron text-[22px] font-semibold text-z-text">
                    <TypeWriter text="Configure orchestration" speed_ms=40 />
                </div>
                <p class="text-sm text-white/60 leading-relaxed">
                    "Prometheus orchestrates complex tasks. Nous provides cognitive intelligence — intent recognition and learning."
                </p>
            </div>

            // Orchestration mode radios
            <div class="mb-5">
                <div class="font-orbitron text-[10px] tracking-[3px] text-z-accent">
                    "ORCHESTRATION MODE"
                </div>
                <div style="display: flex; flex-direction: column; gap: 8px;">
                    {[("enable_all", "Enable All", "Full orchestration — heartbeat, planning, cooking loop (recommended)"),
                      ("disable", "Disable", "No orchestration — agent responds directly without planning"),
                      ("custom", "Custom", "Fine-tune heartbeat interval and cognitive features")].iter().map(|(id, name, desc)| {
                        let id_s = id.to_string();
                        let id_c1 = id_s.clone();
                        let id_c2 = id_s.clone();
                        let id_c3 = id_s.clone();
                        let id_c4 = id_s.clone();
                        view! {
                            <div
                                style=move || {
                                    let sel = config.get().orchestration_mode == id_c1;
                                    format!("padding: 16px 20px; border-radius: 12px; cursor: pointer; transition: all 0.3s ease; background: rgba(255,255,255,0.03); border: 1px solid {};",
                                        if sel { "rgba(255,60,20,0.25)" } else { "rgba(255,60,20,0.1)" })
                                }
                                on:click={
                                    let id_click = id_s.clone();
                                    move |_| config.update(|c| c.orchestration_mode = id_click.clone())
                                }
                            >
                                <div style="display: flex; align-items: center; gap: 14px;">
                                    <div style=move || format!(
                                        "width: 22px; height: 22px; border-radius: 11px; flex-shrink: 0; border: 2px solid {}; display: flex; align-items: center; justify-content: center; transition: all 0.3s;",
                                        if config.get().orchestration_mode == id_c2 { "rgba(255,60,20,1)" } else { "rgba(255,60,20,0.1)" }
                                    )>
                                        <Show when={
                                            let id_show = id_c3.clone();
                                            move || config.get().orchestration_mode == id_show
                                        }>
                                            <div style="width: 10px; height: 10px; border-radius: 5px; background: rgba(255,60,20,1);" />
                                        </Show>
                                    </div>
                                    <div class="flex-1">
                                        <div style=move || format!("font-size: 15px; font-weight: 700; color: {};",
                                            if config.get().orchestration_mode == id_c4 { "rgba(255,245,240,0.9)" } else { "rgba(255,245,240,0.7)" })>
                                            {*name}
                                        </div>
                                        <div style="font-size: 12px; color: rgba(255,245,240,0.5); margin-top: 2px;">{*desc}</div>
                                    </div>
                                </div>
                            </div>
                        }
                    }).collect::<Vec<_>>()}
                </div>
            </div>

            // Custom: heartbeat interval (only shown when custom)
            <Show when=move || config.get().orchestration_mode == "custom">
                <div class="mb-5">
                    <div class="font-orbitron text-[10px] tracking-[3px] text-z-accent">
                        "HEARTBEAT INTERVAL"
                    </div>
                    <div style="padding: 16px 18px; border-radius: 12px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1);">
                        <div class="flex justify-between items-baseline">
                            <div class="text-[13px] font-bold text-white/80">"Interval (seconds)"</div>
                            <div class="text-[11px] text-white/35">"How often Prometheus checks for proactive tasks"</div>
                        </div>
                        <input
                            type="text"
                            placeholder="300"
                            prop:value=move || config.get().heartbeat_interval.clone()
                            on:input=move |e| { let v = event_target_value(&e); config.update(|c| c.heartbeat_interval = v); }
                            class="w-full box-border bg-white/4 border border-z-border rounded-lg px-3.5 py-2.5 text-sm text-z-text font-rajdhani outline-none focus:border-z-accent"
                        />
                    </div>
                </div>
            </Show>

            // Nous toggles
            <div class="mb-5">
                <div class="font-orbitron text-[10px] tracking-[3px] text-z-accent">
                    "COGNITIVE ENGINE (NOUS)"
                </div>
                <div style="display: flex; flex-direction: column; gap: 8px;">
                    // Intent recognition toggle
                    <div
                        style=move || {
                            let active = config.get().nous_intent;
                            format!("display: flex; align-items: center; gap: 16px; padding: 16px 20px; border-radius: 12px; cursor: pointer; transition: all 0.3s ease; background: rgba(255,255,255,0.03); border: 1px solid {};",
                                if active { "rgba(255,60,20,0.25)" } else { "rgba(255,60,20,0.1)" })
                        }
                        on:click=move |_| config.update(|c| c.nous_intent = !c.nous_intent)
                    >
                        <div style=move || format!(
                            "width: 44px; height: 24px; border-radius: 12px; padding: 2px; flex-shrink: 0; background: {}; transition: all 0.3s; cursor: pointer;",
                            if config.get().nous_intent { "rgba(255,60,20,0.4)" } else { "rgba(255,255,255,0.08)" }
                        )>
                            <div style=move || format!(
                                "width: 20px; height: 20px; border-radius: 10px; background: {}; transition: all 0.3s cubic-bezier(0.16,1,0.3,1); transform: {};",
                                if config.get().nous_intent { "#fff" } else { "rgba(255,255,255,0.2)" },
                                if config.get().nous_intent { "translateX(20px)" } else { "translateX(0)" }
                            ) />
                        </div>
                        <div class="flex-1">
                            <div style=move || format!("font-size: 15px; font-weight: 700; color: {};",
                                if config.get().nous_intent { "rgba(255,245,240,0.9)" } else { "rgba(255,245,240,0.7)" })>"Intent Recognition"</div>
                            <div style=move || format!("font-size: 12px; color: {}; margin-top: 2px;",
                                if config.get().nous_intent { "rgba(255,245,240,0.7)" } else { "rgba(255,245,240,0.5)" })>"Classify user intent to route simple vs. complex tasks"</div>
                        </div>
                    </div>
                    // Learning toggle
                    <div
                        style=move || {
                            let active = config.get().nous_learning;
                            format!("display: flex; align-items: center; gap: 16px; padding: 16px 20px; border-radius: 12px; cursor: pointer; transition: all 0.3s ease; background: rgba(255,255,255,0.03); border: 1px solid {};",
                                if active { "rgba(255,60,20,0.25)" } else { "rgba(255,60,20,0.1)" })
                        }
                        on:click=move |_| config.update(|c| c.nous_learning = !c.nous_learning)
                    >
                        <div style=move || format!(
                            "width: 44px; height: 24px; border-radius: 12px; padding: 2px; flex-shrink: 0; background: {}; transition: all 0.3s; cursor: pointer;",
                            if config.get().nous_learning { "rgba(255,60,20,0.4)" } else { "rgba(255,255,255,0.08)" }
                        )>
                            <div style=move || format!(
                                "width: 20px; height: 20px; border-radius: 10px; background: {}; transition: all 0.3s cubic-bezier(0.16,1,0.3,1); transform: {};",
                                if config.get().nous_learning { "#fff" } else { "rgba(255,255,255,0.2)" },
                                if config.get().nous_learning { "translateX(20px)" } else { "translateX(0)" }
                            ) />
                        </div>
                        <div class="flex-1">
                            <div style=move || format!("font-size: 15px; font-weight: 700; color: {};",
                                if config.get().nous_learning { "rgba(255,245,240,0.9)" } else { "rgba(255,245,240,0.7)" })>"Learning from Interactions"</div>
                            <div style=move || format!("font-size: 12px; color: {}; margin-top: 2px;",
                                if config.get().nous_learning { "rgba(255,245,240,0.7)" } else { "rgba(255,245,240,0.5)" })>"Improve responses over time based on feedback and outcomes"</div>
                        </div>
                    </div>
                </div>
            </div>

            // Notifications (Hermes)
            <div class="mb-5">
                <div class="font-orbitron text-[10px] tracking-[3px] text-z-accent">
                    "NOTIFICATIONS"
                </div>
                <div style="display: flex; flex-direction: column; gap: 8px;">
                    {[("console", "Console", "Print notifications to terminal output"),
                      ("telegram", "Telegram", "Send notifications via Telegram channel"),
                      ("discord", "Discord", "Send notifications via Discord channel")].iter().map(|(id, name, desc)| {
                        let id_s = id.to_string();
                        let id_c1 = id_s.clone();
                        let id_c2 = id_s.clone();
                        let id_c3 = id_s.clone();
                        let id_c4 = id_s.clone();
                        view! {
                            <div
                                style=move || {
                                    let sel = config.get().hermes_channel == id_c1;
                                    format!("padding: 16px 20px; border-radius: 12px; cursor: pointer; transition: all 0.3s ease; background: rgba(255,255,255,0.03); border: 1px solid {};",
                                        if sel { "rgba(255,60,20,0.25)" } else { "rgba(255,60,20,0.1)" })
                                }
                                on:click={
                                    let id_click = id_s.clone();
                                    move |_| config.update(|c| c.hermes_channel = id_click.clone())
                                }
                            >
                                <div style="display: flex; align-items: center; gap: 14px;">
                                    <div style=move || format!(
                                        "width: 22px; height: 22px; border-radius: 11px; flex-shrink: 0; border: 2px solid {}; display: flex; align-items: center; justify-content: center; transition: all 0.3s;",
                                        if config.get().hermes_channel == id_c2 { "rgba(255,60,20,1)" } else { "rgba(255,60,20,0.1)" }
                                    )>
                                        <Show when={
                                            let id_show = id_c3.clone();
                                            move || config.get().hermes_channel == id_show
                                        }>
                                            <div style="width: 10px; height: 10px; border-radius: 5px; background: rgba(255,60,20,1);" />
                                        </Show>
                                    </div>
                                    <div style="flex: 1;">
                                        <div style=move || format!("font-size: 15px; font-weight: 700; color: {};",
                                            if config.get().hermes_channel == id_c4 { "rgba(255,245,240,0.9)" } else { "rgba(255,245,240,0.7)" })>
                                            {*name}
                                        </div>
                                        <div style="font-size: 12px; color: rgba(255,245,240,0.5); margin-top: 2px;">{*desc}</div>
                                    </div>
                                </div>
                            </div>
                        }
                    }).collect::<Vec<_>>()}
                </div>
            </div>

            // LLM Council toggle
            <div class="mb-5" style="margin-top: 16px;">
                <div class="font-orbitron text-[10px] tracking-[3px] text-z-accent">"LLM COUNCIL"</div>
                <div
                    style=move || format!(
                        "display: flex; align-items: center; gap: 12px; padding: 14px 18px; background: {}; border: 1px solid rgba(255,60,20,0.1); border-radius: 12px; cursor: pointer; transition: all 0.3s;",
                        if config.get().council_enabled { "rgba(255,60,20,0.08)" } else { "rgba(255,255,255,0.03)" }
                    )
                    on:click=move |_| config.update(|c| c.council_enabled = !c.council_enabled)
                >
                    <div style=move || format!(
                        "width: 40px; height: 22px; border-radius: 11px; background: {}; position: relative; transition: all 0.3s; flex-shrink: 0;",
                        if config.get().council_enabled { "rgba(255,60,20,0.4)" } else { "rgba(255,255,255,0.08)" }
                    )>
                        <div style=move || format!(
                            "width: 16px; height: 16px; border-radius: 50%; background: {}; position: absolute; top: 3px; left: 3px; transition: transform 0.3s; transform: {};",
                            if config.get().council_enabled { "#fff" } else { "rgba(255,255,255,0.2)" },
                            if config.get().council_enabled { "translateX(18px)" } else { "translateX(0)" }
                        ) />
                    </div>
                    <div>
                        <div style=move || format!("font-size: 14px; font-weight: 700; color: {};",
                            if config.get().council_enabled { "rgba(255,245,240,0.9)" } else { "rgba(255,245,240,0.7)" })>"Multi-Model Council"</div>
                        <div style="font-size: 11px; color: rgba(255,245,240,0.5); margin-top: 2px;">"Query multiple LLMs and synthesize responses (experimental)"</div>
                    </div>
                </div>
            </div>

            // Summary badge
            <div style="padding: 12px 18px; background: rgba(255,255,255,0.03); border-radius: 12px; border: 1px solid rgba(255,60,20,0.1);">
                <span style="font-family: 'Orbitron', monospace; font-size: 10px; color: rgba(255,245,240,0.5); letter-spacing: 2px; font-weight: 700;">
                    {move || {
                        let c = config.get();
                        let mode = c.orchestration_mode.to_uppercase();
                        let nous = match (c.nous_intent, c.nous_learning) {
                            (true, true) => "INTENT + LEARNING",
                            (true, false) => "INTENT ONLY",
                            (false, true) => "LEARNING ONLY",
                            (false, false) => "NOUS DISABLED",
                        };
                        let hermes = c.hermes_channel.to_uppercase();
                        let council = if c.council_enabled { " · COUNCIL" } else { "" };
                        format!("ORCHESTRATION: {} · {} · NOTIFY: {}{}", mode, nous, hermes, council)
                    }}
                </span>
            </div>

            // Gateway settings
            <div class="mb-5" style="margin-top: 24px; padding-top: 16px; border-top: 1px solid rgba(255,60,20,0.1);">
                <div class="font-orbitron text-[10px] tracking-[3px] text-z-accent">
                    "GATEWAY"
                </div>
                <div style="display: flex; gap: 16px; flex-wrap: wrap;">
                    <div style="flex: 1; min-width: 200px;">
                        <label style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,245,240,0.5); text-transform: uppercase; display: block; margin-bottom: 6px;">
                            "Timeout (seconds)"
                        </label>
                        <input
                            style="width: 100%; padding: 10px 14px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 8px; color: rgba(255,245,240,0.9); font-family: 'Orbitron', monospace; font-size: 12px; outline: none; box-sizing: border-box;"
                            type="number"
                            placeholder="1800"
                            prop:value=move || config.get().gateway_timeout.clone()
                            on:input=move |ev| config.update(|c| c.gateway_timeout = event_target_value(&ev))
                        />
                        <div style="font-size: 9px; color: rgba(255,245,240,0.4); margin-top: 4px; font-family: 'Orbitron', monospace;">
                            "Max time for a single request (default: 1800 = 30 min)"
                        </div>
                    </div>
                    <div style="flex: 1; min-width: 200px;">
                        <label style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,245,240,0.5); text-transform: uppercase; display: block; margin-bottom: 6px;">
                            "Mentions Only"
                        </label>
                        <div
                            style=move || format!(
                                "display: flex; align-items: center; gap: 10px; padding: 10px 14px; background: {}; border: 1px solid rgba(255,60,20,0.1); border-radius: 8px; cursor: pointer; transition: all 0.3s;",
                                if config.get().gateway_mentions_only { "rgba(255,60,20,0.1)" } else { "rgba(255,255,255,0.03)" }
                            )
                            on:click=move |_| config.update(|c| c.gateway_mentions_only = !c.gateway_mentions_only)
                        >
                            <div style=move || format!(
                                "width: 18px; height: 18px; border-radius: 4px; border: 1.5px solid {}; display: flex; align-items: center; justify-content: center;",
                                if config.get().gateway_mentions_only { "rgba(255,60,20,1)" } else { "rgba(255,60,20,0.2)" }
                            )>
                                <Show when=move || config.get().gateway_mentions_only>
                                    <div style="width: 8px; height: 8px; border-radius: 2px; background: rgba(255,60,20,1);" />
                                </Show>
                            </div>
                            <span style="font-family: 'Orbitron', monospace; font-size: 11px; color: rgba(255,245,240,0.8);">
                                "Only process @mentioned messages"
                            </span>
                        </div>
                        <div style="font-size: 9px; color: rgba(255,245,240,0.4); margin-top: 4px; font-family: 'Orbitron', monospace;">
                            "When enabled, agent ignores messages without @mention"
                        </div>
                    </div>
                </div>
            </div>
        </div>
    }
}

// ─── STEP 9: MEMORY ─────────────────────────────────────

#[component]
fn StepMemory(config: RwSignal<OnboardConfig>) -> impl IntoView {
    view! {
        <div class="animate-fade-in">
            <div class="mb-5">
                <div class="font-orbitron text-[22px] font-semibold text-z-text">
                    <TypeWriter text="Configure memory" speed_ms=40 />
                </div>
                <p class="text-sm text-white/60 leading-relaxed">
                    "Mnemosyne gives Zeus long-term memory. Choose which storage backends to enable."
                </p>
            </div>

            // Database path (TUI parity: memory_fields[0])
            <div class="mb-4">
                <label class="font-orbitron text-[10px] tracking-[3px] text-white/70 block mb-2 uppercase">"Database Path"</label>
                <input
                    style="width: 100%; padding: 12px 16px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 10px; color: rgba(255,245,240,0.9); font-family: 'Rajdhani', sans-serif; font-size: 15px; outline: none; box-sizing: border-box; transition: all 0.3s;"
                    type="text"
                    placeholder="~/.zeus/memory.db"
                    prop:value=move || config.get().memory_db_path.clone()
                    on:input=move |ev| config.update(|c| c.memory_db_path = event_target_value(&ev))
                />
                <div style="font-size: 11px; color: rgba(255,245,240,0.3); margin-top: 6px;">
                    "Where the SQLite memory database lives."
                </div>
            </div>

            <div style="display: flex; flex-direction: column; gap: 8px;">
                // FTS5 toggle
                <div
                    style=move || {
                        let active = config.get().memory_fts;
                        format!("display: flex; align-items: center; gap: 16px; padding: 16px 20px; border-radius: 12px; cursor: pointer; transition: all 0.3s ease; background: rgba(255,255,255,0.03); border: 1px solid {};",
                            if active { "rgba(255,60,20,0.25)" } else { "rgba(255,60,20,0.1)" })
                    }
                    on:click=move |_| config.update(|c| c.memory_fts = !c.memory_fts)
                >
                    <div style=move || format!(
                        "width: 44px; height: 24px; border-radius: 12px; padding: 2px; flex-shrink: 0; background: {}; transition: all 0.3s; cursor: pointer;",
                        if config.get().memory_fts { "rgba(255,60,20,0.4)" } else { "rgba(255,255,255,0.08)" }
                    )>
                        <div style=move || format!(
                            "width: 20px; height: 20px; border-radius: 10px; background: {}; transition: all 0.3s cubic-bezier(0.16,1,0.3,1); transform: {};",
                            if config.get().memory_fts { "#fff" } else { "rgba(255,255,255,0.2)" },
                            if config.get().memory_fts { "translateX(20px)" } else { "translateX(0)" }
                        ) />
                    </div>
                    <div class="flex-1">
                        <div style=move || format!("font-size: 15px; font-weight: 700; color: {};",
                            if config.get().memory_fts { "rgba(255,245,240,0.9)" } else { "rgba(255,245,240,0.7)" })>"Full-Text Search (FTS5)"</div>
                        <div style=move || format!("font-size: 12px; color: {}; margin-top: 2px;",
                            if config.get().memory_fts { "rgba(255,245,240,0.7)" } else { "rgba(255,245,240,0.5)" })>"SQLite FTS5 index for fast keyword search across all stored memories. Recommended for all setups."</div>
                    </div>
                    <span style="font-family: 'Orbitron', monospace; font-size: 8px; font-weight: 700; letter-spacing: 1.5px; color: rgba(255,245,240,0.5); background: rgba(255,245,240,0.05); padding: 3px 8px; border-radius: 4px;">
                        "DEFAULT"
                    </span>
                </div>
                // Vector embeddings toggle
                <div
                    style=move || {
                        let active = config.get().memory_embeddings;
                        format!("display: flex; align-items: center; gap: 16px; padding: 16px 20px; border-radius: 12px; cursor: pointer; transition: all 0.3s ease; background: rgba(255,255,255,0.03); border: 1px solid {};",
                            if active { "rgba(255,60,20,0.25)" } else { "rgba(255,60,20,0.1)" })
                    }
                    on:click=move |_| config.update(|c| c.memory_embeddings = !c.memory_embeddings)
                >
                    <div style=move || format!(
                        "width: 44px; height: 24px; border-radius: 12px; padding: 2px; flex-shrink: 0; background: {}; transition: all 0.3s; cursor: pointer;",
                        if config.get().memory_embeddings { "rgba(255,60,20,0.4)" } else { "rgba(255,255,255,0.08)" }
                    )>
                        <div style=move || format!(
                            "width: 20px; height: 20px; border-radius: 10px; background: {}; transition: all 0.3s cubic-bezier(0.16,1,0.3,1); transform: {};",
                            if config.get().memory_embeddings { "#fff" } else { "rgba(255,255,255,0.2)" },
                            if config.get().memory_embeddings { "translateX(20px)" } else { "translateX(0)" }
                        ) />
                    </div>
                    <div class="flex-1">
                        <div style=move || format!("font-size: 15px; font-weight: 700; color: {};",
                            if config.get().memory_embeddings { "rgba(255,245,240,0.9)" } else { "rgba(255,245,240,0.7)" })>"Vector Embeddings"</div>
                        <div style=move || format!("font-size: 12px; color: {}; margin-top: 2px;",
                            if config.get().memory_embeddings { "rgba(255,245,240,0.7)" } else { "rgba(255,245,240,0.5)" })>"Semantic search using embedding models (Ollama nomic-embed-text or OpenAI). Requires an embedding provider."</div>
                    </div>
                </div>
            </div>

            // Embedding provider selection (TUI parity: memory_fields[2])
            <Show when=move || config.get().memory_embeddings>
                <div style="margin-top: 14px;">
                    <label class="font-orbitron text-[10px] tracking-[3px] text-white/70 block mb-2 uppercase">"Embedding Provider"</label>
                    <div style="display: flex; gap: 8px; flex-wrap: wrap;">
                        {[("ollama", "Ollama", "Local — nomic-embed-text"),
                          ("openai", "OpenAI", "text-embedding-3"),
                          ("gemini", "Gemini", "embedding-001"),
                          ("voyage", "Voyage", "voyage-3")].iter().map(|(id, name, desc)| {
                            let pid: String = id.to_string();
                            let pid_c1 = pid.clone();
                            let pid_c2 = pid.clone();
                            let pid_click = pid.clone();
                            view! {
                                <div
                                    style=move || {
                                        let sel = config.get().memory_embedding_provider == pid_c1;
                                        format!("flex: 1; min-width: 120px; padding: 12px 14px; border-radius: 10px; cursor: pointer; transition: all 0.3s; background: rgba(255,255,255,0.03); border: 1px solid {};",
                                            if sel { "rgba(255,60,20,0.4)" } else { "rgba(255,60,20,0.1)" })
                                    }
                                    on:click=move |_| config.update(|c| c.memory_embedding_provider = pid_click.clone())
                                >
                                    <div style=move || format!("font-size: 14px; font-weight: 700; color: {};",
                                        if config.get().memory_embedding_provider == pid_c2 { "rgba(255,245,240,0.95)" } else { "rgba(255,245,240,0.7)" })>{*name}</div>
                                    <div style="font-size: 11px; color: rgba(255,245,240,0.4); margin-top: 2px;">{*desc}</div>
                                </div>
                            }
                        }).collect_view()}
                    </div>
                </div>
            </Show>

            // Summary badge
            <div style="margin-top: 14px; padding: 12px 18px; background: rgba(255,255,255,0.03); border-radius: 12px; border: 1px solid rgba(255,60,20,0.1);">
                <span style="font-family: 'Orbitron', monospace; font-size: 10px; color: rgba(255,245,240,0.5); letter-spacing: 2px; font-weight: 700;">
                    {move || {
                        let c = config.get();
                        let mut parts: Vec<&str> = Vec::new();
                        if c.memory_fts { parts.push("FTS5"); }
                        if c.memory_embeddings { parts.push("VECTORS"); }
                        if parts.is_empty() { "NO MEMORY BACKENDS — BASIC FILE MEMORY ONLY".to_string() }
                        else { format!("MEMORY: {}", parts.join(" + ")) }
                    }}
                </span>
            </div>

            // Workspace files guide
            <div style="margin-top: 20px; padding: 16px 18px; background: rgba(255,255,255,0.02); border-radius: 12px; border: 1px solid rgba(255,60,20,0.08);">
                <div style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,245,240,0.5); margin-bottom: 10px; text-transform: uppercase;">
                    "WORKSPACE FILES"
                </div>
                <div style="font-size: 12px; color: rgba(255,245,240,0.6); line-height: 1.8; font-family: monospace;">
                    <div><code style="color: rgba(255,60,20,0.6);">"~/.zeus/workspace/"</code></div>
                    <div style="padding-left: 16px; font-size: 11px;">
                        <div><code>"AGENTS.md"</code>" — agent identity and directives"</div>
                        <div><code>"SOUL.md"</code>" — personality and voice"</div>
                        <div><code>"USER.md"</code>" — your context (name, role, preferences)"</div>
                        <div><code>"HEARTBEAT.md"</code>" — proactive task schedule"</div>
                        <div><code>"memory/"</code>" — long-term facts and notes"</div>
                        <div><code>"daily/"</code>" — daily journal entries"</div>
                    </div>
                    <div style="margin-top: 6px; font-size: 10px; color: rgba(255,245,240,0.4);">
                        "Edit these files anytime to customize Zeus's behavior."
                    </div>
                </div>
            </div>
        </div>
    }
}

// ─── STEP 10: SKILLS ────────────────────────────────────

#[component]
fn StepSkills(config: RwSignal<OnboardConfig>) -> impl IntoView {
    let _ = config; // config collected but skills step doesn't write back to it
    let skills = RwSignal::new(vec![
        ("plan-then-execute".to_string(), "Plan-Then-Execute".to_string(), "Decompose complex goals into steps, execute sequentially, verify each".to_string(), true),
        ("systematic-debugging".to_string(), "Systematic Debugging".to_string(), "Binary search, hypothesis testing, root cause analysis protocol".to_string(), true),
        ("verification-gates".to_string(), "Verification Gates".to_string(), "Require proof of correctness before marking tasks complete".to_string(), false),
    ]);

    // Fetch skills from API, replace hardcoded defaults if available
    spawn_local({
        async move {
            if let Ok(resp) = api::fetch_json::<serde_json::Value>("/v1/onboarding/skills").await {
                if let Some(arr) = resp.as_array() {
                    let fetched: Vec<(String, String, String, bool)> = arr.iter().filter_map(|v| {
                        Some((
                            v["id"].as_str()?.to_string(),
                            v["name"].as_str()?.to_string(),
                            v["description"].as_str().unwrap_or("").to_string(),
                            v["default"].as_bool().unwrap_or(false),
                        ))
                    }).collect();
                    if !fetched.is_empty() {
                        skills.set(fetched);
                    }
                }
            }
        }
    });

    view! {
        <div class="animate-fade-in">
            <div class="mb-5">
                <div class="font-orbitron text-[22px] font-semibold text-z-text">
                    <TypeWriter text="Select skills" speed_ms=40 />
                </div>
                <p class="text-sm text-white/60 leading-relaxed">
                    "Skills are reusable reasoning patterns that enhance Zeus. Select built-in skills or browse community skills later."
                </p>
            </div>

            <div style="display: flex; flex-direction: column; gap: 8px;">
                {move || skills.get().iter().enumerate().map(|(idx, (_id, name, desc, enabled))| {
                    let name_c = name.clone();
                    let desc_c = desc.clone();
                    let is_enabled = *enabled;
                    view! {
                        <div
                            style=move || {
                                format!("display: flex; align-items: center; gap: 16px; padding: 16px 20px; border-radius: 12px; cursor: pointer; transition: all 0.3s ease; background: rgba(255,255,255,0.03); border: 1px solid {};",
                                    if is_enabled { "rgba(255,60,20,0.25)" } else { "rgba(255,60,20,0.1)" })
                            }
                            on:click=move |_| {
                                skills.update(|s| {
                                    if let Some(item) = s.get_mut(idx) {
                                        item.3 = !item.3;
                                    }
                                });
                            }
                        >
                            <div style=move || format!(
                                "width: 44px; height: 24px; border-radius: 12px; padding: 2px; flex-shrink: 0; background: {}; transition: all 0.3s; cursor: pointer;",
                                if is_enabled { "rgba(255,60,20,0.4)" } else { "rgba(255,255,255,0.08)" }
                            )>
                                <div style=move || format!(
                                    "width: 20px; height: 20px; border-radius: 10px; background: {}; transition: all 0.3s cubic-bezier(0.16,1,0.3,1); transform: {};",
                                    if is_enabled { "#fff" } else { "rgba(255,255,255,0.2)" },
                                    if is_enabled { "translateX(20px)" } else { "translateX(0)" }
                                ) />
                            </div>
                            <div class="flex-1">
                                <div style=move || format!("font-size: 15px; font-weight: 700; color: {};",
                                    if is_enabled { "rgba(255,245,240,0.9)" } else { "rgba(255,245,240,0.7)" })>{name_c.clone()}</div>
                                <div style=move || format!("font-size: 12px; color: {}; margin-top: 2px;",
                                    if is_enabled { "rgba(255,245,240,0.7)" } else { "rgba(255,245,240,0.5)" })>{desc_c.clone()}</div>
                            </div>
                        </div>
                    }
                }).collect::<Vec<_>>()}
            </div>

            // Browse link
            <div style="margin-top: 18px; text-align: center;">
                <a
                    href="https://github.com/zeuslabai/zeus-skills"
                    target="_blank"
                    rel="noopener noreferrer"
                    style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 2px; color: rgba(255,60,20,1); text-decoration: none; opacity: 0.7; transition: opacity 0.3s;"
                >
                    "BROWSE MORE AT GITHUB.COM/MIKEHASH/ZEUS-SKILLS →"
                </a>
            </div>

            // Summary badge
            <div style="margin-top: 14px; padding: 12px 18px; background: rgba(255,255,255,0.03); border-radius: 12px; border: 1px solid rgba(255,60,20,0.1);">
                <span style="font-family: 'Orbitron', monospace; font-size: 10px; color: rgba(255,245,240,0.5); letter-spacing: 2px; font-weight: 700;">
                    {move || {
                        let enabled = skills.get().iter().filter(|s| s.3).count();
                        format!("{} SKILLS ENABLED", enabled)
                    }}
                </span>
            </div>
        </div>
    }
}

// ─── STEP 11: LAUNCH ────────────────────────────────────

#[component]
fn StepLaunch(config: RwSignal<OnboardConfig>) -> impl IntoView {
    let phase = RwSignal::new(0u8);
    let show_config = RwSignal::new(false);
    let save_status = RwSignal::new(SaveStatus::Idle);
    let save_detail = RwSignal::new(String::new());
    // Gateway auto-start: URL shown after successful save
    let gateway_display_url = RwSignal::new(String::new());

    Effect::new(move |_| {
        let win = web_sys::window().unwrap();
        let p = phase;
        let cb1 = Closure::once(move || p.set(1));
        let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(cb1.as_ref().unchecked_ref(), 800);
        cb1.forget();
        let p = phase;
        let cb2 = Closure::once(move || p.set(2));
        let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(cb2.as_ref().unchecked_ref(), 2200);
        cb2.forget();
        let p = phase;
        let cb3 = Closure::once(move || p.set(3));
        let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(cb3.as_ref().unchecked_ref(), 4000);
        cb3.forget();
        let p = phase;
        let cb4 = Closure::once(move || p.set(4));
        let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(cb4.as_ref().unchecked_ref(), 6000);
        cb4.forget();
    });

    view! {
        <div style="display: flex; flex-direction: column; align-items: center; justify-content: center; min-height: 80vh; text-align: center;">
            {move || {
                let m = if phase.get() >= 3 { "surge" } else if phase.get() >= 2 { "alive" } else { "active" };
                let i = 0.5 + phase.get() as f64 * 0.15;
                view! { <SentientOrb size=280 mode={m} intensity=i /> }
            }}

            <div style="margin-top: 28px; min-height: 200px; max-width: 520px;">
                <Show when=move || { phase.get() >= 1 }>
                    <FadeIn>
                        <div style="font-family: 'Orbitron', monospace; font-size: 12px; letter-spacing: 6px; color: rgba(255,60,20,0.6);">
                            "COGNITIVE ARCHITECTURE INITIALIZED"
                        </div>
                    </FadeIn>
                </Show>

                <Show when=move || { phase.get() >= 2 }>
                    <FadeIn delay_ms=400>
                        <div style="font-family: 'Orbitron', monospace; font-size: 38px; font-weight: 900; letter-spacing: 14px; color: rgba(255,245,240,0.9); margin-top: 14px;">
                            "ZEUS IS ALIVE"
                        </div>
                        <div style="font-size: 15px; color: rgba(255,245,240,0.7); margin-top: 12px; font-weight: 500;">
                            {move || {
                                let c = config.get();
                                format!("{} intelligence sources • {} communication channels • {} abilities",
                                    c.providers.len(), c.channels.len(), c.features.len())
                            }}
                        </div>
                    </FadeIn>
                </Show>

                <Show when=move || { phase.get() >= 3 }>
                    <FadeIn delay_ms=800 style="margin-top: 24px; font-size: 14px; color: rgba(255,245,240,0.7); line-height: 1.7;">
                        {move || {
                            let c = config.get();
                            let name = if c.user_name.is_empty() { "you".to_string() } else { c.user_name.clone() };
                            let role = if c.user_role.is_empty() { "developer".to_string() } else { c.user_role.clone() };
                            let org = if c.user_org.is_empty() { "the terminal".to_string() } else { c.user_org.clone() };
                            format!("Ready to serve {}, {} at {}.", name, role, org)
                        }}
                    </FadeIn>
                </Show>

                <Show when=move || { phase.get() >= 4 }>
                    <FadeIn delay_ms=600 style="margin-top: 32px;">
                        <div style="display: flex; gap: 12px; justify-content: center; flex-wrap: wrap;">
                            <button
                                class="onboarding-btn onboarding-btn-primary"
                                style=move || format!("padding: 14px 36px; font-size: 11px; opacity: {};",
                                    if save_status.get() == SaveStatus::Saving { "0.6" } else { "1" })
                                prop:disabled=move || save_status.get() == SaveStatus::Saving
                                on:click=move |_| {
                                    if save_status.get_untracked() == SaveStatus::Saving { return; }
                                    save_status.set(SaveStatus::Saving);
                                    save_detail.set("Saving configuration...".to_string());
                                    let c = config.get();
                                    spawn_local(async move {
                                        // Step 1: PUT /v1/config — save full config with provider JSON
                                        save_detail.set("Config...".to_string());
                                        let providers_json = {
                                            let mut provs = serde_json::Map::new();
                                            for pid in &c.providers {
                                                let mut entry = serde_json::Map::new();
                                                entry.insert("configured".to_string(), serde_json::Value::Bool(true));
                                                if pid == "ollama" {
                                                    entry.insert("url".to_string(), serde_json::Value::String(c.ollama_url.clone()));
                                                    // Find ollama models from default_model if applicable
                                                    if c.default_model.starts_with("ollama/") {
                                                        let model_name = c.default_model.strip_prefix("ollama/").unwrap_or("");
                                                        entry.insert("models".to_string(), serde_json::json!([model_name]));
                                                    }
                                                } else if let Some(key) = c.api_keys.get(pid)
                                                    && !key.trim().is_empty() {
                                                        entry.insert("api_key".to_string(), serde_json::Value::String(key.clone()));
                                                    }
                                                if c.default_model.starts_with(&format!("{}/", pid)) {
                                                    let model_name = c.default_model.split('/').nth(1).unwrap_or("");
                                                    entry.insert("model".to_string(), serde_json::Value::String(model_name.to_string()));
                                                }
                                                provs.insert(pid.clone(), serde_json::Value::Object(entry));
                                            }
                                            serde_json::Value::Object(provs)
                                        };
                                        let config_json = serde_json::json!({
                                            "name": c.agent_name,
                                            // Workspace/sessions/max_iterations: previously collected
                                            // but never persisted (#216). ConfigUpdateRequest accepts
                                            // all three (config_handlers.rs:20-23).
                                            "workspace": c.qs_workspace,
                                            "sessions": c.qs_sessions,
                                            "max_iterations": c.qs_max_iterations.parse::<usize>().unwrap_or(20),
                                            "user_name": c.user_name,
                                            "user_role": c.user_role,
                                            "user_org": c.user_org,
                                            "personality": c.personality,
                                            "model": c.default_model,
                                            "default_provider": c.providers.first().cloned().unwrap_or_default(),
                                            "providers": providers_json,
                                            "channels": c.channels,
                                            "security_level": c.security_level,
                                            "features": c.features,
                                            "communication_style": c.communication_style,
                                            "council_enabled": c.council_enabled,
                                            "ollama_url": c.ollama_url,
                                            "gateway_url": c.gateway_url,
                                            "gateway": {
                                                "host": c.qs_bind,
                                                "port": c.qs_port.parse::<u16>().unwrap_or(8080),
                                                "timeout_secs": c.gateway_timeout.parse::<u64>().unwrap_or(1800),
                                                "mentions_only": c.gateway_mentions_only,
                                            },
                                            "image_gen": {
                                                "provider": c.image_gen_provider,
                                                "url": c.image_gen_url,
                                                "api_key": c.image_gen_api_key,
                                            },
                                            "deployment": {
                                                "piper_tts_url": c.piper_url,
                                                "whisper_stt_url": c.whisper_url,
                                                "elevenlabs_api_key": c.elevenlabs_api_key,
                                                // Auto-select TTS provider: ElevenLabs wins over Piper (matches TUI logic)
                                                "tts_provider": if !c.elevenlabs_api_key.is_empty() { "elevenlabs" }
                                                                else if !c.piper_url.is_empty() { "piper" }
                                                                else { "" },
                                            },
                                            "video_gen": {
                                                "url": c.video_url,
                                            },
                                            "prometheus": {
                                                "enable_cognitive": c.orchestration_mode != "disable",
                                                "enable_heartbeat": c.orchestration_mode != "disable",
                                                "heartbeat_interval_secs": c.heartbeat_interval.parse::<u64>().unwrap_or(300),
                                                "max_plan_steps": 10,
                                            },
                                            "nous": {
                                                "enable_intent": c.nous_intent,
                                                "enable_learning": c.nous_learning,
                                            },
                                            "mnemosyne": {
                                                "db_path": if c.memory_db_path.trim().is_empty() { "~/.zeus/memory.db" } else { c.memory_db_path.trim() },
                                                "enable_fts": c.memory_fts,
                                                "enable_embeddings": c.memory_embeddings,
                                                "embedding_providers": if c.memory_embedding_provider == "none" || !c.memory_embeddings {
                                                    serde_json::json!([])
                                                } else {
                                                    serde_json::json!([c.memory_embedding_provider])
                                                },
                                            },
                                            "verbosity": c.verbosity,
                                            "fallback_models": serde_json::json!(c.fallback_models),
                                            "rate_limit": {
                                                "enabled": c.rate_limit_enabled,
                                                "global_rpm": 120,
                                                "llm_rpm": c.rate_limit_rpm.parse::<u32>().unwrap_or(20),
                                                "burst_size": 10,
                                            },
                                            "session_compaction": {
                                                "max_context_tokens": c.compaction_max_tokens.parse::<usize>().unwrap_or(180000),
                                                "compaction_threshold": c.compaction_threshold.parse::<f64>().unwrap_or(0.8),
                                            },
                                            // Feature-gated sections (TUI parity: toggles map to
                                            // real config sections — mod.rs:1974-1990).
                                            // athena/hermes deserialize into AthenaConfig/HermesConfig
                                            // via ConfigUpdateRequest (config_handlers.rs:255,267).
                                            "athena": if c.features.iter().any(|f| f == "athena") {
                                                serde_json::json!({})
                                            } else {
                                                serde_json::Value::Null
                                            },
                                            "hermes": if c.features.iter().any(|f| f == "hermes") {
                                                serde_json::json!({ "default_channels": [c.hermes_channel] })
                                            } else {
                                                serde_json::Value::Null
                                            },
                                        });
                                        if let Err(e) = api::save_config(&config_json).await {
                                            web_sys::console::warn_1(&format!("Zeus: save_config failed: {}", e).into());
                                            save_status.set(SaveStatus::Error(format!("Config save failed: {}", e)));
                                            return;
                                        }

                                        // Step 2: Store OAuth tokens via /v1/auth/token (OAuthManager) —
                                        // these are NOT api_keys and must not land in [credentials] as
                                        // env-var keys. Plain API keys + features + model + security +
                                        // name + completion all go through the single consolidated
                                        // POST /v1/onboarding/setup call below (#220 dual-path fix).
                                        save_detail.set("Credentials...".to_string());
                                        let mut plain_keys: std::collections::HashMap<String, String> = std::collections::HashMap::new();
                                        for (provider, key) in &c.api_keys {
                                            if key.trim().is_empty() { continue; }
                                            let is_oauth = c.auth_types.get(provider).map(|t| t == "oauth_token").unwrap_or(false)
                                                || key.starts_with("sk-ant-oat01-");
                                            if is_oauth {
                                                // OAuth token — store via /v1/auth/token (OAuthManager)
                                                if let Err(e) = api::auth_store_oauth_token(key).await {
                                                    web_sys::console::warn_1(&format!("Zeus: store OAuth token for {} failed: {}", provider, e).into());
                                                }
                                            }
                                            // #243: Add ALL non-empty keys (including OAuth) to plain_keys
                                            // so onboarding_setup saves them to config.credentials + vault + env vars.
                                            // Previously OAuth tokens were only stored via auth_store_oauth_token
                                            // but never reached onboarding_setup, causing "No API key configured" at chat time.
                                            plain_keys.insert(provider.clone(), key.clone());
                                        }

                                        // Step 3: Create channels with per-channel config
                                        save_detail.set("Channels...".to_string());
                                        for channel_id in &c.channels {
                                            // Build channel config from collected fields
                                            // Build config from per-channel credential fields
                                            let ch_config = if let Some(fields) = c.channel_creds.get(channel_id) {
                                                let get = |k: &str| fields.get(k).cloned().unwrap_or_default();
                                                match channel_id.as_str() {
                                                    "discord" => {
                                                        let bot_name = get("bot_name");
                                                        let acct_key = if bot_name.is_empty() { "default".to_string() } else { bot_name.clone() };
                                                        serde_json::json!({
                                                            "token": get("bot_token"),
                                                            "allow_bots": c.allow_bots_mode,
                                                            "accounts": {
                                                                acct_key: {
                                                                    "token": get("bot_token"),
                                                                    "guild_id": get("guild_id"),
                                                                    "channel_id": get("channel_id"),
                                                                    "personality": get("personality"),
                                                                }
                                                            }
                                                        })
                                                    }
                                                    "telegram" => serde_json::json!({
                                                        "token": get("bot_token"),
                                                    }),
                                                    "slack" => serde_json::json!({
                                                        "bot_token": get("bot_token"),
                                                        "app_token": get("app_token"),
                                                    }),
                                                    "whatsapp" => serde_json::json!({
                                                        "access_token": get("bot_token"),
                                                        "phone_number_id": get("phone_number_id"),
                                                    }),
                                                    "matrix" => serde_json::json!({
                                                        "homeserver": get("homeserver"),
                                                        "username": get("username"),
                                                        "password": get("password"),
                                                    }),
                                                    "mqtt" => serde_json::json!({
                                                        "broker_url": get("broker_url"),
                                                        "port": get("port").parse::<u16>().unwrap_or(1883),
                                                        "topic_prefix": get("topic_prefix"),
                                                        "client_id": get("client_id"),
                                                        "username": get("username"),
                                                        "password": get("password"),
                                                    }),
                                                    "mattermost" => serde_json::json!({
                                                        "server_url": get("server_url"),
                                                        "token": get("token"),
                                                        "team_id": get("team_id"),
                                                    }),
                                                    "x_twitter" => serde_json::json!({
                                                        "bearer_token": get("bearer_token"),
                                                        "api_key": get("api_key"),
                                                        "api_secret": get("api_secret"),
                                                        "access_token": get("access_token"),
                                                        "access_token_secret": get("access_token_secret"),
                                                    }),
                                                    "irc" => serde_json::json!({
                                                        "server": get("server"),
                                                        "port": get("port").parse::<u16>().unwrap_or(6667),
                                                        "nick": get("nick"),
                                                        "channels": get("channels").split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect::<Vec<&str>>(),
                                                        "use_tls": matches!(get("use_tls").to_lowercase().as_str(), "y" | "yes" | "true" | "1"),
                                                        "nickserv_password": get("nickserv_password"),
                                                    }),
                                                    "twitter" => serde_json::json!({
                                                        "api_key": get("api_key"),
                                                        "api_secret": get("api_secret"),
                                                        "access_token": get("access_token"),
                                                        "access_secret": get("access_secret"),
                                                        "bearer_token": get("bearer_token"),
                                                    }),
                                                    "twitch" => serde_json::json!({
                                                        "oauth_token": get("oauth_token"),
                                                        "username": get("username"),
                                                        "channels": get("channels").split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect::<Vec<&str>>(),
                                                        "client_id": get("client_id"),
                                                    }),
                                                    _ => serde_json::json!({}),
                                                }
                                            } else {
                                                serde_json::json!({})
                                            };
                                            let req = api::CreateChannelReq {
                                                channel_type: channel_id.clone(),
                                                name: channel_id.clone(),
                                                config: ch_config,
                                            };
                                            match api::create_channel(&req).await {
                                                Ok(_) => {},
                                                Err(e) => {
                                                    // Ignore 409 conflicts (channel already exists)
                                                    if !e.contains("409") && !e.contains("already exists") && !e.contains("conflict") {
                                                        web_sys::console::warn_1(&format!("Zeus: create channel {} failed: {}", channel_id, e).into());
                                                    }
                                                }
                                            }
                                        }

                                        // Step 4: Apply security permissions
                                        save_detail.set("Security...".to_string());
                                        let perms = match c.security_level.as_str() {
                                            "minimal" => api::GlobalPerms { shell_access: true, file_write: true, web_access: true, level: "minimal".to_string() },
                                            "standard" => api::GlobalPerms { shell_access: true, file_write: true, web_access: true, level: "standard".to_string() },
                                            "strict" => api::GlobalPerms { shell_access: false, file_write: false, web_access: false, level: "strict".to_string() },
                                            _ => api::GlobalPerms { shell_access: true, file_write: true, web_access: true, level: "standard".to_string() },
                                        };
                                        if let Err(e) = api::update_permissions(&perms).await {
                                            web_sys::console::warn_1(&format!("Zeus: update_permissions failed: {}", e).into());
                                        }

                                        // Step 5: Consolidated onboarding save (#220) — one POST
                                        // /v1/onboarding/setup persists everything atomically:
                                        // plain API keys → [credentials]+vault+env, model+provider,
                                        // security level → aegis.sandbox_level, feature toggles →
                                        // tui.disabled_tools, agent name, ollama URL, and
                                        // complete=true → onboarding_complete + workspace files
                                        // (SOUL.md, AGENTS.md, …) server-side. Replaces the old
                                        // dual path (separate /v1/credentials + feature-toggle +
                                        // /v1/onboarding/complete + trailing PUT /v1/config calls).
                                        save_detail.set("Onboarding...".to_string());
                                        let provider = c.providers.first().cloned().unwrap_or_default();
                                        let model = c.default_model.clone();
                                        let ollama_url = if c.ollama_url.is_empty() { None } else { Some(c.ollama_url.as_str()) };
                                        let feature_map: std::collections::HashMap<String, bool> = FEATURES
                                            .iter()
                                            .map(|f| (f.id.to_string(), c.features.iter().any(|x| x == f.id)))
                                            .collect();
                                        if let Err(e) = api::onboarding_setup(
                                            &provider,
                                            &model,
                                            &plain_keys,
                                            &c.security_level,
                                            &feature_map,
                                            &c.agent_name,
                                            ollama_url,
                                            true,
                                        ).await {
                                            web_sys::console::warn_1(&format!("Zeus: onboarding setup failed: {}", e).into());
                                            save_status.set(SaveStatus::Error(format!("Onboarding save failed: {}", e)));
                                            return;
                                        }

                                        // Step 6: localStorage fallback + show success
                                        if let Some(win) = web_sys::window()
                                            && let Ok(Some(storage)) = win.local_storage() {
                                                let _ = storage.set_item("zeus_onboarding_complete", "true");
                                                let _ = storage.set_item("zeus_gateway_url", &c.gateway_url);
                                            }
                                        // Step 7: Trigger gateway restart (fire-and-forget — don't block redirect)
                                        let port = c.qs_port.parse::<u16>().unwrap_or(8080);
                                        let bind = c.qs_bind.clone();
                                        let derived_url = {
                                            let host = if bind == "0.0.0.0" { "localhost" } else { &bind };
                                            format!("http://{}:{}", host, port)
                                        };
                                        // Fire gateway restart in background — don't await it
                                        let port_bg = port;
                                        let bind_bg = bind.clone();
                                        let gw_url_sig = gateway_display_url;
                                        let derived_url_bg = derived_url.clone();
                                        spawn_local(async move {
                                            match api::start_gateway(port_bg, &bind_bg).await {
                                                Ok(gw) if !gw.url.is_empty() => {
                                                    gw_url_sig.set(gw.url);
                                                }
                                                _ => {
                                                    gw_url_sig.set(derived_url_bg);
                                                }
                                            }
                                        });

                                        save_status.set(SaveStatus::Success);
                                        save_detail.set("Redirecting to dashboard...".to_string());

                                        // Redirect after 1.5s — use js eval so closure lifetime isn't an issue
                                        let _ = js_sys::eval(&format!(
                                            "setTimeout(function(){{ window.location.href = '/'; }}, 1500);"
                                        ));
                                    });
                                }
                            >
                                {move || match save_status.get() {
                                    SaveStatus::Saving => "Saving...",
                                    SaveStatus::Success => "Saved!",
                                    SaveStatus::Error(_) => "Retry",
                                    _ => "Launch Dashboard",
                                }}
                            </button>
                            <button
                                class="onboarding-btn"
                                style="background: rgba(255,255,255,0.03); border-color: rgba(255,60,20,0.1); color: rgba(255,245,240,0.7);"
                                on:click=move |_| show_config.update(|v| *v = !*v)
                            >
                                "View Config"
                            </button>
                            <button
                                class="onboarding-btn onboarding-btn-ghost"
                                style="font-size: 9px;"
                            >
                                "Open Terminal (TUI)"
                            </button>
                        </div>

                        // Save progress indicator
                        <Show when=move || save_status.get() != SaveStatus::Idle>
                            <div style=move || format!("margin-top: 16px; padding: 12px 18px; border-radius: 10px; font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 2px; text-align: center; transition: all 0.3s; background: {}; border: 1px solid {}; color: {};",
                                match save_status.get() {
                                    SaveStatus::Saving => "rgba(255,255,255,0.03)",
                                    SaveStatus::Success => "rgba(34,197,94,0.08)",
                                    SaveStatus::Error(_) => "rgba(239,68,68,0.08)",
                                    _ => "transparent",
                                },
                                match save_status.get() {
                                    SaveStatus::Saving => "rgba(255,60,20,0.15)",
                                    SaveStatus::Success => "rgba(34,197,94,0.2)",
                                    SaveStatus::Error(_) => "rgba(239,68,68,0.2)",
                                    _ => "transparent",
                                },
                                match save_status.get() {
                                    SaveStatus::Saving => "rgba(255,245,240,0.7)",
                                    SaveStatus::Success => "#22c55e",
                                    SaveStatus::Error(_) => "#ef4444",
                                    _ => "rgba(255,245,240,0.5)",
                                },
                            )>
                                {move || save_detail.get()}
                            </div>
                        </Show>

                        // Gateway URL banner (shown after successful start)
                        <Show when=move || !gateway_display_url.get().is_empty()>
                            <FadeIn delay_ms=200 style="margin-top: 14px;">
                                <div style="padding: 14px 20px; border-radius: 10px; background: rgba(34,197,94,0.07); border: 1px solid rgba(34,197,94,0.2); display: flex; align-items: center; gap: 14px; flex-wrap: wrap; justify-content: center;">
                                    <span style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 3px; color: rgba(34,197,94,0.7); font-weight: 700;">"GATEWAY ONLINE"</span>
                                    <a
                                        href={move || gateway_display_url.get()}
                                        target="_blank"
                                        style="font-family: 'JetBrains Mono', 'Orbitron', monospace; font-size: 13px; color: #22c55e; text-decoration: none; letter-spacing: 1px; font-weight: 600;"
                                    >
                                        {move || gateway_display_url.get()}
                                    </a>
                                    <span style="font-size: 11px; color: rgba(255,245,240,0.4);">"— click to open"</span>
                                </div>
                            </FadeIn>
                        </Show>

                        // Config summary (expandable)
                        <Show when=move || show_config.get()>
                            <FadeIn delay_ms=1500 style="margin-top: 24px; padding: 24px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 12px; text-align: left;">
                                <div style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 3px; color: rgba(255,245,240,0.7); margin-bottom: 16px; text-transform: uppercase; font-weight: 700;">
                                    "Configuration Summary"
                                </div>
                                {move || {
                                    let c = config.get();
                                    let items = vec![
                                        ("Identity", format!("{} — {} @ {}", c.user_name, c.user_role, c.user_org)),
                                        ("Personality", c.personality.clone()),
                                        ("Providers", format!("{} enabled", c.providers.len())),
                                        ("Default Model", c.default_model.clone()),
                                        ("Channels", format!("{} configured", c.channels.len())),
                                        ("Security", c.security_level.to_uppercase()),
                                        ("Features", format!("{} enabled", c.features.len())),
                                        ("Orchestration", c.orchestration_mode.to_uppercase()),
                                        ("Memory", format!("FTS:{} Vectors:{}", if c.memory_fts { "ON" } else { "OFF" }, if c.memory_embeddings { "ON" } else { "OFF" })),
                                    ];
                                    items.into_iter().map(|(label, value)| view! {
                                        <div style="display: flex; justify-content: space-between; padding: 10px 0; border-bottom: 1px solid rgba(255,60,20,0.1);">
                                            <span style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 2px; color: rgba(255,245,240,0.5); font-weight: 700;">{label}</span>
                                            <span style="font-size: 13px; color: rgba(255,245,240,0.9); font-weight: 600;">{value}</span>
                                        </div>
                                    }).collect::<Vec<_>>()
                                }}
                                <div style="margin-top: 16px; padding: 10px 14px; background: rgba(255,255,255,0.02); border-radius: 8px; font-family: 'Orbitron', monospace; font-size: 10px; color: rgba(255,245,240,0.5); letter-spacing: 1px;">
                                    "Saved to ~/.zeus/config.toml"
                                </div>
                            </FadeIn>
                        </Show>
                    </FadeIn>
                </Show>
            </div>
        </div>
    }
}
