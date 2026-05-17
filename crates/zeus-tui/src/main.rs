//! main.rs — Async event loop with gateway polling
//! Owner: mikes-Mac-mini (feat/s68-tui-core)

mod api;
mod app;
mod markdown;
mod markdown_stream;
mod office;
mod onboarding;
mod pantheon;
mod theme;
mod ui;
mod screens;

use app::{App, Agent, AgentStatus, Channel, ChannelStatus, ChatMessage, Role, Tab};
use api::ApiClient;
use crossterm::{
    event::{self, DisableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::prelude::*;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use chrono::Local;

#[tokio::main]
async fn main() -> io::Result<()> {
    // --continue flag: skip onboarding, resume last session with history loaded
    let resume_session = std::env::args().any(|a| a == "--continue");

    // --resume <id> flag: resume a specific session by ID
    let resume_specific = {
        let args: Vec<String> = std::env::args().collect();
        args.iter().position(|a| a == "--resume").and_then(|i| args.get(i + 1).cloned())
    };

    // --classic flag: launch classic numbered-menu onboarding immediately
    if std::env::args().any(|a| a == "--classic") {
        let stdout = io::stdout();
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = ratatui::Terminal::new(backend)?;
        enable_raw_mode()?;
        execute!(terminal.backend_mut(), EnterAlternateScreen, crossterm::event::EnableMouseCapture)?;
        run_onboarding(&mut terminal).await?;
        disable_raw_mode()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
        return Ok(());
    }

    let gateway_url = std::env::var("ZEUS_GATEWAY_URL")
        .unwrap_or_else(|_| "http://localhost:8080".to_string());

    let app = Arc::new(Mutex::new(App::new(&gateway_url)));

    // Background polling task — updates app state every 5s
    let poll_app = app.clone();
    let poll_url = gateway_url.clone();
    tokio::spawn(async move {
        let api = ApiClient::new(&poll_url);
        loop {
            let connected = api.health().await;
            if let Ok(mut a) = poll_app.lock() {
                if !connected && !a.connected {
                    a.reconnect_attempts += 1;
                } else if connected {
                    a.reconnect_attempts = 0;
                }
                a.connected = connected;
            }

            // Fetch config for Settings tab — works with or without gateway
            let needs_config = poll_app.lock().map(|a| a.settings_config.is_null()).unwrap_or(false);
            if needs_config {
                let mut loaded = false;
                if connected {
                    if let Ok(config) = api.config().await {
                        if let Ok(mut a) = poll_app.lock() {
                            a.settings_config = config;
                            loaded = true;
                        }
                    }
                }
                if !loaded {
                    // Always try disk fallback — even if connected but API failed
                    if let Ok(mut a) = poll_app.lock() {
                        if a.settings_config.is_null() {
                            if let Ok(config) = zeus_core::Config::load() {
                                if let Ok(val) = serde_json::to_value(&config) {
                                    a.settings_config = val;
                                    loaded = true;
                                }
                            }
                        }
                    }
                }
            }

            if connected {
                if let Ok(status) = api.status().await {
                    if let Ok(mut a) = poll_app.lock() {
                        a.model = status.model;
                        a.provider = status.provider;
                        a.tools_count = status.tools;
                        a.sessions_count = status.sessions_count;
                        a.auth_method = status.auth_method;
                        a.gateway_version = status.version;
                        if !status.agent_name.is_empty() {
                            a.self_name = status.agent_name;
                        }
                    }
                }
                if let Ok(agents) = api.agents().await {
                    if let Ok(mut a) = poll_app.lock() {
                        a.agents = agents.iter().map(|ag| Agent {
                            name: ag.name.clone(),
                            task: ag.current_task.clone().unwrap_or_else(|| "idle".into()),
                            status: match ag.status.as_str() {
                                "running" | "active" => AgentStatus::Running,
                                "error" => AgentStatus::Error,
                                "done" | "completed" => AgentStatus::Completed,
                                _ => AgentStatus::Idle,
                            },
                            progress: (ag.health_score * 100.0) as u16,
                            iterations: (0, 0),
                        }).collect();

                        // Sync Office agents from fleet data
                        office::state::sync_from_fleet(&mut a.office, &agents);
                    }
                }
                if let Ok(channels) = api.channels().await {
                    if let Ok(mut a) = poll_app.lock() {
                        a.channels = channels.into_iter().map(|c| Channel {
                            platform: c.channel_type.clone(),
                            icon: match c.channel_type.as_str() {
                                "discord" => "◈",
                                "telegram" => "◆",
                                "slack" => "▣",
                                _ => "●",
                            },
                            name: c.name,
                            status: match c.status.as_str() {
                                "connected" | "active" => ChannelStatus::Connected,
                                "relay" => ChannelStatus::Relay,
                                _ => ChannelStatus::Offline,
                            },
                            unread: 0,
                            last_msg: String::new(),
                        }).collect();
                    }
                }
                // Pantheon v2 — polling disabled (being rebuilt as standalone service)

                // Fetch memo for Office overlay (once, cached in state)
                let needs_memo = poll_app.lock().map(|a| a.office.memo_text.is_empty()).unwrap_or(false);
                if needs_memo {
                    let yesterday = (chrono::Local::now() - chrono::Duration::days(1))
                        .format("%Y-%m-%d").to_string();
                    if let Ok(content) = api.memory_file(&format!("daily/{}.md", yesterday)).await {
                        if let Ok(mut a) = poll_app.lock() {
                            a.office.memo_text = content.lines().map(|l| l.to_string()).collect();
                            a.office.memo_date = yesterday;
                        }
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });

    // Chat response channel
    let (chat_tx, mut chat_rx) = tokio::sync::mpsc::channel::<ChatMessage>(16);
    let chat_app = app.clone();
    tokio::spawn(async move {
        while let Some(msg) = chat_rx.recv().await {
            if let Ok(mut a) = chat_app.lock() {
                a.messages.push(msg);
            }
        }
    });

    // Terminal setup
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, crossterm::event::EnableMouseCapture)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;

    // Run onboarding if this is a first-time setup (no config present)
    if !resume_session && resume_specific.is_none() && needs_onboarding() {
        run_onboarding(&mut terminal).await?;
    }

    // --continue or --resume <id>: load session history into chat
    let target_session_id = if let Some(ref id) = resume_specific {
        Some(Ok(id.clone()))
    } else if resume_session {
        let api = ApiClient::new(&gateway_url);
        Some(api.default_session_id().await)
    } else {
        None
    };
    if let Some(session_result) = target_session_id {
        let api = ApiClient::new(&gateway_url);
        match session_result {
            Ok(session_id) => {
                if let Ok(history) = api.session_messages(&session_id).await {
                    if let Ok(mut a) = app.lock() {
                        a.messages.push(ChatMessage {
                            role: Role::System,
                            content: format!("↩ Resuming session {} ({} messages)", &session_id[..session_id.len().min(12)], history.len()),
                            timestamp: Local::now().format("%H:%M:%S").to_string(),
                            agent_name: Some("System".into()), streaming: false,
                            channel_source: Some("tui".into()),
                            stream_state: None,
                        });
                        let self_name = a.self_name.clone();
                        for msg in history {
                            let role = match msg.role.as_str() {
                                "user" => Role::User,
                                "assistant" => Role::Assistant,
                                _ => Role::System,
                            };
                            a.messages.push(ChatMessage {
                                role,
                                content: msg.content,
                                timestamp: msg.timestamp,
                                agent_name: match msg.role.as_str() {
                                    "user" => Some("You".into()),
                                    _ => Some(self_name.clone()),
                                },
                                streaming: false,
                                channel_source: Some("tui".into()),
                                stream_state: None,
                            });
                        }
                        a.scroll_to_bottom();
                    }
                }
            }
            Err(e) => {
                if let Ok(mut a) = app.lock() {
                    a.messages.push(ChatMessage {
                        role: Role::System,
                        content: format!("⚠ Could not resume session: {}", e),
                        timestamp: Local::now().format("%H:%M:%S").to_string(),
                        agent_name: Some("System".into()), streaming: false,
                        channel_source: Some("tui".into()),
                        stream_state: None,
                    });
                }
            }
        }
    } else {
        // Show welcome message + gateway status in chat
        if let Ok(mut a) = app.lock() {
            a.messages.push(ChatMessage {
                role: Role::System,
                content: format!("Zeus TUI — connecting to {}...\nType a message and press Enter to chat.", gateway_url),
                timestamp: Local::now().format("%H:%M:%S").to_string(),
                agent_name: Some("System".into()), streaming: false,
                channel_source: Some("tui".into()),
                stream_state: None,
            });
        }
    }

    let result = run_app(&mut terminal, &app, &gateway_url, chat_tx).await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    terminal.show_cursor()?;

    if let Err(err) = result { eprintln!("Error: {err}"); }
    Ok(())
}

async fn run_app<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &Arc<Mutex<App>>,
    gateway_url: &str,
    _chat_tx: tokio::sync::mpsc::Sender<ChatMessage>,
) -> io::Result<()>
where
    io::Error: From<<B as Backend>::Error>,
{
    let tick = Duration::from_millis(100);

    loop {
        {
            let mut a = app.lock().unwrap();
            terminal.draw(|f| ui::render(f, &mut a))?;
        }

        if event::poll(tick)? {
            let ev = event::read()?;
            // Mouse scroll support — only when capture is enabled
            if let Event::Mouse(mouse) = &ev {
                if app.lock().unwrap().mouse_capture_enabled {
                    match mouse.kind {
                        crossterm::event::MouseEventKind::ScrollUp => {
                            app.lock().unwrap().scroll_up(3);
                            continue;
                        }
                        crossterm::event::MouseEventKind::ScrollDown => {
                            app.lock().unwrap().scroll_down(3);
                            continue;
                        }
                        _ => {}
                    }
                    continue;
                }
                // mouse_capture_enabled == false: let mouse events pass through for text selection
            }
            if let Event::Key(key) = ev {
                // Global quit
                match (key.modifiers, key.code) {
                    (_, KeyCode::F(10)) | (KeyModifiers::CONTROL, KeyCode::Char('c')) => return Ok(()),
                    _ => {}
                }

                let mut a = app.lock().unwrap();

                // Tab key — cycle tabs: Chat → Office → Pantheon → Chat
                if key.code == KeyCode::Tab {
                    a.active_tab = match a.active_tab {
                        Tab::Chat => Tab::Office,
                        Tab::Office => Tab::Pantheon,
                        Tab::Pantheon => Tab::Settings,
                        Tab::Settings => Tab::Chat,
                    };
                    continue;
                }

                // Global: Shift+M toggles mouse capture (enables text selection)
                if key.modifiers == KeyModifiers::SHIFT && key.code == KeyCode::Char('M') {
                    a.mouse_capture_enabled = !a.mouse_capture_enabled;
                    continue;
                }

                // Office-specific keys
                if a.active_tab == Tab::Office {
                    match key.code {
                        KeyCode::Char('m') | KeyCode::Char('M') => { a.office.show_memo = !a.office.show_memo; }
                        KeyCode::Char('?') => { a.office.show_help = !a.office.show_help; }
                        KeyCode::Char('f') | KeyCode::Char('F') => { a.office.cycle_focus(); }
                        KeyCode::Char('r') | KeyCode::Char('R') => { a.office.connected = true; }
                        KeyCode::Esc => { a.office.clear_focus(); }
                        // Cross-nav: Enter on focused agent → switch to Pantheon
                        KeyCode::Enter => {
                            if a.office.focused_agent.is_some() {
                                a.active_tab = Tab::Pantheon;
                                a.pantheon_panel = app::PantheonPanel::Messages;
                            }
                        }
                        _ => {}
                    }
                    continue;
                }

                // Pantheon — keyboard input + IRC commands + plan card API effects
                if a.active_tab == Tab::Pantheon {
                    use crate::pantheon::{commands, input_bar};
                    use crate::pantheon::app::{IrcMessage, MessageKind};
                    use crate::pantheon::commands::PantheonApiEffect;

                    let mut pending_effect: Option<PantheonApiEffect> = None;

                    match key.code {
                        KeyCode::Char(c) => {
                            input_bar::type_char(&mut a.pantheon_irc, c);
                        }
                        KeyCode::Backspace => {
                            input_bar::backspace(&mut a.pantheon_irc);
                        }
                        KeyCode::Esc => {
                            input_bar::clear_input(&mut a.pantheon_irc);
                        }
                        KeyCode::Enter => {
                            if let Some(text) = input_bar::submit(&mut a.pantheon_irc) {
                                if let Some(cmd) = commands::parse(&text) {
                                    let (_, effect) = commands::dispatch(&mut a.pantheon_irc, cmd);
                                    pending_effect = effect;
                                } else {
                                    // Plain message — show locally
                                    let nick = a.pantheon_irc.nick.clone();
                                    let msg = IrcMessage {
                                        nick,
                                        content: text,
                                        kind: MessageKind::Normal,
                                        timestamp: chrono::Utc::now(),
                                    };
                                    if let Some(ch) = a.pantheon_irc.active_channel_mut() {
                                        ch.messages.push(msg);
                                    }
                                }
                            }
                        }
                        KeyCode::Left => {
                            if a.pantheon_irc.active_channel > 0 {
                                a.pantheon_irc.active_channel -= 1;
                            }
                        }
                        KeyCode::Right => {
                            let max = a.pantheon_irc.channels.len().saturating_sub(1);
                            if a.pantheon_irc.active_channel < max {
                                a.pantheon_irc.active_channel += 1;
                            }
                        }
                        _ => {}
                    }
                    drop(a); // release lock before spawning

                    if let Some(effect) = pending_effect {
                        let app_arc = Arc::clone(app);
                        let url = gateway_url.to_string();
                        tokio::spawn(async move {
                            let api = ApiClient::new(&url);
                            let result_msg = match effect {
                                PantheonApiEffect::ApprovePlan { plan_id } => {
                                    match api.pantheon_approve_plan(&plan_id).await {
                                        Ok(_) => format!("✓ Plan '{}' approved.", plan_id),
                                        Err(e) => format!("✗ approve failed: {}", e),
                                    }
                                }
                                PantheonApiEffect::RejectPlan { plan_id, reason } => {
                                    let r = if reason.is_empty() { "no reason given".to_string() } else { reason };
                                    match api.pantheon_reject_plan(&plan_id, &r).await {
                                        Ok(_) => format!("✓ Plan '{}' rejected: {}", plan_id, r),
                                        Err(e) => format!("✗ reject failed: {}", e),
                                    }
                                }
                                PantheonApiEffect::ListMissions => {
                                    match api.pantheon_missions().await {
                                        Ok(ms) if ms.is_empty() => "No active missions.".to_string(),
                                        Ok(ms) => ms.iter()
                                            .map(|m| format!("[{}] {} — {}", m.id, m.status, m.name))
                                            .collect::<Vec<_>>()
                                            .join(" | "),
                                        Err(e) => format!("✗ missions failed: {}", e),
                                    }
                                }
                            };
                            // Inject result back into Pantheon channel
                            if let Ok(mut a) = app_arc.lock() {
                                let msg = IrcMessage {
                                    nick: String::new(),
                                    content: result_msg,
                                    kind: MessageKind::System,
                                    timestamp: chrono::Utc::now(),
                                };
                                if let Some(ch) = a.pantheon_irc.active_channel_mut() {
                                    ch.messages.push(msg);
                                }
                            }
                        });
                    }

                    continue;
                }

                // Settings-specific keys — see lib.rs for full implementation
                // main.rs only handles the minimal standalone TUI case
                if a.active_tab == Tab::Settings {
                    let max_idx = crate::ui::settings_count().saturating_sub(1);
                    if a.settings_editing {
                        match key.code {
                            KeyCode::Esc => {
                                a.settings_editing = false;
                                a.settings_edit_value.clear();
                            }
                            KeyCode::Enter => {
                                let entries = crate::ui::settings_entry_paths();
                                let val = a.settings_edit_value.clone();
                                if let Some(path) = entries.get(a.settings_cursor) {
                                    a.settings_dirty.insert(path.to_string(), val);
                                }
                                a.settings_editing = false;
                                a.settings_edit_value.clear();
                            }
                            KeyCode::Backspace => {
                                if a.settings_edit_cursor > 0 {
                                    let byte_pos = a.settings_edit_value
                                        .char_indices()
                                        .nth(a.settings_edit_cursor - 1)
                                        .map(|(i, _)| i)
                                        .unwrap_or(0);
                                    let next_byte = a.settings_edit_value
                                        .char_indices()
                                        .nth(a.settings_edit_cursor)
                                        .map(|(i, _)| i)
                                        .unwrap_or(a.settings_edit_value.len());
                                    a.settings_edit_value.replace_range(byte_pos..next_byte, "");
                                    a.settings_edit_cursor -= 1;
                                }
                            }
                            KeyCode::Char(c) => {
                                let byte_pos = a.settings_edit_value
                                    .char_indices()
                                    .nth(a.settings_edit_cursor)
                                    .map(|(i, _)| i)
                                    .unwrap_or(a.settings_edit_value.len());
                                a.settings_edit_value.insert(byte_pos, c);
                                a.settings_edit_cursor += 1;
                            }
                            _ => {}
                        }
                        continue;
                    }
                    match key.code {
                        KeyCode::Up => {
                            a.settings_cursor = a.settings_cursor.saturating_sub(1);
                        }
                        KeyCode::Down => {
                            if a.settings_cursor < max_idx { a.settings_cursor += 1; }
                        }
                        KeyCode::Enter => {
                            let entries = crate::ui::settings_entry_paths();
                            let toggles = crate::ui::settings_entry_toggles();
                            if let Some(path) = entries.get(a.settings_cursor) {
                                let is_toggle = toggles.get(a.settings_cursor).copied().unwrap_or(false);
                                if is_toggle {
                                    let current = a.settings_dirty.get(*path).cloned()
                                        .unwrap_or_else(|| crate::ui::resolve_config_pub(&a.settings_config, path));
                                    let new_val = if current == "enabled" || current == "true" {
                                        "disabled".to_string()
                                    } else {
                                        "enabled".to_string()
                                    };
                                    a.settings_dirty.insert(path.to_string(), new_val);
                                } else {
                                    let current = a.settings_dirty.get(*path).cloned()
                                        .unwrap_or_else(|| {
                                            let v = crate::ui::resolve_config_pub(&a.settings_config, path);
                                            if v == "not set" { String::new() } else { v }
                                        });
                                    a.settings_edit_value = current;
                                    a.settings_edit_cursor = a.settings_edit_value.chars().count();
                                    a.settings_editing = true;
                                }
                            }
                        }
                        KeyCode::Char('r') | KeyCode::Char('R') => {
                            drop(a);
                            tokio::spawn(async move {
                                let _ = tokio::process::Command::new("zeus")
                                    .args(["daemon", "restart"])
                                    .output()
                                    .await;
                            });
                            continue;
                        }
                        KeyCode::Char('s') | KeyCode::Char('S') => {
                            if !a.settings_dirty.is_empty() {
                                let update = crate::ui::build_config_update(&a.settings_dirty);
                                // Merge updates into the live config and save
                                match zeus_core::Config::load() {
                                    Ok(mut config) => {
                                        // Apply each dirty field to the config struct
                                        if let Some(model) = update.get("model").and_then(|v| v.as_str()) {
                                            config.model = model.to_string();
                                        }
                                        if let Some(n) = update.get("max_iterations").and_then(|v| v.as_u64()) {
                                            config.max_iterations = n as usize;
                                        }
                                        if let Some(v) = update.get("verbosity").and_then(|v| v.as_str()) {
                                            config.verbosity = match v {
                                                "silent" => zeus_core::Verbosity::Silent,
                                                "barfly" => zeus_core::Verbosity::Barfly,
                                                _ => zeus_core::Verbosity::Normal,
                                            };
                                        }
                                        // Gateway sub-fields
                                        if let Some(gw_obj) = update.get("gateway").and_then(|v| v.as_object()) {
                                            let gw = config.gateway.get_or_insert_with(Default::default);
                                            for (key, val) in gw_obj {
                                                match key.as_str() {
                                                    "host" => if let Some(s) = val.as_str() { gw.host = s.to_string(); }
                                                    "port" => if let Some(n) = val.as_u64() { gw.port = n as u16; }
                                                    "enable_channels" => if let Some(b) = val.as_bool() { gw.enable_channels = b; }
                                                    "enable_heartbeat" => if let Some(b) = val.as_bool() { gw.enable_heartbeat = b; }
                                                    "enable_agent_processing" => if let Some(b) = val.as_bool() { gw.enable_agent_processing = b; }
                                                    "mentions_only" => if let Some(b) = val.as_bool() { gw.mentions_only = b; }
                                                    "timeout_secs" => if let Some(n) = val.as_u64() { gw.timeout_secs = n; }
                                                    _ => {}
                                                }
                                            }
                                        }
                                        match config.save() {
                                            Ok(()) => {
                                                a.settings_status = "Config saved successfully".to_string();
                                                a.settings_dirty.clear();
                                                a.settings_config = serde_json::to_value(&config).unwrap_or_default();
                                            }
                                            Err(e) => {
                                                a.settings_status = format!("Save failed: {}", e);
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        a.settings_status = format!("Failed to load config: {}", e);
                                    }
                                }
                            }
                        }
                        KeyCode::Char('c') | KeyCode::Char('C') => {
                            a.settings_dirty.clear();
                            a.settings_status.clear();
                        }
                        KeyCode::Esc => {
                            a.settings_dirty.clear();
                            a.settings_status.clear();
                            a.active_tab = Tab::Chat;
                        }
                        _ => {}
                    }
                    continue;
                }

                // Global ? toggle — keybinding overlay (works in Chat + Pantheon)
                if key.code == KeyCode::Char('?') && a.active_tab != Tab::Office {
                    a.show_keybind_overlay = !a.show_keybind_overlay;
                    continue;
                }
                // Esc dismisses overlay if open
                if key.code == KeyCode::Esc && a.show_keybind_overlay {
                    a.show_keybind_overlay = false;
                    continue;
                }

                // Chat-first: just type and go (like Claude Code)
                match key.code {
                    KeyCode::Enter => {
                        let msg = a.input.trim().to_string();
                        if !msg.is_empty() {
                            // Slash commands
                            if msg == "/clear" {
                                a.messages.clear();
                                a.clear_input();
                                // Push to input history
                            a.input_history.push(msg.clone());
                            a.input_history_idx = -1;
                            a.messages.push(ChatMessage {
                                    role: Role::System, content: "Session cleared.".into(),
                                    timestamp: Local::now().format("%H:%M:%S").to_string(),
                                    agent_name: Some("System".into()), streaming: false,
                                    channel_source: Some("tui".into()),
                                    stream_state: None,
                                });
                                let url = gateway_url.to_string();
                                drop(a);
                                tokio::spawn(async move {
                                    let api = ApiClient::new(&url);
                                    let _ = api.session_clear("agent:main:main").await;
                                });
                                continue;
                            }
                            if msg == "/compact" {
                                a.clear_input();
                                a.messages.push(ChatMessage {
                                    role: Role::System, content: "Compacting session...".into(),
                                    timestamp: Local::now().format("%H:%M:%S").to_string(),
                                    agent_name: Some("System".into()), streaming: false,
                                    channel_source: Some("tui".into()),
                                    stream_state: None,
                                });
                                let url = gateway_url.to_string();
                                let compact_app = app.clone();
                                drop(a);
                                tokio::spawn(async move {
                                    let api = ApiClient::new(&url);
                                    let result = api.session_compact("agent:main:main").await
                                        .unwrap_or_else(|e| format!("Error: {e}"));
                                    if let Ok(mut a) = compact_app.lock() {
                                        a.messages.push(ChatMessage {
                                            role: Role::System, content: result,
                                            timestamp: Local::now().format("%H:%M:%S").to_string(),
                                            agent_name: Some("System".into()), streaming: false,
                                            channel_source: Some("tui".into()),
                                            stream_state: None,
                                        });
                                    }
                                });
                                continue;
                            }

                            a.messages.push(ChatMessage {
                                role: Role::User,
                                content: msg.clone(),
                                timestamp: Local::now().format("%H:%M:%S").to_string(),
                                agent_name: Some("You".into()), streaming: false,
                                channel_source: Some("tui".into()),
                                stream_state: None,
                            });
                            a.clear_input();
                            // Snap to live bottom so streaming tokens are visible
                            a.scroll_to_bottom();
                            let url = gateway_url.to_string();
                            let stream_app = app.clone();
                            drop(a);
                            // Push streaming placeholder immediately
                            stream_app.lock().unwrap().begin_stream("Zeus");
                            tokio::spawn(async move {
                                let api = ApiClient::new(&url);
                                let mut first_token = true;

                                let result = api.chat_stream(&msg, |event: api::SseEvent| {
                                    if let Ok(mut a) = stream_app.lock() {
                                        match event {
                                            api::SseEvent::Token(token) => {
                                                if first_token {
                                                    a.set_first_token(&token);
                                                    first_token = false;
                                                } else {
                                                    a.append_token(&token);
                                                }
                                                a.scroll_to_bottom();
                                            }
                                            api::SseEvent::ToolStart { name, input } => {
                                                let summary = if input.len() > 60 { format!("{}…", zeus_core::truncate_str(&input, 60)) } else { input };
                                                a.push_tool_event(&name, &summary);
                                                a.finish_intermediate_streams();
                                                a.begin_stream("Zeus");
                                                first_token = true;
                                            }
                                            api::SseEvent::Iter(n) => { a.cooking_iter = n; }
                                            api::SseEvent::Thinking(text) => { a.set_thinking(&text); }
                                            api::SseEvent::ToolEnd { .. } => {}
                                            api::SseEvent::Usage { input, output } => { a.record_turn_tokens(input, output); }
                                        }
                                    }
                                }).await;
                                if let Ok(mut a) = stream_app.lock() {
                                    if result.is_err() {
                                        a.append_token(&format!("⚠ {}", result.unwrap_err()));
                                    }
                                    a.finish_stream();
                                    a.scroll_to_bottom();
                                }
                            });
                            continue;
                        }
                    }
                    // Scrolling while a response streams
                    KeyCode::PageUp => { a.scroll_up(10); }
                    KeyCode::PageDown => { a.scroll_down(10); }
                    KeyCode::Home if key.modifiers == crossterm::event::KeyModifiers::CONTROL => {
                        a.scroll_to_top();
                    }
                    KeyCode::End if key.modifiers == crossterm::event::KeyModifiers::CONTROL => {
                        a.scroll_to_bottom();
                    }
                    // Cursor-aware input editing
                    KeyCode::Up if a.input.is_empty() || a.input_history_idx >= 0 => {
                        if !a.input_history.is_empty() {
                            if a.input_history_idx < 0 {
                                a.input_history_idx = a.input_history.len() as isize - 1;
                            } else if a.input_history_idx > 0 {
                                a.input_history_idx -= 1;
                            }
                            if let Some(cmd) = a.input_history.get(a.input_history_idx as usize) {
                                a.input = cmd.clone();
                                a.cursor_pos = a.input.chars().count();
                            }
                        }
                    }
                    KeyCode::Down if a.input_history_idx >= 0 => {
                        a.input_history_idx += 1;
                        if a.input_history_idx >= a.input_history.len() as isize {
                            a.input_history_idx = -1;
                            a.input.clear();
                            a.cursor_pos = 0;
                        } else if let Some(cmd) = a.input_history.get(a.input_history_idx as usize) {
                            a.input = cmd.clone();
                            a.cursor_pos = a.input.chars().count();
                        }
                    }
                    KeyCode::Backspace => { a.delete_char_before(); }
                    KeyCode::Left  => { a.cursor_left(); }
                    KeyCode::Right => { a.cursor_right(); }
                    KeyCode::Home  => { a.cursor_home(); }
                    KeyCode::End   => { a.cursor_end(); }
                    KeyCode::Char(c) => { a.insert_char(c); }
                    _ => {}
                }
            }
        }

        {
            let mut a = app.lock().unwrap();
            a.tick();
            // Office logic tick at ~8 TPS (every other 100ms frame)
            if a.tick_count % 2 == 0 {
                a.office.tick();
                a.office.connected = a.connected;
            }
        }
    }
}

/// Returns true if Zeus config is missing or incomplete — triggers onboarding.
fn needs_onboarding() -> bool {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    let config_path = std::path::PathBuf::from(&home).join(".zeus").join("config.toml");
    !config_path.exists()
}

/// Run the onboarding state machine until complete or user quits.
async fn run_onboarding<B: Backend>(terminal: &mut ratatui::Terminal<B>) -> io::Result<()>
where
    io::Error: From<<B as Backend>::Error>,
{
    use onboarding::{OnboardingState, OnboardingStep};
    use crossterm::event::{Event, KeyCode};

    let mut state = OnboardingState::new();
    let tick = std::time::Duration::from_millis(100);

    // Model fetch channel (same as lib.rs — enables async model loading)
    let (model_tx, mut model_rx) = tokio::sync::mpsc::channel::<Result<Vec<String>, String>>(1);

    loop {
        // Check for model fetch results
        if state.models_fetching {
            if let Ok(result) = model_rx.try_recv() {
                state.models_fetching = false;
                match result {
                    Ok(models) => {
                        state.fetched_models = models;
                        state.selected_model = 0;
                    }
                    Err(e) => {
                        state.models_fetch_error = Some(e);
                    }
                }
            }
        }

        state.tick();
        terminal.draw(|f| onboarding::render_onboarding(f, &state))?;

        if event::poll(tick)? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    // Quit
                    KeyCode::Char('q') if state.step == OnboardingStep::Welcome => {
                        return Ok(());
                    }
                    KeyCode::F(10) => return Ok(()),

                    // Navigate back
                    KeyCode::Esc => { state.back(); }

                    // Tab — skip channels OR switch fields within a step
                    KeyCode::Tab => {
                        match state.step {
                            OnboardingStep::Channels => {
                                state.advance(); // Tab = skip channels
                            }
                            OnboardingStep::Agent => {
                                state.sel = 1 - state.sel;
                            }
                            _ => {}
                        }
                    }

                    // Navigate list items
                    KeyCode::Up => {
                        match state.step {
                            OnboardingStep::Provider => {
                                if state.selected_provider > 0 { state.selected_provider -= 1; }
                            }
                            OnboardingStep::Model => {
                                if state.selected_model > 0 { state.selected_model -= 1; }
                            }
                            OnboardingStep::Agent => {
                                if state.sel == 1 {
                                    if state.persona_item > 0 {
                                        state.persona_item -= 1;
                                    } else if state.persona_cat > 0 {
                                        state.persona_cat -= 1;
                                        state.persona_item = state.personas.get(state.persona_cat).map(|c| c.items.len()).unwrap_or(0).saturating_sub(1);
                                    }
                                }
                            }
                            OnboardingStep::Channels => {
                                if state.sel > 0 { state.sel -= 1; }
                            }
                            OnboardingStep::Security => {
                                if state.security_level > 0 { state.security_level -= 1; }
                            }
                            _ => {}
                        }
                    }
                    KeyCode::Down => {
                        match state.step {
                            OnboardingStep::Provider => {
                                if state.selected_provider + 1 < onboarding::PROVIDERS.len() {
                                    state.selected_provider += 1;
                                }
                            }
                            OnboardingStep::Model => {
                                if state.selected_model + 1 < state.current_models().len() {
                                    state.selected_model += 1;
                                }
                            }
                            OnboardingStep::Agent => {
                                if state.sel == 1 {
                                    let cat_len = state.personas.get(state.persona_cat).map(|c| c.items.len()).unwrap_or(0);
                                    if state.persona_item + 1 < cat_len {
                                        state.persona_item += 1;
                                    } else if state.persona_cat + 1 < state.personas.len() {
                                        state.persona_cat += 1;
                                        state.persona_item = 0;
                                    }
                                }
                            }
                            OnboardingStep::Channels => {
                                if state.sel + 1 < onboarding::CHANNELS.len() { state.sel += 1; }
                            }
                            OnboardingStep::Security => {
                                if state.security_level + 1 < onboarding::SECURITY_LEVELS.len() {
                                    state.security_level += 1;
                                }
                            }
                            _ => {}
                        }
                    }

                    // Text input
                    KeyCode::Backspace => {
                        match state.step {
                            OnboardingStep::Agent => {
                                if state.sel == 0 { state.agent_name.pop(); }
                            }
                            _ => {}
                        }
                    }
                    KeyCode::Char(c) => {
                        match state.step {
                            OnboardingStep::Agent => {
                                if state.sel == 0 { state.agent_name.push(c); }
                            }
                            OnboardingStep::ChanConfig => {
                                if c == 'b' || c == 'B' {
                                    // Cycle bot policy: all -> @mentioned -> off -> all
                                    state.cycle_allow_bots();
                                } else {
                                    state.type_char_in_field(c);
                                }
                            }
                            _ => { state.type_char_in_field(c); }
                        }
                    }

                    // Advance
                    KeyCode::Enter => {
                        // Trigger model fetch when leaving Auth step
                        if state.step == OnboardingStep::Auth {
                            let has_key = !state.api_key.trim().is_empty() || !state.oauth_token.trim().is_empty();
                            if has_key {
                                let provider_id = onboarding::PROVIDERS.get(state.selected_provider)
                                    .map(|p| p.provider_id).unwrap_or("anthropic");
                                let key = if !state.api_key.trim().is_empty() {
                                    state.api_key.clone()
                                } else {
                                    state.oauth_token.clone()
                                };
                                state.models_fetching = true;
                                state.fetched_models.clear();
                                let pid = provider_id.to_string();
                                let tx = model_tx.clone();
                                tokio::spawn(async move {
                                    let result = onboarding::fetch_models(&pid, &key).await;
                                    let _ = tx.send(result).await;
                                });
                            }
                        }
                        state.advance();
                    }

                    _ => {}
                }
            }
        }

        if state.complete {
            break;
        }
    }
    Ok(())
}
