//! Zeus TUI — Terminal interface
//!
//! Architecture: strict separation of concerns
//! - app.rs  — pure data model (no rendering)
//! - ui.rs   — pure rendering (no state mutation)
//! - theme.rs — color constants
//! - screens/ — individual screen renderers

pub mod api;
pub mod app;
pub mod markdown;
pub mod markdown_stream;
pub mod office;
pub mod onboarding;
pub mod theme;
pub mod ui;
pub mod diff_viewer;
pub mod pantheon;
pub mod screens;
#[cfg(test)]
mod chat_tests;

/// Install a panic hook that restores the terminal and logs the backtrace to a file.
/// This ensures panic messages are visible even when the TUI crashes with raw mode active.
fn install_panic_hook() {
    // Ensure backtraces are enabled by default
    if std::env::var("RUST_BACKTRACE").is_err() {
        // SAFETY: set_var is documented as safe to call from a panic hook
        // (runs before any other shutdown code)
        unsafe { std::env::set_var("RUST_BACKTRACE", "1"); }
    }

    std::panic::set_hook(Box::new(|panic_info| {
        // Try to restore terminal state so the panic message is visible
        let _ = crossterm::execute!(std::io::stderr(), crossterm::terminal::LeaveAlternateScreen);
        let _ = crossterm::terminal::disable_raw_mode();

        // Write backtrace to a timestamped log file
        let log_dir = dirs::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join(".zeus")
            .join("logs");
        let _ = std::fs::create_dir_all(&log_dir);
        let timestamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string();
        let log_path = log_dir.join(format!("tui-panic-{}.log", timestamp));

        let msg = format!(
            "Zeus TUI panic at {}\n{}\n",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
            panic_info
        );

        let _ = std::fs::write(&log_path, &msg);
        let _ = std::fs::write(&log_path, &format!("\n--- backtrace ---\n{:?}", std::backtrace::Backtrace::force_capture()));

        eprintln!("Zeus TUI panicked. Panic logged to: {}", log_path.display());
    }));
}

/// Resolve the gateway URL from config, env var, or default.
/// Precedence: config.gateway → ZEUS_GATEWAY_URL env var → http://localhost:8080
pub fn resolve_gateway_url(config: &zeus_core::Config) -> String {
    config.gateway.as_ref()
        .map(|g| {
            // 0.0.0.0 is a bind address, not a connect address — use localhost
            let host = if g.host == "0.0.0.0" { "127.0.0.1" } else { &g.host };
            format!("http://{}:{}", host, g.port)
        })
        .or_else(|| std::env::var("ZEUS_GATEWAY_URL").ok())
        .unwrap_or_else(|| "http://localhost:8080".to_string())
}

/// Run the TUI v2 — entry point called from the main zeus binary.
/// Takes a Config, extracts gateway URL, runs the async event loop.
pub async fn run(config: zeus_core::Config) -> anyhow::Result<()> {
    install_panic_hook();

    let gateway_url = resolve_gateway_url(&config);

    // Re-use the main.rs logic but as a library function
    let app_state = std::sync::Arc::new(std::sync::Mutex::new(app::App::new(&gateway_url)));
    // Set agent name from config immediately (don't wait for first status poll)
    let config_name = config.agent.as_ref().and_then(|a| a.name.as_deref())
        .or_else(|| config.name.as_deref())
        .or_else(|| config.network.as_ref().and_then(|n| n.agent_name.as_deref()))
        .unwrap_or("");
    if !config_name.is_empty() {
        if let Ok(mut a) = app_state.lock() {
            a.self_name = config_name.to_string();
        }
    }
    let _api_client = api::ApiClient::new(&gateway_url);

    // Background polling
    let poll_app = app_state.clone();
    let poll_url = gateway_url.clone();
    tokio::spawn(async move {
        let api = api::ApiClient::new(&poll_url);
        let mut last_msg_count: usize = 0;
        let session_id = "agent:main:main".to_string();
        loop {
            let connected = api.health().await;
            if let Ok(mut a) = poll_app.lock() { a.connected = connected; }

            // Load session history ONCE on first connect (not polling — polling caused token explosion)
            // Cross-channel messages appear here from the shared session.
            // Live updates come through /v1/chat responses, not session polling.
            if connected && last_msg_count == 0 {
                if let Ok(msgs) = api.session_messages(&session_id).await {
                    if !msgs.is_empty() {
                        if let Ok(mut a) = poll_app.lock() {
                            // Skip history if TUI already has live messages (prevents duplicates)
                            // Use normalized content hash (first 200 chars + length) to catch
                            // near-duplicates that differ only by whitespace/formatting.
                            let existing_keys: std::collections::HashSet<_> = a.messages.iter()
                                .map(|m| {
                                    let normalized = m.content.split_whitespace().collect::<Vec<_>>().join(" ");
                                    let prefix = &normalized[..normalized.len().min(200)];
                                    (prefix.to_string(), normalized.len())
                                })
                                .collect();
                            for m in msgs.iter().rev().take(30).rev() {
                                // Deduplicate: skip if we already have a message with the same content
                                let normalized = m.content.split_whitespace().collect::<Vec<_>>().join(" ");
                                let prefix = &normalized[..normalized.len().min(200)];
                                let key = (prefix.to_string(), normalized.len());
                                if existing_keys.contains(&key) {
                                    continue;
                                }
                                let role = match m.role.as_str() {
                                    "User" | "user" => app::Role::User,
                                    "Assistant" | "assistant" => app::Role::Assistant,
                                    "System" | "system" => app::Role::System,
                                    _ => app::Role::System,
                                };
                                let agent_name = m.channel_source.as_ref()
                                    .and_then(|cs| cs.sender_name.clone())
                                    .or_else(|| match role {
                                        app::Role::User => Some("You".to_string()),
                                        app::Role::Assistant => Some(a.self_name.clone()),
                                        _ => None,
                                    });
                                a.messages.push(app::ChatMessage {
                                    role,
                                    content: m.content.clone(),
                                    timestamp: m.timestamp.chars().take(8).collect(),
                                    agent_name,
                                    streaming: false,
                                    stream_state: None,
                                    channel_source: m.channel_source.as_ref().map(|cs| cs.channel_type.clone()),
                                });
                            }
                        }
                        last_msg_count = msgs.len();
                    }
                }
            }
            if connected {
                // Update agent name from gateway status (mirrors main.rs:100-111)
                if let Ok(status) = api.status().await {
                    if let Ok(mut a) = poll_app.lock() {
                        if !status.agent_name.is_empty() {
                            a.self_name = status.agent_name;
                        }
                        a.model = status.model;
                        a.provider = status.provider;
                        a.tools_count = status.tools;
                        a.sessions_count = status.sessions_count;
                        a.auth_method = status.auth_method;
                        a.gateway_version = status.version;
                    }
                }
                if let Ok(agents) = api.agents().await {
                    if let Ok(mut a) = poll_app.lock() {
                        // Update sidebar agent list
                        a.agents = agents.iter().map(|ag| app::Agent {
                            name: ag.name.clone(),
                            task: ag.current_task.clone().unwrap_or_default(),
                            status: if ag.status.starts_with("busy") { app::AgentStatus::Running } else { app::AgentStatus::Idle },
                            progress: (ag.health_score * 100.0) as u16,
                            iterations: (0, 0),
                        }).collect();

                        // Sync Office agents from fleet data
                        office::state::sync_from_fleet(&mut a.office, &agents);
                    }
                }
                if let Ok(channels) = api.channels().await {
                    if let Ok(mut a) = poll_app.lock() {
                        a.channels = channels.into_iter().map(|c| app::Channel {
                            platform: c.channel_type.clone(),
                            icon: match c.channel_type.as_str() { "discord" => "◈", "telegram" => "◆", "slack" => "▣", _ => "●" },
                            name: c.name,
                            status: if c.status == "connected" { app::ChannelStatus::Connected } else { app::ChannelStatus::Offline },
                            unread: 0, last_msg: String::new(),
                        }).collect();
                    }
                }
                // Pantheon v2 — polling disabled (being rebuilt as standalone service)

                // Fetch config for Settings tab (once per poll cycle)
                if let Ok(config) = api.config().await {
                    if let Ok(mut a) = poll_app.lock() {
                        a.settings_config = config;
                    }
                }

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
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    });

    // Chat response channel
    let (chat_tx, mut chat_rx) = tokio::sync::mpsc::channel::<app::ChatMessage>(16);
    let chat_app = app_state.clone();
    tokio::spawn(async move {
        while let Some(msg) = chat_rx.recv().await {
            if let Ok(mut a) = chat_app.lock() {
                // Replace the streaming placeholder with the final clean copy.
                //
                // Previous code checked only `messages.last().streaming` and
                // popped it. That races: if any non-streaming message (tool
                // event, verifier, etc.) is pushed between streaming completion
                // and this handler running, `last()` is no longer the
                // placeholder → pop misses → both the placeholder (with full
                // streamed content) AND the clean copy end up in the list →
                // the user sees the response twice.
                //
                // Fix: remove ALL streaming placeholders (there may be multiple
                // from tool iterations — each ToolStart pushes a new one).
                // Retain only non-streaming messages, then push the final copy.
                a.messages.retain(|m| !m.streaming);
                a.messages.push(msg);
                // Clear thinking text + cooking counters (matches finish_stream())
                a.thinking_text = None;
                a.cooking_iter = 0;
                a.cooking_tools = 0;
            }
        }
    });

    // Terminal
    use crossterm::{execute, terminal::{EnterAlternateScreen, LeaveAlternateScreen, enable_raw_mode, disable_raw_mode}};
    use ratatui::prelude::*;

    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    // Enable mouse capture for trackpad/mouse scroll support.
    // Toggle with Shift+M to disable capture (enables text selection).
    execute!(stdout, EnterAlternateScreen, crossterm::event::EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = ratatui::Terminal::new(backend)?;

    // Run onboarding if this is a first-time setup (no config present)
    if needs_onboarding() {
        let launch_choice = run_onboarding(&mut terminal).await?;
        // Launch gateway via daemon (launchd/systemd) so it survives terminal close.
        // `zeus daemon install` now auto-loads into launchd with KeepAlive=true.
        if launch_choice == 0 {
            let _ = std::process::Command::new("/usr/local/bin/zeus")
                .args(["daemon", "install"])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
            // Give gateway a moment to start
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        }
    }

    // Show welcome message
    if let Ok(mut a) = app_state.lock() {
        a.messages.push(app::ChatMessage {
            role: app::Role::System,
            content: format!("Zeus TUI — connected to {}\nType a message and press Enter.", gateway_url),
            timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
            agent_name: Some("System".into()),
            streaming: false,
            stream_state: None,
            channel_source: Some("tui".into()),
        });
    }

    // Event loop — chat-first, always in input mode (like Claude Code)
    loop {
        {
            let mut a = app_state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            terminal.draw(|f| ui::render(f, &mut a))?;
        }

        // Watchdog backstop: if cooking has been active for >300s and there
        // are pending inputs, force-clear cooking state and drain the queue.
        // This catches edge cases where the scopeguard didn't fire (e.g. the
        // tokio task was dropped without running drop handlers, or the lock
        // was poisoned and the guard couldn't acquire it).
        {
            let mut a = app_state.lock().unwrap_or_else(|p| p.into_inner());
            if a.cooking_cancel.is_some() {
                if let Some(started) = a.cooking_started_at {
                    if started.elapsed() > std::time::Duration::from_secs(300)
                        && !a.pending_inputs.is_empty()
                    {
                        a.cooking_cancel = None;
                        a.cooking_iter = 0;
                        a.cooking_tools = 0;
                        a.thinking_text = None;
                        a.cooking_started_at = None;
                        if let Some(next) = a.pending_inputs.pop_front() {
                            a.pending_drain = Some(next);
                        }
                    }
                }
            }
        }

        // T20: Drain queued message into the input + synthesize Enter so the
        // existing Enter handler runs unchanged. Only fires when the chat tab
        // is active and no stream is in flight (defensive — drain is only set
        // post-stream, but cheap to re-check).
        let synthetic_enter: Option<crossterm::event::Event> = {
            let mut a = app_state.lock().unwrap_or_else(|p| p.into_inner());
            if a.active_tab == app::Tab::Chat && a.cooking_cancel.is_none() {
                if let Some(next) = a.pending_drain.take() {
                    a.input = next;
                    a.cursor_pos = a.input.chars().count();
                    Some(crossterm::event::Event::Key(crossterm::event::KeyEvent::new(
                        crossterm::event::KeyCode::Enter,
                        crossterm::event::KeyModifiers::NONE,
                    )))
                } else { None }
            } else { None }
        };

        let ev = if let Some(syn) = synthetic_enter {
            syn
        } else if crossterm::event::poll(std::time::Duration::from_millis(100))? {
            crossterm::event::read()?
        } else {
            continue;
        };
        {
            let _force_block_below = (); // keep diff tight: re-enter event handling block below
            // Handle mouse scroll — only when capture is enabled
            if let crossterm::event::Event::Mouse(mouse) = ev {
                if app_state.lock().unwrap().mouse_capture_enabled {
                    match mouse.kind {
                        crossterm::event::MouseEventKind::ScrollUp => {
                            app_state.lock().unwrap().scroll_up(3);
                        }
                        crossterm::event::MouseEventKind::ScrollDown => {
                            app_state.lock().unwrap().scroll_down(3);
                        }
                        _ => {}
                    }
                    continue;
                }
                // mouse_capture_enabled == false: let mouse events pass through for text selection
            }
            if let crossterm::event::Event::Key(key) = ev {
                match (key.modifiers, key.code) {
                    (_, crossterm::event::KeyCode::F(10)) => break,
                    // Ctrl+C: 3-strike system — cancel → warn → exit
                    (crossterm::event::KeyModifiers::CONTROL, crossterm::event::KeyCode::Char('c')) => {
                        if let Ok(mut a) = app_state.lock() {
                            let now = std::time::Instant::now();
                            // Reset counter if >3 seconds since last Ctrl+C
                            if let Some(last) = a.ctrl_c_last {
                                if now.duration_since(last).as_secs() > 3 {
                                    a.ctrl_c_count = 0;
                                }
                            }
                            a.ctrl_c_count += 1;
                            a.ctrl_c_last = Some(now);

                            match a.ctrl_c_count {
                                1 => {
                                    // Cancel streaming or clear input
                                    let is_streaming = a.messages.last().map(|m| m.streaming).unwrap_or(false);
                                    if is_streaming {
                                        a.stream_cancelled = true;
                                        // Trip the CancellationToken — this aborts the
                                        // in-flight chat_stream HTTP request, not just the
                                        // event callback.
                                        if let Some(token) = a.cooking_cancel.take() {
                                            token.cancel();
                                        }
                                        if let Some(last) = a.messages.last_mut() {
                                            last.streaming = false;
                                            if last.content == "thinking..." || last.content.is_empty() {
                                                last.content = "[cancelled]".to_string();
                                            } else {
                                                last.content.push_str("\n\n[cancelled by user]");
                                            }
                                        }
                                    } else if !a.input.is_empty() {
                                        a.input.clear();
                                        a.cursor_pos = 0;
                                    }
                                }
                                2 => {
                                    // Show warning
                                    a.messages.push(app::ChatMessage {
                                        role: app::Role::System,
                                        content: "Press Ctrl+C again to quit.".to_string(),
                                        timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                                        agent_name: Some("System".into()),
                                        streaming: false,
                                        channel_source: None,
                                        stream_state: None,
                                    });
                                }
                                _ => break, // 3rd+ Ctrl+C: exit
                            }
                        } else {
                            break; // Lock poisoned — exit
                        }
                    }
                    // T20: Ctrl+K = clear pending input queue (does NOT cancel current stream)
                    (crossterm::event::KeyModifiers::CONTROL, crossterm::event::KeyCode::Char('k')) => {
                        if let Ok(mut a) = app_state.lock() {
                            if a.active_tab == app::Tab::Chat && !a.pending_inputs.is_empty() {
                                let cleared = a.pending_inputs.len();
                                a.pending_inputs.clear();
                                a.messages.push(app::ChatMessage {
                                    role: app::Role::System,
                                    content: format!("🧹 Cleared {} queued message(s)", cleared),
                                    timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                                    agent_name: Some("System".into()),
                                    streaming: false,
                                    channel_source: Some("tui".into()),
                                    stream_state: None,
                                });
                                continue;
                            }
                        }
                    }
                    // F5 or Cmd+R = manual refresh — reload session messages from all channels
                    (_, crossterm::event::KeyCode::F(5)) |
                    (crossterm::event::KeyModifiers::SUPER, crossterm::event::KeyCode::Char('r')) => {
                        let url = gateway_url.clone();
                        let refresh_app = app_state.clone();
                        tokio::spawn(async move {
                            let api = api::ApiClient::new(&url);
                            if let Ok(msgs) = api.session_messages("agent:main:main").await {
                                if let Ok(mut a) = refresh_app.lock() {
                                    // Clear and reload all messages
                                    a.messages.clear();
                                    for m in msgs.iter().rev().take(50).rev() {
                                        let role = match m.role.as_str() {
                                            "User" | "user" => app::Role::User,
                                            "Assistant" | "assistant" => app::Role::Assistant,
                                            "System" | "system" => app::Role::System,
                                            _ => app::Role::System,
                                        };
                                        let agent_name = m.channel_source.as_ref()
                                            .and_then(|cs| cs.sender_name.clone())
                                            .or_else(|| match role {
                                                app::Role::User => Some("You".to_string()),
                                                app::Role::Assistant => Some(a.self_name.clone()),
                                                _ => None,
                                            });
                                        a.messages.push(app::ChatMessage {
                                            role,
                                            content: m.content.clone(),
                                            timestamp: m.timestamp.chars().take(8).collect(),
                                            agent_name,
                                            streaming: false,
                                            stream_state: None,
                                            channel_source: m.channel_source.as_ref().map(|cs| cs.channel_type.clone()),
                                        });
                                    }
                                }
                            }
                        });
                        continue;
                    }
                    _ => {}
                }

                let mut a = app_state.lock().unwrap();

                // Tab key cycles between tabs: Chat → Office → Pantheon → Chat
                if key.code == crossterm::event::KeyCode::Tab && key.modifiers == crossterm::event::KeyModifiers::NONE {
                    a.active_tab = match a.active_tab {
                        app::Tab::Chat => app::Tab::Office,
                        app::Tab::Office => app::Tab::Pantheon,
                        app::Tab::Pantheon => app::Tab::Settings,
                        app::Tab::Settings => app::Tab::Chat,
                    };
                    continue;
                }

                // Global: Shift+M toggles mouse capture (enables text selection)
                if key.modifiers == crossterm::event::KeyModifiers::SHIFT
                    && key.code == crossterm::event::KeyCode::Char('M')
                {
                    a.mouse_capture_enabled = !a.mouse_capture_enabled;
                    continue;
                }

                // Office-specific keys
                if a.active_tab == app::Tab::Office {
                    match (key.modifiers, key.code) {
                        (crossterm::event::KeyModifiers::NONE, crossterm::event::KeyCode::Char('m') | crossterm::event::KeyCode::Char('M')) => {
                            a.office.show_memo = !a.office.show_memo;
                        }
                        (crossterm::event::KeyModifiers::NONE, crossterm::event::KeyCode::Char('?')) => {
                            a.office.show_help = !a.office.show_help;
                        }
                        (crossterm::event::KeyModifiers::NONE, crossterm::event::KeyCode::Char('f') | crossterm::event::KeyCode::Char('F')) => {
                            a.office.cycle_focus();
                        }
                        (crossterm::event::KeyModifiers::NONE, crossterm::event::KeyCode::Char('r') | crossterm::event::KeyCode::Char('R')) => {
                            a.office.connected = true;
                        }
                        (crossterm::event::KeyModifiers::NONE, crossterm::event::KeyCode::Esc) => {
                            a.office.clear_focus();
                        }
                        // Cross-nav: Enter on focused agent → switch to Pantheon
                        (crossterm::event::KeyModifiers::NONE, crossterm::event::KeyCode::Enter) => {
                            if let Some(idx) = a.office.focused_agent {
                                if let Some(agent) = a.office.agents.get(idx) {
                                    let agent_name = agent.name.clone();
                                    a.active_tab = app::Tab::Pantheon;
                                    a.pantheon_panel = app::PantheonPanel::Messages;
                                    a.pantheon_dm_target = Some(agent_name);
                                }
                            }
                        }
                        _ => {}
                    }
                    continue;
                }

                // Pantheon IRC keys
                if a.active_tab == app::Tab::Pantheon {
                    use crossterm::event::{KeyCode, KeyModifiers};
                    match (key.modifiers, key.code) {
                        // Channel navigation
                        (KeyModifiers::NONE, KeyCode::Up) | (KeyModifiers::NONE, KeyCode::Char('k')) => {
                            if a.pantheon_irc.active_channel > 0 {
                                a.pantheon_irc.active_channel -= 1;
                            }
                        }
                        (KeyModifiers::NONE, KeyCode::Down) | (KeyModifiers::NONE, KeyCode::Char('j')) => {
                            if a.pantheon_irc.active_channel + 1 < a.pantheon_irc.channels.len() {
                                a.pantheon_irc.active_channel += 1;
                            }
                        }
                        // Clear unread on channel focus change
                        (KeyModifiers::NONE, KeyCode::Enter) if a.pantheon_irc.input.is_empty() => {
                            let idx = a.pantheon_irc.active_channel;
                            if let Some(ch) = a.pantheon_irc.channels.get_mut(idx) {
                                ch.unread = 0;
                            }
                        }
                        // Text input
                        (KeyModifiers::NONE, KeyCode::Char(c)) => {
                            a.pantheon_irc.input.push(c);
                        }
                        (KeyModifiers::NONE, KeyCode::Backspace) => {
                            a.pantheon_irc.input.pop();
                        }
                        (KeyModifiers::NONE, KeyCode::Enter) => {
                            // Send message — gateway connection will be wired when pantheon-server is running
                            let msg_text = a.pantheon_irc.input.trim().to_string();
                            if !msg_text.is_empty() {
                                let nick = a.pantheon_irc.nick.clone();
                                let idx = a.pantheon_irc.active_channel;
                                if let Some(ch) = a.pantheon_irc.channels.get_mut(idx) {
                                    ch.messages.push(crate::pantheon::app::IrcMessage {
                                        nick: nick.clone(),
                                        content: msg_text,
                                        kind: crate::pantheon::app::MessageKind::Normal,
                                        timestamp: chrono::Utc::now(),
                                    });
                                }
                                a.pantheon_irc.input.clear();
                            }
                        }
                        (KeyModifiers::NONE, KeyCode::Esc) => {
                            a.active_tab = app::Tab::Chat;
                        }
                        _ => {}
                    }
                    continue;
                }
                // Settings-specific keys
                if a.active_tab == app::Tab::Settings {
                    use crossterm::event::{KeyCode, KeyModifiers};
                    let max_idx = crate::ui::settings_count().saturating_sub(1);

                    // Edit mode intercepts all keys
                    if a.settings_editing {
                        match key.code {
                            KeyCode::Esc => {
                                a.settings_editing = false;
                                a.settings_edit_value.clear();
                                a.settings_edit_cursor = 0;
                            }
                            KeyCode::Enter => {
                                // Commit edit to dirty map
                                let entries = crate::ui::settings_entry_paths();
                                let val = a.settings_edit_value.clone();
                                if let Some(path) = entries.get(a.settings_cursor) {
                                    a.settings_dirty.insert(path.to_string(), val);
                                }
                                a.settings_editing = false;
                                a.settings_edit_value.clear();
                                a.settings_edit_cursor = 0;
                                a.settings_status = " Unsaved changes (S to save, C to discard)".into();
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
                            KeyCode::Left => {
                                a.settings_edit_cursor = a.settings_edit_cursor.saturating_sub(1);
                            }
                            KeyCode::Right => {
                                let char_count = a.settings_edit_value.chars().count();
                                if a.settings_edit_cursor < char_count {
                                    a.settings_edit_cursor += 1;
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

                    // Normal mode
                    match (key.modifiers, key.code) {
                        (KeyModifiers::NONE, KeyCode::Up) | (KeyModifiers::NONE, KeyCode::Char('k')) => {
                            a.settings_cursor = a.settings_cursor.saturating_sub(1);
                        }
                        (KeyModifiers::NONE, KeyCode::Down) | (KeyModifiers::NONE, KeyCode::Char('j')) => {
                            if a.settings_cursor < max_idx { a.settings_cursor += 1; }
                        }
                        (KeyModifiers::NONE, KeyCode::Enter) => {
                            // Start editing current field
                            let entries = crate::ui::settings_entry_paths();
                            let toggles = crate::ui::settings_entry_toggles();
                            let selects = crate::ui::settings_entry_selects();
                            if let Some(path) = entries.get(a.settings_cursor) {
                                let is_toggle = toggles.get(a.settings_cursor).copied().unwrap_or(false);
                                let select_opts = selects.get(a.settings_cursor).copied().unwrap_or(&[]);
                                if is_toggle {
                                    // Toggle: flip the value immediately
                                    let current = a.settings_dirty.get(*path).cloned()
                                        .unwrap_or_else(|| crate::ui::resolve_config_pub(&a.settings_config, path));
                                    let new_val = if current == "enabled" || current == "true" {
                                        "disabled".to_string()
                                    } else {
                                        "enabled".to_string()
                                    };
                                    a.settings_dirty.insert(path.to_string(), new_val);
                                    a.settings_status = " Unsaved changes (S to save, C to discard)".into();
                                } else if !select_opts.is_empty() {
                                    // Select: cycle to next option
                                    let current = a.settings_dirty.get(*path).cloned()
                                        .unwrap_or_else(|| crate::ui::resolve_config_pub(&a.settings_config, path));
                                    let idx = select_opts.iter().position(|&o| o == current).unwrap_or(0);
                                    let next = select_opts[(idx + 1) % select_opts.len()];
                                    a.settings_dirty.insert(path.to_string(), next.to_string());
                                    a.settings_status = " Unsaved changes (S to save, C to discard)".into();
                                } else {
                                    // Text: enter edit mode with current value
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
                        (KeyModifiers::NONE, KeyCode::Char('s') | KeyCode::Char('S')) => {
                            // Save config via API
                            if !a.settings_dirty.is_empty() {
                                let dirty = a.settings_dirty.clone();
                                let url = a.gateway_url.clone();
                                a.settings_status = " Saving...".into();
                                drop(a);
                                let save_app = app_state.clone();
                                tokio::spawn(async move {
                                    let api = crate::api::ApiClient::new(&url);
                                    let updates = crate::ui::build_config_update(&dirty);
                                    match api.update_config(&updates).await {
                                        Ok(_) => {
                                            if let Ok(mut a) = save_app.lock() {
                                                a.settings_dirty.clear();
                                                a.settings_status = " Saved successfully".into();
                                                // Refetch config
                                            }
                                            // Refetch config after save
                                            if let Ok(config) = api.config().await {
                                                if let Ok(mut a) = save_app.lock() {
                                                    a.settings_config = config;
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            if let Ok(mut a) = save_app.lock() {
                                                a.settings_status = format!(" Error: {}", e);
                                            }
                                        }
                                    }
                                });
                                continue;
                            }
                        }
                        (KeyModifiers::NONE, KeyCode::Char('r') | KeyCode::Char('R')) => {
                            // Restart gateway
                            a.settings_status = " Restarting gateway...".into();
                            drop(a);
                            tokio::spawn(async move {
                                let _ = tokio::process::Command::new("zeus")
                                    .args(["daemon", "restart"])
                                    .output()
                                    .await;
                            });
                            continue;
                        }
                        (KeyModifiers::NONE, KeyCode::Char('c') | KeyCode::Char('C')) => {
                            // Cancel — discard pending edits
                            a.settings_dirty.clear();
                            a.settings_status.clear();
                        }
                        (KeyModifiers::NONE, KeyCode::Esc) => {
                            if a.settings_dirty.is_empty() {
                                a.active_tab = app::Tab::Chat;
                            } else {
                                // Discard dirty edits then go back
                                a.settings_dirty.clear();
                                a.settings_status.clear();
                                a.active_tab = app::Tab::Chat;
                            }
                        }
                        _ => {}
                    }
                    continue;
                }

                // Chat-specific keys
                // S100 #13: Search mode intercepts all keys when active
                if a.search_active {
                    match (key.modifiers, key.code) {
                        (crossterm::event::KeyModifiers::NONE, crossterm::event::KeyCode::Esc) => {
                            a.search_active = false;
                            a.search_query.clear();
                            a.search_cursor = 0;
                            a.search_matches.clear();
                            a.search_match_idx = 0;
                        }
                        (crossterm::event::KeyModifiers::NONE, crossterm::event::KeyCode::Enter) => {
                            a.search_next();
                        }
                        (crossterm::event::KeyModifiers::NONE, crossterm::event::KeyCode::Char('n')) => {
                            a.search_next();
                        }
                        (crossterm::event::KeyModifiers::SHIFT, crossterm::event::KeyCode::Char('N')) |
                        (crossterm::event::KeyModifiers::NONE, crossterm::event::KeyCode::Char('N')) => {
                            a.search_prev();
                        }
                        (crossterm::event::KeyModifiers::NONE, crossterm::event::KeyCode::Backspace) => {
                            if a.search_cursor > 0 {
                                let byte_idx = a.search_query.char_indices()
                                    .nth(a.search_cursor - 1)
                                    .map(|(b, _)| b)
                                    .unwrap_or(a.search_query.len());
                                a.search_query.remove(byte_idx);
                                a.search_cursor -= 1;
                                a.update_search_matches();
                            }
                        }
                        (crossterm::event::KeyModifiers::NONE, crossterm::event::KeyCode::Left) => {
                            if a.search_cursor > 0 { a.search_cursor -= 1; }
                        }
                        (crossterm::event::KeyModifiers::NONE, crossterm::event::KeyCode::Right) => {
                            let len = a.search_query.chars().count();
                            if a.search_cursor < len { a.search_cursor += 1; }
                        }
                        (crossterm::event::KeyModifiers::NONE, crossterm::event::KeyCode::Char(c)) => {
                            let byte_idx = a.search_query.char_indices()
                                .nth(a.search_cursor)
                                .map(|(b, _)| b)
                                .unwrap_or(a.search_query.len());
                            a.search_query.insert(byte_idx, c);
                            a.search_cursor += 1;
                            a.update_search_matches();
                        }
                        _ => {}
                    }
                    continue;
                }

                match (key.modifiers, key.code) {
                    // Escape: cancel streaming, or jump to bottom if scrolled up
                    (crossterm::event::KeyModifiers::NONE, crossterm::event::KeyCode::Esc) => {
                        let is_streaming = a.messages.last().map(|m| m.streaming).unwrap_or(false);
                        if is_streaming {
                            a.stream_cancelled = true;
                            // Trip the CancellationToken — aborts the in-flight HTTP
                            // request so cooking actually stops, not just the UI updates.
                            if let Some(token) = a.cooking_cancel.take() {
                                token.cancel();
                            }
                            if let Some(last) = a.messages.last_mut() {
                                last.streaming = false;
                                if last.content == "thinking..." || last.content.is_empty() {
                                    last.content = "[cancelled]".to_string();
                                } else {
                                    last.content.push_str("\n\n[cancelled by user]");
                                }
                            }
                        } else if a.scroll_offset > 0 {
                            // Not streaming + scrolled up → jump back to bottom
                            a.scroll_to_bottom();
                        }
                    }
                    // S100 #13: Ctrl+F toggles search mode
                    (crossterm::event::KeyModifiers::CONTROL, crossterm::event::KeyCode::Char('f')) => {
                        a.search_active = true;
                        a.search_query.clear();
                        a.search_cursor = 0;
                        a.search_matches.clear();
                        a.search_match_idx = 0;
                    }
                    (crossterm::event::KeyModifiers::CONTROL, crossterm::event::KeyCode::Char('y')) => {
                        a.copy_selected_to_clipboard();
                    }
                    (crossterm::event::KeyModifiers::NONE, crossterm::event::KeyCode::Up) => {
                        a.scroll_up(3);
                    }
                    (crossterm::event::KeyModifiers::NONE, crossterm::event::KeyCode::Down) => {
                        a.scroll_down(3);
                    }
                    // Track A5: scroll keys
                    (crossterm::event::KeyModifiers::NONE, crossterm::event::KeyCode::PageUp) => {
                        a.scroll_up(10);
                    }
                    (crossterm::event::KeyModifiers::NONE, crossterm::event::KeyCode::PageDown) => {
                        a.scroll_down(10);
                    }
                    (crossterm::event::KeyModifiers::NONE, crossterm::event::KeyCode::Home) => {
                        a.scroll_to_top();
                    }
                    (crossterm::event::KeyModifiers::NONE, crossterm::event::KeyCode::End) => {
                        a.scroll_to_bottom();
                    }
                    _ => {}
                }
                // S100 #15: Shift+Enter inserts newline instead of submitting
                if key.code == crossterm::event::KeyCode::Enter
                    && key.modifiers.contains(crossterm::event::KeyModifiers::SHIFT)
                {
                    a.insert_newline();
                    continue;
                }

                match key.code {
                    crossterm::event::KeyCode::Enter => {
                        let msg = a.input.trim().to_string();
                        if !msg.is_empty() {
                            // Slash commands: /copy, /clear, /compact
                            if msg == "/copy" || msg.starts_with("/copy ") {
                                let arg = msg.strip_prefix("/copy").unwrap_or("").trim();
                                a.clear_input();
                                // /copy → last assistant message, /copy N → Nth-to-last
                                let n: usize = arg.parse().unwrap_or(1);
                                let assistant_msgs: Vec<(usize, &app::ChatMessage)> = a.messages.iter()
                                    .enumerate()
                                    .filter(|(_, m)| m.role == app::Role::Assistant)
                                    .collect();
                                if let Some(&(idx, _)) = assistant_msgs.iter().rev().nth(n.saturating_sub(1)) {
                                    a.selected_message = Some(idx);
                                    if let Some(content) = a.copy_selected_to_clipboard() {
                                        let preview = if content.len() > 80 {
                                            format!("{}...", &content[..80])
                                        } else {
                                            content.clone()
                                        };
                                        a.messages.push(app::ChatMessage {
                                            role: app::Role::System,
                                            content: format!("Copied to clipboard ({} chars): {}", content.len(), preview),
                                            timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                                            agent_name: Some("System".into()), streaming: false,
                                            channel_source: Some("tui".into()),
                                            stream_state: None,
                                        });
                                    }
                                } else {
                                    a.messages.push(app::ChatMessage {
                                        role: app::Role::System,
                                        content: "No assistant message to copy.".into(),
                                        timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                                        agent_name: Some("System".into()), streaming: false,
                                        channel_source: Some("tui".into()),
                                        stream_state: None,
                                    });
                                }
                                continue;
                            }
                            if msg == "/clear" {
                                a.messages.clear();
                                a.clear_input();
                                a.messages.push(app::ChatMessage {
                                    role: app::Role::System,
                                    content: "Session cleared.".into(),
                                    timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                                    agent_name: Some("System".into()), streaming: false,
                                    channel_source: Some("tui".into()),
                                    stream_state: None,
                                });
                                let url = gateway_url.clone();
                                drop(a);
                                tokio::spawn(async move {
                                    let api = api::ApiClient::new(&url);
                                    let _ = api.session_clear("agent:main:main").await;
                                });
                                continue;
                            }
                            if msg == "/compact" {
                                a.clear_input();
                                a.messages.push(app::ChatMessage {
                                    role: app::Role::System,
                                    content: "Compacting session...".into(),
                                    timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                                    agent_name: Some("System".into()), streaming: false,
                                    channel_source: Some("tui".into()),
                                    stream_state: None,
                                });
                                let url = gateway_url.clone();
                                let compact_app = app_state.clone();
                                drop(a);
                                tokio::spawn(async move {
                                    let api = api::ApiClient::new(&url);
                                    let result = api.session_compact("agent:main:main").await
                                        .unwrap_or_else(|e| format!("Error: {e}"));
                                    if let Ok(mut a) = compact_app.lock() {
                                        a.messages.push(app::ChatMessage {
                                            role: app::Role::System,
                                            content: result,
                                            timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                                            agent_name: Some("System".into()), streaming: false,
                                            channel_source: Some("tui".into()),
                                            stream_state: None,
                                        });
                                    }
                                });
                                continue;
                            }

                            if msg == "/plan" || msg.starts_with("/plan ") {
                                let arg = msg.strip_prefix("/plan").unwrap_or("").trim().to_string();
                                a.clear_input();
                                let content = render_plan_command(&arg);
                                a.messages.push(app::ChatMessage {
                                    role: app::Role::System,
                                    content,
                                    timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                                    agent_name: Some("System".into()),
                                    streaming: false,
                                    channel_source: Some("tui".into()),
                                    stream_state: None,
                                });
                                continue;
                            }

                            if msg == "/help" {
                                a.clear_input();
                                let help_text = "\
**Zeus TUI — Commands & Keybindings**

Commands:
  /clear    — Clear chat and reset session
  /compact  — Summarize session to save context
  /model    — Show current model or switch: /model <name>
  /verify   — Spawn verification agent to review last response
  /council  — Run multi-model consensus pipeline on a question
  /plan     — List active plans, or /plan <slug>|latest to view steps
  /help     — Show this help
  /status   — Show model, gateway, session info
  /think    — Enable deep thinking for next message

Navigation:
  PgUp/PgDn    — Scroll chat history
  Home/End     — Jump to top/bottom
  ↑/↓          — Recall input history
  Ctrl+C       — Quit
  Esc          — Cancel streaming response";
                                a.messages.push(app::ChatMessage {
                                    role: app::Role::System,
                                    content: help_text.into(),
                                    timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                                    agent_name: Some("System".into()),
                                    streaming: false,
                                    channel_source: Some("tui".into()),
                                    stream_state: None,
                                });
                                continue;
                            }

                            // /model — show current or switch model
                            if msg == "/model" || msg.starts_with("/model ") {
                                let arg = msg.strip_prefix("/model").unwrap_or("").trim();
                                a.clear_input();
                                if arg.is_empty() {
                                    // Show current model
                                    let model = if a.model.is_empty() { "unknown".to_string() } else { a.model.clone() };
                                    a.messages.push(app::ChatMessage {
                                        role: app::Role::System,
                                        content: format!("Current model: **{}**\n\nUsage: `/model <provider/model>` to switch.", model),
                                        timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                                        agent_name: Some("System".into()),
                                        streaming: false, stream_state: None,
                                        channel_source: Some("tui".into()),
                                    });
                                } else {
                                    // Switch model via config API
                                    let new_model = arg.to_string();
                                    a.messages.push(app::ChatMessage {
                                        role: app::Role::System,
                                        content: format!("Switching model to **{}**...", new_model),
                                        timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                                        agent_name: Some("System".into()),
                                        streaming: false, stream_state: None,
                                        channel_source: Some("tui".into()),
                                    });
                                    let url = gateway_url.clone();
                                    let model_app = app_state.clone();
                                    drop(a);
                                    tokio::spawn(async move {
                                        let api = api::ApiClient::new(&url);
                                        let updates = serde_json::json!({"model": {"default": new_model}});
                                        match api.update_config(&updates).await {
                                            Ok(_) => {
                                                if let Ok(mut a) = model_app.lock() {
                                                    a.model = new_model.clone();
                                                    a.messages.push(app::ChatMessage {
                                                        role: app::Role::System,
                                                        content: format!("Model switched to **{}**.", new_model),
                                                        timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                                                        agent_name: Some("System".into()),
                                                        streaming: false, stream_state: None,
                                                        channel_source: Some("tui".into()),
                                                    });
                                                }
                                            }
                                            Err(e) => {
                                                if let Ok(mut a) = model_app.lock() {
                                                    a.messages.push(app::ChatMessage {
                                                        role: app::Role::System,
                                                        content: format!("Failed to switch model: {}", e),
                                                        timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                                                        agent_name: Some("System".into()),
                                                        streaming: false, stream_state: None,
                                                        channel_source: Some("tui".into()),
                                                    });
                                                }
                                            }
                                        }
                                    });
                                    continue;
                                }
                                continue;
                            }

                            if msg == "/status" {
                                let model = a.model.clone();
                                let gw_url = a.gateway_url.clone();
                                let gw_ver = a.gateway_version.clone();
                                a.clear_input();
                                let status_text = format!(
                                    "**Zeus TUI Status**\n\nModel:   {}\nGateway: {}\nVersion: {}\nSession: agent:main:main",
                                    if model.is_empty() { "unknown".to_string() } else { model },
                                    gw_url,
                                    if gw_ver.is_empty() { "unknown".to_string() } else { gw_ver },
                                );
                                a.messages.push(app::ChatMessage {
                                    role: app::Role::System,
                                    content: status_text,
                                    timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                                    agent_name: Some("System".into()),
                                    streaming: false,
                                    channel_source: Some("tui".into()),
                                    stream_state: None,
                                });
                                continue;
                            }

                            // S103 #29: ultrathink / /think — boost reasoning for next message
                            if msg == "/think" || msg.to_lowercase().starts_with("ultrathink") {
                                a.clear_input();
                                a.ultrathink_next = true;
                                a.messages.push(app::ChatMessage {
                                    role: app::Role::System,
                                    content: "Deep thinking enabled for next message. Type your question.".to_string(),
                                    timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                                    agent_name: Some("System".into()),
                                    streaming: false,
                                    channel_source: Some("tui".into()),
                                    stream_state: None,
                                });
                                continue;
                            }

                            // /verify — spawn subagent to review last assistant response
                            if msg == "/verify" {
                                // Find the last assistant message
                                let last_assistant = a.messages.iter().rev()
                                    .find(|m| m.role == app::Role::Assistant && !m.streaming)
                                    .map(|m| m.content.clone());
                                a.clear_input();
                                match last_assistant {
                                    None => {
                                        a.messages.push(app::ChatMessage {
                                            role: app::Role::System,
                                            content: "⚠ No assistant response to verify yet.".into(),
                                            timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                                            agent_name: Some("System".into()),
                                            streaming: false,
                                            channel_source: Some("tui".into()),
                                            stream_state: None,
                                        });
                                    }
                                    Some(response_to_verify) => {
                                        a.messages.push(app::ChatMessage {
                                            role: app::Role::System,
                                            content: "🔍 Verification agent running...".into(),
                                            timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                                            agent_name: Some("Verifier".into()),
                                            streaming: false,
                                            channel_source: Some("tui".into()),
                                            stream_state: None,
                                        });
                                        // Placeholder for streaming result
                                        a.messages.push(app::ChatMessage {
                                            role: app::Role::System,
                                            content: "thinking...".into(),
                                            timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                                            agent_name: Some("Verifier".into()),
                                            streaming: true,
                                            channel_source: Some("tui".into()),
                                            stream_state: None,
                                        });
                                        let url = gateway_url.clone();
                                        let verify_app = app_state.clone();
                                        drop(a);
                                        tokio::spawn(async move {
                                            let verify_prompt = format!(
                                                "You are a verification agent. Review the following assistant response for correctness. Check for: (1) code that won't compile or has syntax errors, (2) incorrect or hallucinated file paths, (3) hallucinated function names or APIs that don't exist. Be concise. Flag issues clearly or confirm it looks correct.\n\n---\n{}\n---",
                                                response_to_verify
                                            );
                                            let api = api::ApiClient::new(&url);
                                            let mut first_token = true;
                                            let _ = api.chat_stream(&verify_prompt, |event| {
                                                use api::SseEvent;
                                                if let Ok(mut a) = verify_app.lock() {
                                                    match event {
                                                        SseEvent::Token(token) => {
                                                            if let Some(last) = a.messages.iter_mut().rev()
                                                                .find(|m| m.streaming && m.agent_name.as_deref() == Some("Verifier"))
                                                            {
                                                                if first_token {
                                                                    last.content = token;
                                                                    first_token = false;
                                                                } else {
                                                                    last.content.push_str(&token);
                                                                }
                                                            }
                                                        }
                                                        _ => {}
                                                    }
                                                }
                                            }).await;
                                            // Stream complete — mark as done and add badge
                                            if let Ok(mut a) = verify_app.lock() {
                                                if let Some(last) = a.messages.iter_mut().rev()
                                                    .find(|m| m.streaming && m.agent_name.as_deref() == Some("Verifier"))
                                                {
                                                    last.streaming = false;
                                                    last.content = format!("**🔍 Verification Result**\n\n{}", last.content);
                                                }
                                            }
                                        });
                                                continue;
                                    }
                                }
                                continue;
                            }

                            // S105 #48: /council <question> — run multi-model consensus pipeline
                            if msg.starts_with("/council ") || msg == "/council" {
                                let question = msg.trim_start_matches("/council").trim().to_string();
                                a.clear_input();
                                if question.is_empty() {
                                    a.messages.push(app::ChatMessage {
                                        role: app::Role::System,
                                        content: "Usage: /council <question>".into(),
                                        timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                                        agent_name: Some("System".into()),
                                        streaming: false,
                                        channel_source: Some("tui".into()),
                                        stream_state: None,
                                    });
                                } else {
                                    a.messages.push(app::ChatMessage {
                                        role: app::Role::System,
                                        content: format!("⚖ Council convening on: {}", question),
                                        timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                                        agent_name: Some("Council".into()),
                                        streaming: false,
                                        channel_source: Some("tui".into()),
                                        stream_state: None,
                                    });
                                    // Streaming placeholder
                                    a.messages.push(app::ChatMessage {
                                        role: app::Role::System,
                                        content: "Stage 1: gathering opinions...".into(),
                                        timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                                        agent_name: Some("Council".into()),
                                        streaming: true,
                                        channel_source: Some("tui".into()),
                                        stream_state: None,
                                    });
                                    let council_app = app_state.clone();
                                    drop(a);
                                    tokio::spawn(async move {
                                        let config = zeus_council::CouncilConfig::default();
                                        let result = zeus_council::pipeline::run_council(&question, config).await;
                                        if let Ok(mut a) = council_app.lock() {
                                            if let Some(last) = a.messages.iter_mut().rev()
                                                .find(|m| m.streaming && m.agent_name.as_deref() == Some("Council"))
                                            {
                                                last.streaming = false;
                                                last.content = match result {
                                                    Ok(r) => format!("**⚖ Council Verdict**\n\n{}", r.final_answer),
                                                    Err(e) => format!("**⚖ Council Error**\n\n{}", e),
                                                };
                                            }
                                        }
                                    });
                                    continue;
                                }
                                continue;
                            }

                            // T20: Queue new sends while a response is still streaming.
                            // Drained one-at-a-time when the current stream finishes (see
                            // stream-end hook). Esc cancels the in-flight stream and the
                            // queue is preserved so the user can keep typing ahead.
                            let is_streaming = a.messages.iter().any(|m| m.streaming);
                            if is_streaming {
                                if a.pending_inputs.len() >= a.max_pending {
                                    // Cap reached: drop the oldest queued message.
                                    a.pending_inputs.pop_front();
                                }
                                a.pending_inputs.push_back(msg.clone());
                                let depth = a.pending_inputs.len();
                                a.messages.push(app::ChatMessage {
                                    role: app::Role::User,
                                    content: msg.clone(),
                                    timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                                    agent_name: Some("You".into()),
                                    streaming: false,
                                    channel_source: Some("tui".into()),
                                    stream_state: None,
                                });
                                a.messages.push(app::ChatMessage {
                                    role: app::Role::System,
                                    content: format!("📥 Queued ({} pending) — will send when current response finishes. Ctrl-K to clear queue, Esc to cancel current.", depth),
                                    timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                                    agent_name: Some("System".into()),
                                    streaming: false,
                                    channel_source: Some("tui".into()),
                                    stream_state: None,
                                });
                                a.input_history.push(msg.clone());
                                a.input_history_idx = -1;
                                a.clear_input();
                                continue;
                            }
                            // Push to input history for arrow-key recall
                            a.input_history.push(msg.clone());
                            a.input_history_idx = -1;
                            a.messages.push(app::ChatMessage { role: app::Role::User, content: msg.clone(), timestamp: chrono::Local::now().format("%H:%M:%S").to_string(), agent_name: Some("You".to_string()), streaming: false, channel_source: Some("tui".into()), stream_state: None });
                            // Show "thinking..." while waiting for first token
                            let self_name_thinking = a.self_name.clone();
                            a.messages.push(app::ChatMessage { role: app::Role::Assistant, content: "thinking...".to_string(), timestamp: chrono::Local::now().format("%H:%M:%S").to_string(), agent_name: Some(self_name_thinking), streaming: true, channel_source: Some("tui".into()), stream_state: None });
                            a.clear_input();
                            // S103 #29: consume ultrathink flag
                            let use_ultrathink = a.ultrathink_next;
                            if a.ultrathink_next { a.ultrathink_next = false; }
                            let send_msg = if use_ultrathink {
                                format!("[THINKING_LEVEL=xhigh] {}", msg)
                            } else {
                                msg.clone()
                            };
                            let url = gateway_url.clone();
                            let tx = chat_tx.clone();
                            let stream_app = app_state.clone();
                            // Create a CancellationToken for this cooking task.
                            // Esc/Ctrl+C handlers fire `.cancel()` to abort cooperatively.
                            let cancel = tokio_util::sync::CancellationToken::new();
                            a.stream_cancelled = false;
                            a.cooking_cancel = Some(cancel.clone());
                            a.cooking_started_at = Some(std::time::Instant::now());
                            drop(a);
                            tokio::spawn(async move {
                                // Scopeguard drain: guarantees cleanup + pending-queue drain
                                // on EVERY exit path (success, error, panic, cancellation).
                                // Without this, errors / lock failures / 200-iter cap /
                                // edge-case cancellations leave the queue stuck.
                                let drain_app = stream_app.clone();
                                let _drain_guard = scopeguard::guard((), |_| {
                                    if let Ok(mut a) = drain_app.lock() {
                                        a.cooking_cancel = None;
                                        a.cooking_started_at = None;
                                        a.cooking_iter = 0;
                                        a.cooking_tools = 0;
                                        a.thinking_text = None;
                                        if !a.stream_cancelled {
                                            if let Some(next) = a.pending_inputs.pop_front() {
                                                a.pending_drain = Some(next);
                                            }
                                        }
                                    }
                                });
                                let api = api::ApiClient::new(&url);
                                let msg = send_msg;
                                let mut first_token = true;
                                let cancel_for_cb = cancel.clone();
                                let stream_fut = api.chat_stream(&msg, |event| {
                                    // Short-circuit if the user has cancelled — keeps the
                                    // callback cheap and avoids touching app state after abort.
                                    if cancel_for_cb.is_cancelled() { return; }
                                    use api::SseEvent;
                                    if let Ok(mut a) = stream_app.lock() {
                                        // S100 #4: check if user cancelled via Esc
                                        if a.stream_cancelled {
                                            return; // skip all further events
                                        }
                                        match event {
                                            // ── Layer 1: live token streaming ────────────────
                                            SseEvent::Token(token) => {
                                                if let Some(last) = a.messages.last_mut() {
                                                    if last.streaming {
                                                        if first_token {
                                                            last.content = token;
                                                            first_token = false;
                                                        } else {
                                                            last.content.push_str(&token);
                                                        }
                                                    }
                                                }
                                            }
                                            // ── Layer 2: tool event display ──────────────────
                                            SseEvent::ToolStart { name, input } => {
                                                // Truncate long inputs for display
                                                let summary = if input.len() > 60 {
                                                    format!("{}…", zeus_core::truncate_str(&input, 60))
                                                } else {
                                                    input.clone()
                                                };
                                                // If the previous message is an empty/"thinking..." streaming
                                                // placeholder, drop it so the ⚡ line takes its place instead of
                                                // stacking a stale bubble above every tool call.
                                                if let Some(last) = a.messages.last() {
                                                    let stale = last.streaming
                                                        && (last.content.is_empty()
                                                            || last.content == "thinking...");
                                                    if stale {
                                                        a.messages.pop();
                                                    }
                                                }
                                                a.push_tool_event(&name, &summary);
                                                // Push a fresh streaming placeholder for the next token batch.
                                                let self_name_tool = a.self_name.clone();
                                                a.messages.push(app::ChatMessage {
                                                    role: app::Role::Assistant,
                                                    content: String::new(),
                                                    timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                                                    agent_name: Some(self_name_tool),
                                                    streaming: true,
                                                    stream_state: None,
                                                    channel_source: Some("tui".into()),
                                                });
                                                first_token = true;
                                            }
                                            SseEvent::ToolEnd { output, .. } => {
                                                // Append ✓ / ✗ to the matching ⚡ line.
                                                // Heuristic: output starting with "Error" or "error:" = failure.
                                                let failed = output.trim_start()
                                                    .to_ascii_lowercase()
                                                    .starts_with("error");
                                                a.mark_last_tool_done(!failed);
                                            }
                                            // ── Layer 3: iteration counter ───────────────────
                                            // NOTE: do NOT reset total_tools here — it accumulates
                                            // across the whole turn for the final summary.
                                            SseEvent::Iter(n) => {
                                                a.cooking_iter = n;
                                                a.cooking_tools = 0;
                                            }
                                            // ── Layer 3: thinking display ────────────────────
                                            SseEvent::Thinking(text) => {
                                                a.set_thinking(&text);
                                            }
                                            SseEvent::Usage { input, output } => {
                                                a.record_turn_tokens(input, output);
                                            }
                                        }
                                    }
                                });
                                // Race the stream against cancellation. If the user hits
                                // Esc/Ctrl+C, `cancel.cancelled()` resolves immediately and
                                // we drop the in-flight reqwest stream — no half-rendered
                                // tokens, no zombie HTTP request.
                                let cancelled_result: Option<Result<String, String>> = tokio::select! {
                                    biased;
                                    _ = cancel.cancelled() => None,
                                    res = stream_fut => Some(res),
                                };
                                let was_cancelled = cancelled_result.is_none();
                                let final_content = match cancelled_result {
                                    Some(Ok(full)) => full,
                                    Some(Err(e)) => format!("Error: {e}"),
                                    None => "[cancelled]".to_string(),
                                };
                                let self_name_final = stream_app.lock()
                                    .map(|a| a.self_name.clone())
                                    .unwrap_or_else(|_| "Zeus".to_string());
                                // On clean finish, emit the "✅ Done" summary.
                                // On cancel we skip the summary — the "[cancelled]" message
                                // is enough signal.
                                // NOTE: cooking_cancel / cooking_iter / cooking_tools /
                                // thinking_text / pending_drain cleanup is handled by the
                                // scopeguard _drain_guard — runs on EVERY exit path.
                                if !was_cancelled {
                                    if let Ok(mut a) = stream_app.lock() {
                                        a.push_turn_summary();
                                    }
                                }
                                if was_cancelled {
                                    // Don't push a duplicate cancel bubble — the Esc/Ctrl+C
                                    // handler already mutated the streaming placeholder in-place.
                                    return;
                                }
                                let _ = tx.send(app::ChatMessage { role: app::Role::Assistant, content: final_content, timestamp: chrono::Local::now().format("%H:%M:%S").to_string(), agent_name: Some(self_name_final), streaming: false, channel_source: Some("tui".into()), stream_state: None }).await;
                            });
                            continue;
                        }
                    }
                    crossterm::event::KeyCode::Up if a.input.is_empty() || a.input_history_idx >= 0 => {
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
                    crossterm::event::KeyCode::Down if a.input_history_idx >= 0 => {
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
                    crossterm::event::KeyCode::Backspace => { a.delete_char_before(); }
                    crossterm::event::KeyCode::Left => { a.cursor_left(); }
                    crossterm::event::KeyCode::Right => { a.cursor_right(); }
                    crossterm::event::KeyCode::Home => { a.cursor_home(); }
                    crossterm::event::KeyCode::End => { a.cursor_end(); }
                    crossterm::event::KeyCode::Char(c) => { a.insert_char(c); }
                    _ => {}
                }
            }
        }
        {
            let mut a = app_state.lock().unwrap();
            a.tick();
            a.update_context_tokens();
            // Office runs at 8 TPS (every ~125ms). With 100ms poll interval,
            // tick every other frame to approximate 8 TPS.
            // Note: office_bg / scene_dimensions are now self-healed inside
            // `office::render` (sized to live `area` on each frame), so no
            // resize-poll regen is needed here.
            if a.tick_count % 2 == 0 {
                a.office.tick();
                a.office.connected = a.connected;
            }
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), crossterm::event::DisableMouseCapture, LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

/// Render `/plan` command output — lists active plans, or shows steps for a slug.
///
/// Usage:
///   `/plan`            → list all plans in `~/.zeus/workspace/plans/`
///   `/plan latest`     → show the most recently modified plan's PLAN.md
///   `/plan <slug>`     → show PLAN.md for that specific plan
fn render_plan_command(arg: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    let plans_root = std::path::PathBuf::from(&home)
        .join(".zeus")
        .join("workspace")
        .join("plans");

    if !plans_root.exists() {
        return "📋 No plans directory yet. Plans appear under `~/.zeus/workspace/plans/` when a titan enters plan mode.".to_string();
    }

    // Collect plan dirs with their mtime
    let mut entries: Vec<(std::path::PathBuf, std::time::SystemTime, String)> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&plans_root) {
        for entry in rd.flatten() {
            let path = entry.path();
            if !path.is_dir() { continue; }
            let name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };
            let mtime = entry.metadata().and_then(|m| m.modified()).unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            entries.push((path, mtime, name));
        }
    }
    // Newest first
    entries.sort_by(|a, b| b.1.cmp(&a.1));

    if entries.is_empty() {
        return "📋 No plans yet.\n\nPlans appear here when a titan breaks a complex task into steps.".to_string();
    }

    // No arg → list
    if arg.is_empty() {
        let mut out = String::from("📋 **Active plans** (newest first):\n\n");
        for (path, _mtime, name) in entries.iter().take(20) {
            let status = read_plan_status(path).unwrap_or_else(|| "UNKNOWN".to_string());
            let steps = read_plan_step_progress(path).unwrap_or_else(|| "".to_string());
            let steps_suffix = if steps.is_empty() { String::new() } else { format!(" · {}", steps) };
            out.push_str(&format!("  • `{}` — {}{}\n", name, status, steps_suffix));
        }
        out.push_str("\nUse `/plan <slug>` or `/plan latest` to view steps.");
        return out;
    }

    // Resolve slug
    let target = if arg == "latest" {
        entries.first().map(|(p, _, n)| (p.clone(), n.clone()))
    } else {
        entries.iter()
            .find(|(_, _, n)| n == arg)
            .map(|(p, _, n)| (p.clone(), n.clone()))
    };

    let (plan_dir, slug) = match target {
        Some(t) => t,
        None => return format!("📋 Plan not found: `{}`\n\nUse `/plan` to list available plans.", arg),
    };

    let plan_md = plan_dir.join("PLAN.md");
    if !plan_md.exists() {
        return format!("📋 `{}` — no PLAN.md yet (plan still being written).", slug);
    }

    let content = match std::fs::read_to_string(&plan_md) {
        Ok(c) => c,
        Err(e) => return format!("📋 Failed to read PLAN.md for `{}`: {}", slug, e),
    };

    let status = read_plan_status(&plan_dir).unwrap_or_else(|| "UNKNOWN".to_string());
    let progress = read_plan_step_progress(&plan_dir).unwrap_or_else(|| "".to_string());
    let progress_suffix = if progress.is_empty() { String::new() } else { format!(" · {}", progress) };

    let mut out = format!("📋 **{}** — {}{}\n\n", slug, status, progress_suffix);

    // Skip YAML frontmatter if present
    let body = if content.starts_with("---") {
        if let Some(idx) = content[3..].find("\n---") {
            &content[3 + idx + 4..]
        } else {
            content.as_str()
        }
    } else {
        content.as_str()
    };
    let body = body.trim_start();

    // Cap output to keep the chat bubble sane
    const MAX_CHARS: usize = 4000;
    if body.chars().count() > MAX_CHARS {
        let truncated: String = body.chars().take(MAX_CHARS).collect();
        out.push_str(&truncated);
        out.push_str("\n\n… (truncated — open the file for full plan)");
    } else {
        out.push_str(body);
    }
    out.push_str(&format!("\n\n_{}_", plan_md.display()));
    out
}

/// Read STATUS.md and extract the `status:` frontmatter field.
fn read_plan_status(plan_dir: &std::path::Path) -> Option<String> {
    let status_md = plan_dir.join("STATUS.md");
    let content = std::fs::read_to_string(&status_md).ok()?;
    for line in content.lines().take(40) {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("status:") {
            return Some(rest.trim().trim_matches('"').to_string());
        }
    }
    None
}

/// Read STATUS.md and return a short progress string like "3/7 steps".
fn read_plan_step_progress(plan_dir: &std::path::Path) -> Option<String> {
    let status_md = plan_dir.join("STATUS.md");
    let content = std::fs::read_to_string(&status_md).ok()?;
    let mut done: Option<String> = None;
    let mut total: Option<String> = None;
    for line in content.lines().take(60) {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("steps_completed:") {
            done = Some(rest.trim().to_string());
        } else if let Some(rest) = trimmed.strip_prefix("total_steps:") {
            total = Some(rest.trim().to_string());
        }
    }
    match (done, total) {
        (Some(d), Some(t)) => Some(format!("{}/{} steps", d, t)),
        _ => None,
    }
}

/// Returns true if Zeus config is missing — triggers onboarding.
fn needs_onboarding() -> bool {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    let config_path = std::path::PathBuf::from(&home).join(".zeus").join("config.toml");
    if !config_path.exists() { return true; }
    // Check if onboarding_complete = false (bootstrap config)
    if let Ok(content) = std::fs::read_to_string(&config_path) {
        if content.contains("onboarding_complete = false") { return true; }
    }
    false
}

/// Cycle auth_mode for the Auth step. `forward=true` = Tab/Down, `false` = Up.
/// Resets any in-flight browser/device-code attempt when the mode changes.
fn cycle_auth_mode(state: &mut onboarding::OnboardingState, forward: bool) {
    let provider_id = onboarding::PROVIDERS
        .get(state.selected_provider)
        .map(|p| p.provider_id)
        .unwrap_or("");
    let supports_oauth = matches!(provider_id, "anthropic" | "openai" | "google" | "google-gemini-cli");
    let supports_device_code = matches!(provider_id, "qwen" | "minimax");
    state.browser_auth_pending = false; // reset any stuck in-flight attempt
    if supports_device_code {
        // Both Qwen and MiniMax cycle between API Key (0) and Device Code (3)
        state.auth_mode = if state.auth_mode == 0 { 3 } else { 0 };
        // Reset device code state when switching modes
        state.device_code_user_code.clear();
        state.device_code_verification_url.clear();
        state.device_code_pending = false;
    } else if supports_oauth {
        state.auth_mode = if forward {
            match state.auth_mode { 0 => 1, 1 => 2, _ => 0 }
        } else {
            match state.auth_mode { 0 => 2, 1 => 0, _ => 1 }
        };
        if provider_id == "google-gemini-cli" && state.auth_mode == 0 {
            state.auth_mode = if forward { 1 } else { 2 };
        }
    }
}

/// Run the onboarding state machine until complete or user quits.
/// Spawn a background model fetch for the current provider + key.
/// Called from the Auth step on both Enter (normal) and Y (CLI credential prompt).
fn start_model_fetch(
    state: &mut onboarding::OnboardingState,
    tx: tokio::sync::mpsc::Sender<Result<Vec<String>, String>>,
) {
    let provider_id = onboarding::PROVIDERS
        .get(state.selected_provider)
        .map(|p| p.provider_id)
        .unwrap_or("anthropic");
    let key = if !state.api_key.trim().is_empty() {
        state.api_key.clone()
    } else {
        state.oauth_token.clone()
    };
    state.models_fetching = true;
    state.fetched_models.clear();
    state.error = None;
    let pid = provider_id.to_string();
    tokio::spawn(async move {
        let result = onboarding::fetch_models(&pid, &key).await;
        let _ = tx.send(result).await;
    });
}

async fn run_onboarding<B: ratatui::backend::Backend>(
    terminal: &mut ratatui::Terminal<B>,
) -> std::io::Result<usize>
where
    std::io::Error: From<<B as ratatui::backend::Backend>::Error>,
{
    use onboarding::{OnboardingState, OnboardingStep};
    use crossterm::event::{self, Event, KeyCode, KeyModifiers};

    let mut state = OnboardingState::new();

    // Channel for async model fetching
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

        terminal.draw(|f| onboarding::render_onboarding(f, &state))?;

        if state.complete { return Ok(state.complete_selection); }

        if event::poll(std::time::Duration::from_millis(50))? {
            state.tick();
            let ev = event::read()?;
            // Handle paste events (bracket paste mode) for API key input
            if let Event::Paste(ref text) = ev {
                if state.step == OnboardingStep::Auth && !state.cli_cred_prompt {
                    if state.auth_mode == 0 { state.api_key.push_str(text); }
                    else { state.oauth_token.push_str(text); }
                }
            }
            if let Event::Key(key) = ev {
                match (key.modifiers, key.code) {
                    // Welcome screen: Y continues, N/Q exits
                    (KeyModifiers::NONE, KeyCode::Char('y') | KeyCode::Char('Y'))
                        if state.step == OnboardingStep::Welcome => { state.advance(); }
                    (KeyModifiers::NONE, KeyCode::Char('n') | KeyCode::Char('N'))
                        if state.step == OnboardingStep::Welcome => return Ok(2),
                    (KeyModifiers::NONE, KeyCode::Char('q'))
                        if state.step == OnboardingStep::Welcome => return Ok(2),
                    // Ctrl-C always exits
                    (KeyModifiers::CONTROL, KeyCode::Char('c')) => return Ok(2), // Save & Exit
                    (KeyModifiers::NONE, KeyCode::Esc) => { state.back(); }
                    // Tab acts as Down on selector pages (universal navigation)
                    (KeyModifiers::NONE, KeyCode::Tab)
                        if matches!(state.step, OnboardingStep::SetupMode | OnboardingStep::Provider
                            | OnboardingStep::Model | OnboardingStep::Fallback | OnboardingStep::Channels
                            | OnboardingStep::Features | OnboardingStep::Security
                            | OnboardingStep::Skills | OnboardingStep::Complete) =>
                    {
                        match state.step {
                            OnboardingStep::SetupMode => {
                                if state.setup_mode < 2 { state.setup_mode += 1; }
                            }
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
                            OnboardingStep::Fallback => {
                                let max = onboarding::PROVIDERS.len().saturating_sub(2);
                                if state.fallback_focus + 1 <= max { state.fallback_focus += 1; }
                            }
                            OnboardingStep::Channels => {
                                if state.sel + 1 < onboarding::CHANNELS.len() {
                                    state.sel += 1;
                                }
                            }
                            OnboardingStep::Features => {
                                let feature_count = 9; // FEATURES list: nous, mnemosyne, aegis, athena, hermes, prometheus, browser, talos, mcp
                                if state.sel + 1 < feature_count { state.sel += 1; }
                            }
                            OnboardingStep::Security => {
                                if state.security_level < 2 { state.security_level += 1; }
                            }
                            OnboardingStep::Skills => {
                                let total: usize = state.skills.iter().map(|c| c.items.len()).sum();
                                if state.sel + 1 < total { state.sel += 1; }
                            }
                            OnboardingStep::Complete => {
                                if state.complete_selection < 2 { state.complete_selection += 1; }
                            }
                            _ => {}
                        }
                    }
                    // Enter during Y/N credential prompt acts as Y
                    (KeyModifiers::NONE, KeyCode::Enter)
                        if state.step == OnboardingStep::Auth && state.cli_cred_prompt =>
                    {
                        if let Some(ref cred) = state.cli_cred.clone() {
                            if cred.is_oauth {
                                state.oauth_token = cred.token.clone();
                                state.auth_mode = 1;
                            } else {
                                state.api_key = cred.token.clone();
                                state.auth_mode = 0;
                            }
                        }
                        state.cli_cred_prompt = false;
                        start_model_fetch(&mut state, model_tx.clone());
                        state.advance();
                    }
                    (KeyModifiers::NONE, KeyCode::Enter) => {
                        // When leaving Auth step, fetch models from provider API
                        if state.step == OnboardingStep::Auth {
                            let has_key = !state.api_key.trim().is_empty() || !state.oauth_token.trim().is_empty();
                            let is_browser_flow = state.auth_mode == 2 || state.auth_mode == 3;
                            if is_browser_flow {
                                // Browser OAuth / device code — advance() owns the flow
                                state.advance();
                            } else if has_key {
                                start_model_fetch(&mut state, model_tx.clone());
                                state.advance();
                            } else {
                                // No key entered — check if detected in env
                                let detected = state.providers_with_detection.get(state.selected_provider).copied().unwrap_or(false);
                                let is_local = onboarding::PROVIDERS.get(state.selected_provider)
                                    .map(|p| p.provider_id == "ollama").unwrap_or(false);
                                if detected || is_local {
                                    state.advance();
                                } else {
                                    state.error = Some("Enter an API key or OAuth token to continue".into());
                                }
                            }
                        } else {
                            state.advance();
                        }
                    }
                    // Left/Right for provider grid navigation (6 cols)
                    (KeyModifiers::NONE, KeyCode::Left) if state.step == OnboardingStep::Provider => {
                        if state.selected_provider > 0 { state.selected_provider -= 1; }
                    }
                    (KeyModifiers::NONE, KeyCode::Right) if state.step == OnboardingStep::Provider => {
                        if state.selected_provider + 1 < onboarding::PROVIDERS.len() {
                            state.selected_provider += 1;
                        }
                    }
                    // B key shortcut removed — was eating 'b'/'B' from Discord bot tokens.
                    // Policy cycling is now Tab-only (via bot_policy_focused).
                    (KeyModifiers::NONE, KeyCode::Up) => {
                        match state.step {
                            OnboardingStep::Auth if !state.cli_cred_prompt => {
                                cycle_auth_mode(&mut state, false);
                            }
                            OnboardingStep::SetupMode => {
                                if state.setup_mode > 0 { state.setup_mode -= 1; }
                            }
                            OnboardingStep::Provider => {
                                // Up moves to previous row (6 cols)
                                if state.selected_provider >= 6 { state.selected_provider -= 6; }
                            }
                            OnboardingStep::Model => {
                                if state.selected_model > 0 { state.selected_model -= 1; }
                            }
                            OnboardingStep::Agent => {
                                if state.sel == 4 {
                                    // personality style selector
                                    if state.personality_style > 0 { state.personality_style -= 1; }
                                } else if state.sel >= 5 {
                                    // S80: 2D persona navigation
                                    if state.persona_item > 0 {
                                        state.persona_item -= 1;
                                    } else if state.persona_cat > 0 {
                                        state.persona_cat -= 1;
                                        let prev_len = state.personas.get(state.persona_cat).map(|c| c.items.len()).unwrap_or(0);
                                        state.persona_item = prev_len.saturating_sub(1);
                                    }
                                }
                            }
                            OnboardingStep::Fallback => {
                                if state.fallback_focus > 0 { state.fallback_focus -= 1; }
                            }
                            OnboardingStep::Channels => {
                                if state.sel > 0 { state.sel -= 1; }
                            }
                            OnboardingStep::Features => {
                                if state.sel > 0 { state.sel -= 1; }
                            }
                            OnboardingStep::Security => {
                                if state.security_level > 0 { state.security_level -= 1; }
                            }
                            OnboardingStep::Skills => {
                                if state.sel > 0 { state.sel -= 1; }
                            }
                            OnboardingStep::Complete => {
                                if state.complete_selection > 0 { state.complete_selection -= 1; }
                            }
                            _ => {}
                        }
                    }
                    (KeyModifiers::NONE, KeyCode::Down) => {
                        match state.step {
                            OnboardingStep::Auth if !state.cli_cred_prompt => {
                                cycle_auth_mode(&mut state, true);
                            }
                            OnboardingStep::SetupMode => {
                                if state.setup_mode < 2 { state.setup_mode += 1; }
                            }
                            OnboardingStep::Provider => {
                                // Down moves to next row (6 cols)
                                if state.selected_provider + 6 < onboarding::PROVIDERS.len() {
                                    state.selected_provider += 6;
                                }
                            }
                            OnboardingStep::Model => {
                                if state.selected_model + 1 < state.current_models().len() {
                                    state.selected_model += 1;
                                }
                            }
                            OnboardingStep::Agent => {
                                if state.sel == 4 {
                                    // personality style selector — 4 options
                                    if state.personality_style < 3 { state.personality_style += 1; }
                                } else if state.sel >= 5 {
                                    // S80: 2D persona navigation
                                    let cat_len = state.personas.get(state.persona_cat).map(|c| c.items.len()).unwrap_or(0);
                                    if state.persona_item + 1 < cat_len {
                                        state.persona_item += 1;
                                    } else if state.persona_cat + 1 < state.personas.len() {
                                        state.persona_cat += 1;
                                        state.persona_item = 0;
                                    }
                                }
                            }
                            OnboardingStep::Fallback => {
                                let max = onboarding::PROVIDERS.len().saturating_sub(2);
                                if state.fallback_focus + 1 <= max { state.fallback_focus += 1; }
                            }
                            OnboardingStep::Channels => {
                                if state.sel + 1 < onboarding::CHANNELS.len() {
                                    state.sel += 1;
                                }
                            }
                            OnboardingStep::Features => {
                                let feature_count = 9; // FEATURES: nous, mnemosyne, aegis, athena, hermes, prometheus, browser, talos, mcp
                                if state.sel + 1 < feature_count {
                                    state.sel += 1;
                                }
                            }
                            OnboardingStep::Security => {
                                if state.security_level + 1 < onboarding::SECURITY_LEVELS.len() {
                                    state.security_level += 1;
                                }
                            }
                            OnboardingStep::Skills => {
                                let total: usize = state.skills.iter().map(|c| c.items.len()).sum();
                                if state.sel + 1 < total { state.sel += 1; }
                            }
                            OnboardingStep::Complete => {
                                if state.complete_selection < 2 { state.complete_selection += 1; }
                            }
                            _ => {}
                        }
                    }
                    // Tab cycles auth mode on Auth step (↑/↓ handled in their own arms above)
                    (KeyModifiers::NONE, KeyCode::Tab)
                        if state.step == OnboardingStep::Auth && !state.cli_cred_prompt =>
                    {
                        cycle_auth_mode(&mut state, true);
                    }
                    // Tab switches focus on Persona step: agent name → user name → persona list → agent name
                    (KeyModifiers::NONE, KeyCode::Tab) if state.step == OnboardingStep::Agent => {
                        // 0=agent name, 1=user name, 2=user role, 3=user org, 4=style, 5=persona list
                        state.sel = (state.sel + 1) % 6;
                    }
                    // Space toggles fallback provider selection
                    (KeyModifiers::NONE, KeyCode::Char(' ')) if state.step == OnboardingStep::Fallback => {
                        state.toggle_fallback_provider();
                    }
                    // Space toggles channels and skills
                    (KeyModifiers::NONE, KeyCode::Char(' ')) if state.step == OnboardingStep::Channels => {
                        let idx = state.sel;
                        // Block toggle for "coming soon" channels — they're shown but non-selectable.
                        if crate::onboarding::CHANNELS.get(idx).map(|c| c.coming_soon).unwrap_or(false) {
                            // Silent no-op: card is displayed greyed-out; toggle blocked.
                        } else if state.channel_toggled.contains(&idx) {
                            state.channel_toggled.retain(|&x| x != idx);
                        } else {
                            state.channel_toggled.push(idx);
                        }
                    }
                    (KeyModifiers::NONE, KeyCode::Char(' ')) if state.step == OnboardingStep::Skills => {
                        // Toggle the currently focused skill
                        state.toggle_current_skill();
                    }
                    (KeyModifiers::NONE, KeyCode::Char(' ')) if state.step == OnboardingStep::Features => {
                        let keys = ["nous", "mnemosyne", "aegis", "athena", "hermes", "prometheus", "browser", "talos", "mcp"];
                        if let Some(key) = keys.get(state.sel) {
                            let cur = state.feature_toggles.get(key).copied().unwrap_or(false);
                            state.feature_toggles.insert(key, !cur);
                        }
                    }
                    // Tab switches focus in config steps
                    (KeyModifiers::NONE, KeyCode::Tab) if matches!(state.step,
                        OnboardingStep::QuickStart | OnboardingStep::ChanConfig |
                        OnboardingStep::Gateway | OnboardingStep::Workspace |
                        OnboardingStep::Voice | OnboardingStep::Images |
                        OnboardingStep::Orchestration | OnboardingStep::Memory) =>
                    {
                        state.next_field();
                    }
                    // Shift+Tab moves focus backwards in config steps
                    (KeyModifiers::SHIFT, KeyCode::BackTab) if matches!(state.step,
                        OnboardingStep::QuickStart | OnboardingStep::ChanConfig |
                        OnboardingStep::Gateway | OnboardingStep::Workspace |
                        OnboardingStep::Voice | OnboardingStep::Images |
                        OnboardingStep::Orchestration | OnboardingStep::Memory) =>
                    {
                        state.prev_field();
                    }
                    (KeyModifiers::NONE, KeyCode::Backspace) => {
                        match state.step {
                            OnboardingStep::Auth => {
                                if !state.cli_cred_prompt {
                                    if state.auth_mode == 0 { state.api_key.pop(); }
                                    else { state.oauth_token.pop(); }
                                }
                            }
                            OnboardingStep::Agent => {
                                state.delete_char_in_field();
                            }
                            OnboardingStep::QuickStart | OnboardingStep::ChanConfig |
                            OnboardingStep::Gateway | OnboardingStep::Workspace |
                            OnboardingStep::Voice | OnboardingStep::Images |
                            OnboardingStep::Orchestration | OnboardingStep::Memory => {
                                state.delete_char_in_field();
                            }
                            _ => {}
                        }
                    }
                    // Auth step: Y/N response for CLI credential prompt
                    (KeyModifiers::NONE, KeyCode::Char('y') | KeyCode::Char('Y'))
                        if state.step == OnboardingStep::Auth && state.cli_cred_prompt =>
                    {
                        if let Some(ref cred) = state.cli_cred.clone() {
                            // Auto-switch from "Google" to "Gemini CLI" provider when
                            // Gemini CLI credentials are detected on the Google provider
                            if cred.source.contains("Gemini CLI") && cred.provider_name == "Google" {
                                // Switch to Gemini CLI provider (index 4)
                                state.selected_provider = 4; // PROVIDERS[4] = google-gemini-cli
                            }
                            if cred.is_oauth {
                                state.oauth_token = cred.token.clone();
                                state.auth_mode = 1;
                            } else {
                                state.api_key = cred.token.clone();
                                state.auth_mode = 0;
                            }
                        }
                        state.cli_cred_prompt = false;
                        start_model_fetch(&mut state, model_tx.clone());
                        state.advance();
                    }
                    (KeyModifiers::NONE, KeyCode::Char('n') | KeyCode::Char('N'))
                        if state.step == OnboardingStep::Auth && state.cli_cred_prompt =>
                    {
                        state.api_key.clear(); // clear any token previously loaded by Y
                        state.oauth_token.clear();
                        state.cli_cred = None;
                        state.cli_cred_prompt = false;
                    }
                    (_, KeyCode::Char(c)) => {
                        match state.step {
                            OnboardingStep::Auth => {
                                if !state.cli_cred_prompt {
                                    if state.auth_mode == 0 { state.api_key.push(c); }
                                    else { state.oauth_token.push(c); }
                                }
                            }
                            OnboardingStep::Agent => {
                                state.type_char_in_field(c);
                            }
                            OnboardingStep::ChanConfig => {
                                if state.bot_policy_focused {
                                    // Any key cycles policy when focused (Tab got us here)
                                    state.cycle_allow_bots();
                                } else {
                                    // Always type into text field — no 'B' shortcut
                                    // (was eating 'b'/'B' from Discord bot tokens!)
                                    state.type_char_in_field(c);
                                }
                            }
                            OnboardingStep::QuickStart |
                            OnboardingStep::Gateway | OnboardingStep::Workspace |
                            OnboardingStep::Voice | OnboardingStep::Images |
                            OnboardingStep::Orchestration | OnboardingStep::Memory => {
                                state.type_char_in_field(c);
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
        } else {
            state.tick();
        }
    }
    Ok(2) // Save & Exit (fallback)
}
