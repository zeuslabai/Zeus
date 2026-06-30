// =================================================================
// ZEUS — Office Page — Cartoonish futuristic fleet visualization
// 2-panel: agent floor (left) + Pantheon chat with voice (right)
// =================================================================

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;
use crate::api;
use crate::components::visibility::use_tab_visible;

// ── Constants ───────────────────────────────────────────────

const OFFICE_ROOM_NAME: &str = "Zeus Office";
const POLL_INTERVAL_MS: i32 = 4_000;
const FLEET_POLL_MS: i32 = 10_000;

// ── Agent desk data ─────────────────────────────────────────

#[derive(Clone, Debug)]
struct DeskAgent {
    id: String,
    name: String,
    role: String,
    machine: String,
    ip: String,
    status: String,
    health: f32,
    load: f64,
    capabilities: Vec<String>,
}

impl From<api::FleetAgent> for DeskAgent {
    fn from(a: api::FleetAgent) -> Self {
        let role = a.metadata.get("role").cloned().unwrap_or_default();
        let machine = a.metadata.get("machine").cloned().unwrap_or_default();
        let ip = a.metadata.get("ip").cloned().unwrap_or_default();
        Self {
            id: a.id,
            name: a.name,
            role,
            machine,
            ip,
            status: a.status.clone(),
            health: a.health_score,
            load: a.load_pct,
            capabilities: a.capabilities,
        }
    }
}

// ── LocalStorage identity ───────────────────────────────────

fn get_local(key: &str) -> Option<String> {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item(key).ok().flatten())
        .filter(|v| !v.is_empty())
}

fn set_local(key: &str, val: &str) {
    if let Some(storage) = web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
    {
        let _ = storage.set_item(key, val);
    }
}

fn scroll_chat_to_bottom() {
    if let Some(doc) = web_sys::window().and_then(|w| w.document())
        && let Some(el) = doc.get_element_by_id("office-chat-feed") {
            el.set_scroll_top(el.scroll_height());
        }
}

// ── Status helpers ──────────────────────────────────────────

fn status_color(status: &str) -> &'static str {
    match status.to_lowercase().as_str() {
        "online" | "active" | "healthy" => "#00ff88",
        "busy" | "working" | "executing" => "#ffcc00",
        "idle" | "standby" => "#00ccff",
        "offline" | "down" | "error" => "#ff4466",
        _ => "#8888aa",
    }
}

fn status_label(status: &str) -> &'static str {
    match status.to_lowercase().as_str() {
        "online" | "active" | "healthy" => "ONLINE",
        "busy" | "working" | "executing" => "BUSY",
        "idle" | "standby" => "IDLE",
        "offline" | "down" | "error" => "OFFLINE",
        _ => "UNKNOWN",
    }
}

fn role_emoji(role: &str) -> &'static str {
    match role.to_lowercase().as_str() {
        r if r.contains("coordinator") => "\u{1f451}",
        r if r.contains("frontend") => "\u{1f3a8}",
        r if r.contains("backend") => "\u{2699}\u{fe0f}",
        r if r.contains("security") => "\u{1f6e1}\u{fe0f}",
        r if r.contains("marketing") => "\u{1f4e3}",
        r if r.contains("provisioner") => "\u{1f527}",
        r if r.contains("guard") => "\u{1f6a7}",
        _ => "\u{1f916}",
    }
}

fn machine_emoji(machine: &str) -> &'static str {
    let m = machine.to_lowercase();
    if m.contains("freebsd") { return "\u{1f47e}"; }
    if m.contains("macbook") { return "\u{1f4bb}"; }
    if m.contains("mac studio") { return "\u{1f5a5}\u{fe0f}"; }
    if m.contains("mac mini") { return "\u{1f4e6}"; }
    "\u{1f4bb}"
}

fn avatar_color(name: &str) -> &'static str {
    const COLORS: &[&str] = &[
        "#00ff88", "#00ccff", "#ff6600", "#cc44ff",
        "#ffcc00", "#ff4466", "#44ffcc", "#ff88cc",
    ];
    let idx = name.bytes().fold(0usize, |acc, b| acc.wrapping_add(b as usize)) % COLORS.len();
    COLORS[idx]
}

fn initials(name: &str) -> String {
    let parts: Vec<&str> = name.split_whitespace().collect();
    match parts.as_slice() {
        [] => "?".to_string(),
        [w] => w.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or("?".to_string()),
        [a, b, ..] => format!(
            "{}{}",
            a.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default(),
            b.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default(),
        ),
    }
}

fn fmt_time(ts: &str) -> String {
    if ts.len() >= 16 { ts[11..16].to_string() } else { ts.to_string() }
}

// ── Agent Card Component ────────────────────────────────────


// ── Agent Detail Panel ──────────────────────────────────────

#[component]
fn AgentDetailPanel(agent: DeskAgent) -> impl IntoView {
    let sc = status_color(&agent.status);
    let sl = status_label(&agent.status);
    let re = role_emoji(&agent.role);
    let me = machine_emoji(&agent.machine);
    let load_pct = (agent.load * 100.0) as u32;
    let health_pct = (agent.health * 100.0) as u32;
    let caps = agent.capabilities.clone();

    view! {
        <div style={format!(
            "background: linear-gradient(180deg, rgba(10,15,30,0.98) 0%, rgba(5,8,20,0.99) 100%);\
             border: 1px solid {}44; border-radius: 16px; padding: 20px;\
             box-shadow: 0 0 30px {}22, inset 0 0 60px rgba(0,0,0,0.3);",
            sc, sc
        )}>
            // Header
            <div style="display: flex; align-items: center; gap: 12px; margin-bottom: 16px;">
                <div style={format!(
                    "width: 48px; height: 48px; border-radius: 12px;\
                     background: linear-gradient(135deg, {}22, {}44);\
                     border: 1px solid {}66; display: flex; align-items: center; justify-content: center;\
                     font-size: 24px; box-shadow: 0 0 16px {}33;",
                    sc, sc, sc, sc
                )}>
                    {re}
                </div>
                <div>
                    <div style="font-family: 'Orbitron', monospace; font-size: 16px; font-weight: 700;\
                        color: #e0e8f0; letter-spacing: 1px;">
                        {agent.name.clone()}
                    </div>
                    <div style="font-size: 10px; color: #7788aa; font-family: 'Orbitron', monospace;\
                        letter-spacing: 1.5px; text-transform: uppercase;">
                        {agent.role.clone()}
                    </div>
                </div>
                <div style={format!(
                    "margin-left: auto; padding: 4px 14px; border-radius: 20px; font-size: 10px;\
                     font-weight: 800; letter-spacing: 2px; color: #0a0f1e;\
                     background: {}; font-family: 'Orbitron', monospace;\
                     box-shadow: 0 0 12px {}66;",
                    sc, sc
                )}>{sl}</div>
            </div>

            // Stats grid
            <div style="display: grid; grid-template-columns: 1fr 1fr; gap: 10px; margin-bottom: 16px;">
                <div style="background: rgba(0,255,136,0.04); border: 1px solid rgba(0,255,136,0.1);\
                    border-radius: 10px; padding: 10px; text-align: center;">
                    <div style="font-size: 8px; color: #556688; font-family: 'Orbitron', monospace;\
                        letter-spacing: 2px; margin-bottom: 4px;">"LOAD"</div>
                    <div style={format!("font-size: 22px; font-weight: 800; color: {};\
                        font-family: 'Orbitron', monospace;", sc)}>{format!("{}%", load_pct)}</div>
                </div>
                <div style="background: rgba(0,204,255,0.04); border: 1px solid rgba(0,204,255,0.1);\
                    border-radius: 10px; padding: 10px; text-align: center;">
                    <div style="font-size: 8px; color: #556688; font-family: 'Orbitron', monospace;\
                        letter-spacing: 2px; margin-bottom: 4px;">"HEALTH"</div>
                    <div style="font-size: 22px; font-weight: 800; color: #00ff88;\
                        font-family: 'Orbitron', monospace;">{format!("{}%", health_pct)}</div>
                </div>
            </div>

            // Machine details
            <div style="background: rgba(255,255,255,0.02); border: 1px solid rgba(255,255,255,0.05);\
                border-radius: 10px; padding: 12px; margin-bottom: 12px;">
                <div style="font-size: 8px; color: #556688; font-family: 'Orbitron', monospace;\
                    letter-spacing: 2px; margin-bottom: 8px;">"HARDWARE"</div>
                <div style="display: flex; align-items: center; gap: 8px;">
                    <span style="font-size: 24px;">{me}</span>
                    <div>
                        <div style="font-size: 12px; color: #8899bb; font-family: 'JetBrains Mono', monospace;">
                            {agent.machine}
                        </div>
                        <div style="font-size: 11px; color: #556688; font-family: 'JetBrains Mono', monospace;">
                            {agent.ip}
                        </div>
                    </div>
                </div>
            </div>

            // Capabilities
            {(!caps.is_empty()).then(|| view! {
                <div style="background: rgba(255,255,255,0.02); border: 1px solid rgba(255,255,255,0.05);\
                    border-radius: 10px; padding: 12px;">
                    <div style="font-size: 8px; color: #556688; font-family: 'Orbitron', monospace;\
                        letter-spacing: 2px; margin-bottom: 8px;">"CAPABILITIES"</div>
                    <div style="display: flex; flex-wrap: wrap; gap: 4px;">
                        {caps.into_iter().map(|c| view! {
                            <span style={format!(
                                "padding: 2px 8px; border-radius: 10px; font-size: 9px;\
                                 background: {}11; border: 1px solid {}33; color: {};\
                                 font-family: 'JetBrains Mono', monospace;",
                                sc, sc, sc
                            )}>{c}</span>
                        }).collect::<Vec<_>>()}
                    </div>
                </div>
            })}

            // Quick Actions (T11)
            {
                let agent_id = agent.id.clone();
                let agent_name_for_msg = agent.name.clone();
                let action_status = RwSignal::new(String::new());

                let on_status = {
                    let aid = agent_id.clone();
                    move |_| {
                        let aid = aid.clone();
                        let status = action_status;
                        leptos::task::spawn_local(async move {
                            status.set("Fetching...".into());
                            match api::fetch_agent_status(&aid).await {
                                Ok(s) => {
                                    let info = if s.status.is_empty() { "unknown".to_string() } else { s.status };
                                    let active = if s.last_active.is_empty() { "n/a".to_string() } else { s.last_active };
                                    status.set(format!("Status: {} | Last active: {} | Messages: {}", info, active, s.message_count));
                                }
                                Err(e) => status.set(format!("Error: {}", e)),
                            }
                        });
                    }
                };

                let on_send = {
                    let aid = agent_id.clone();
                    let aname = agent_name_for_msg.clone();
                    move |_| {
                        let aid = aid.clone();
                        let aname = aname.clone();
                        let status = action_status;
                        if let Some(win) = web_sys::window() {
                            if let Ok(Some(msg)) = win.prompt_with_message(
                                &format!("Send message to {}:", aname)
                            ) {
                                if !msg.trim().is_empty() {
                                    let aid = aid.clone();
                                    leptos::task::spawn_local(async move {
                                        status.set("Sending...".into());
                                        match api::agent_send(&aid, &msg).await {
                                            Ok(_) => status.set("✓ Sent".into()),
                                            Err(e) => status.set(format!("Error: {}", e)),
                                        }
                                    });
                                }
                            }
                        }
                    }
                };

                let on_restart = {
                    let aname = agent_name_for_msg.clone();
                    move |_| {
                        let aname = aname.clone();
                        let status = action_status;
                        if let Some(win) = web_sys::window() {
                            if win.confirm_with_message(&format!("Restart agent {}?", aname)).unwrap_or(false) {
                                let aname = aname.clone();
                                leptos::task::spawn_local(async move {
                                    status.set("Restarting...".into());
                                    match api::spawn_agent(&api::SpawnAgentReq {
                                        name: aname,
                                        role: None,
                                        model: None,
                                        autonomy: None,
                                        persona: None,
                                        soul: None,
                                        tools: None,
                                    }).await {
                                        Ok(_) => status.set("✓ Restart initiated".into()),
                                        Err(e) => status.set(format!("Error: {}", e)),
                                    }
                                });
                            }
                        }
                    }
                };

                view! {
                    <div style="margin-top: 12px; padding-top: 12px; border-top: 1px solid rgba(255,255,255,0.05);">
                        <div style="font-size: 8px; color: #556688; font-family: 'Orbitron', monospace;\
                            letter-spacing: 2px; margin-bottom: 8px;">"ACTIONS"</div>
                        <div style="display: flex; gap: 6px; flex-wrap: wrap;">
                            <button
                                on:click=on_status
                                style="padding: 4px 10px; border-radius: 8px; font-size: 10px;\
                                    background: rgba(0,204,255,0.08); border: 1px solid rgba(0,204,255,0.2);\
                                    color: #00ccff; cursor: pointer; font-family: 'JetBrains Mono', monospace;\
                                    transition: all 0.15s ease;"
                            >"📊 Status"</button>
                            <button
                                on:click=on_send
                                style="padding: 4px 10px; border-radius: 8px; font-size: 10px;\
                                    background: rgba(0,255,136,0.08); border: 1px solid rgba(0,255,136,0.2);\
                                    color: #00ff88; cursor: pointer; font-family: 'JetBrains Mono', monospace;\
                                    transition: all 0.15s ease;"
                            >"💬 Message"</button>
                            <button
                                on:click=on_restart
                                style="padding: 4px 10px; border-radius: 8px; font-size: 10px;\
                                    background: rgba(255,170,0,0.08); border: 1px solid rgba(255,170,0,0.2);\
                                    color: #ffaa00; cursor: pointer; font-family: 'JetBrains Mono', monospace;\
                                    transition: all 0.15s ease;"
                            >"🔄 Restart"</button>
                        </div>
                        {move || {
                            let s = action_status.get();
                            (!s.is_empty()).then(|| view! {
                                <div style="margin-top: 6px; font-size: 10px; color: #7788aa;\
                                    font-family: 'JetBrains Mono', monospace;">
                                    {s}
                                </div>
                            })
                        }}
                    </div>
                }
            }
        </div>
    }
}

// ── Chat Message Component ──────────────────────────────────

#[component]
fn ChatBubble(msg: api::PantheonRoomMessage) -> impl IntoView {
    let color = avatar_color(&msg.sender_name);
    let ini = initials(&msg.sender_name);
    let time = fmt_time(&msg.timestamp);

    view! {
        <div style="display: flex; gap: 10px; padding: 6px 0; animation: fadeIn 0.2s ease;">
            // Avatar
            <div style={format!(
                "width: 32px; height: 32px; border-radius: 8px; flex-shrink: 0;\
                 background: linear-gradient(135deg, {}33, {}66);\
                 border: 1px solid {}44; display: flex; align-items: center; justify-content: center;\
                 font-size: 11px; font-weight: 700; color: {};\
                 font-family: 'Orbitron', monospace; box-shadow: 0 0 8px {}22;",
                color, color, color, color, color
            )}>
                {ini}
            </div>
            // Content
            <div style="flex: 1; min-width: 0;">
                <div style="display: flex; align-items: baseline; gap: 8px; margin-bottom: 2px;">
                    <span style={format!(
                        "font-size: 11px; font-weight: 700; color: {};\
                         font-family: 'Orbitron', monospace; letter-spacing: 0.5px;",
                        color
                    )}>{msg.sender_name.clone()}</span>
                    <span style="font-size: 9px; color: #445566; font-family: monospace;">{time}</span>
                </div>
                <div style="font-size: 12px; color: #b0c0d0; line-height: 1.5;\
                    font-family: 'Rajdhani', 'JetBrains Mono', sans-serif; word-break: break-word;">
                    {msg.content.clone()}
                </div>
            </div>
        </div>
    }
}

// ── Main Office Page ────────────────────────────────────────

#[component]
pub fn OfficePage() -> impl IntoView {
    // Visibility — pause polling when tab is hidden
    let tab_visible = use_tab_visible();

    // Fleet state
    let agents = RwSignal::new(Vec::<DeskAgent>::new());
    let selected_agent = RwSignal::new(Option::<String>::None);
    let loading = RwSignal::new(true);
    let error = RwSignal::new(Option::<String>::None);
    let now = RwSignal::new(String::new());

    // Right panel tab: chat | rooms | missions
    let right_panel_tab = RwSignal::new("chat".to_string());

    // Chat state
    let room_id = RwSignal::new(Option::<String>::None);
    let messages = RwSignal::new(Vec::<api::PantheonRoomMessage>::new());
    let chat_input = RwSignal::new(String::new());
    let chat_loading = RwSignal::new(false);

    // Voice state
    let auto_speak = RwSignal::new(false);
    let is_recording = RwSignal::new(false);
    let voice_status = RwSignal::new(String::new());

    // Identity
    let user_id = get_local("zeus_user_id").unwrap_or_else(|| {
        let id = format!("user_{}", js_sys::Math::random().to_string().replace("0.", ""));
        set_local("zeus_user_id", &id);
        id
    });
    let user_name = get_local("zeus_nick").unwrap_or_else(|| "Operator".to_string());

    // ── Load fleet agents ────────────────────────────────
    let load_agents = {
        let agents = agents;
        let loading = loading;
        let error = error;
        let now = now;
        move || {
            spawn_local(async move {
                match api::discover_agents(None, None, None).await {
                    Ok(resp) => {
                        let desks: Vec<DeskAgent> = resp.agents.into_iter().map(DeskAgent::from).collect();
                        agents.set(desks);
                        error.set(None);
                    }
                    Err(e) => error.set(Some(e)),
                }
                loading.set(false);
                let d = js_sys::Date::new_0();
                now.set(format!("{:02}:{:02}:{:02}", d.get_hours(), d.get_minutes(), d.get_seconds()));
            });
        }
    };
    load_agents();

    // Fleet auto-refresh (pauses when tab hidden)
    {
        let load_agents = load_agents.clone();
        spawn_local(async move {
            if let Some(win) = web_sys::window() {
                let cb = Closure::wrap(Box::new(move || {
                    if tab_visible.get() { load_agents(); }
                }) as Box<dyn Fn()>);
                let handle = win.set_interval_with_callback_and_timeout_and_arguments_0(
                    cb.as_ref().unchecked_ref(), FLEET_POLL_MS,
                ).unwrap_or(-1);
                cb.forget();
                on_cleanup(move || {
                    if let Some(w) = web_sys::window() { w.clear_interval_with_handle(handle); }
                });
            }
        });
    }

    // ── Find or create "Zeus Office" room ────────────────
    {
        let room_id = room_id;
        let user_id = user_id.clone();
        let user_name = user_name.clone();
        spawn_local(async move {
            // Try to find existing office room
            if let Ok(rooms) = api::fetch_pantheon_rooms().await {
                for r in &rooms {
                    if r.name == OFFICE_ROOM_NAME {
                        room_id.set(Some(r.id.clone()));
                        // Auto-join
                        if let Err(e) = api::join_room(&r.id, &user_id, &user_name).await { web_sys::console::error_1(&format!("API error: {}", e).into()); }
                        return;
                    }
                }
            }
            // Create if not found — use API directly
            let body = serde_json::json!({
                "name": OFFICE_ROOM_NAME,
                "description": "Fleet office — all agents hang out here",
                "room_type": "public",
                "created_by": user_id,
            });
            let result: Result<serde_json::Value, String> = api::post_json("/v1/pantheon/rooms", &body).await;
            if let Ok(room) = result {
                if let Some(id_str) = room.get("id").and_then(|v| v.as_str()) {
                    let new_id = String::from(id_str);
                    room_id.set(Some(new_id.clone()));
                    if let Err(e) = api::join_room(&new_id, &user_id, &user_name).await { web_sys::console::error_1(&format!("API error: {}", e).into()); }
                }
            }
        });
    }

    // Clone user_id before it gets moved into spawn_local closures
    let _user_id_for_cleanup = user_id.clone();

    // ── Poll chat messages ───────────────────────────────
    {
        let room_id = room_id;
        let messages = messages;
        let auto_speak = auto_speak;
        spawn_local(async move {
            // Wait for room to be discovered
            loop {
                gloo_timers::future::sleep(std::time::Duration::from_millis(500)).await;
                if room_id.get().is_some() { break; }
            }
            // Start polling
            if let Some(win) = web_sys::window() {
                let cb = Closure::wrap(Box::new(move || {
                    if !tab_visible.get() { return; } // Skip when tab hidden
                    let rid = room_id.get();
                    if let Some(rid) = rid {
                        let messages = messages;
                        let auto_speak = auto_speak;
                        spawn_local(async move {
                            if let Ok(msgs) = api::fetch_room_messages(&rid, 50).await {
                                let prev_len = messages.get_untracked().len();
                                messages.set(msgs.clone());
                                scroll_chat_to_bottom();
                                // Auto-speak new messages
                                if auto_speak.get_untracked() && msgs.len() > prev_len {
                                    if let Some(last) = msgs.last() {
                                        let text = last.content.clone();
                                        spawn_local(async move {
                                            if let Ok(audio) = api::tts_synthesize(&text).await {
                                                play_audio_bytes(&audio);
                                            }
                                        });
                                    }
                                }
                            }
                        });
                    }
                }) as Box<dyn Fn()>);
                let handle = win.set_interval_with_callback_and_timeout_and_arguments_0(
                    cb.as_ref().unchecked_ref(), POLL_INTERVAL_MS,
                ).unwrap_or(-1);
                cb.forget();
                on_cleanup(move || {
                    if let Some(w) = web_sys::window() { w.clear_interval_with_handle(handle); }
                    // Leave room on cleanup to avoid presence leak (leave_room not yet in API)
                });
                // Also do initial load
                if let Some(rid) = room_id.get() {
                    if let Ok(msgs) = api::fetch_room_messages(&rid, 50).await {
                        messages.set(msgs);
                        scroll_chat_to_bottom();
                    }
                }
            }
        });
    }

    // ── Send chat message ────────────────────────────────
    let send_message = {
        let chat_input = chat_input;
        let room_id = room_id;
        let user_id = user_id.clone();
        let user_name = user_name.clone();
        let messages = messages;
        move |_| {
            let text = chat_input.get();
            if text.trim().is_empty() { return; }
            let rid = match room_id.get() {
                Some(r) => r,
                None => return,
            };
            chat_input.set(String::new());
            chat_loading.set(true);
            let uid = user_id.clone();
            let uname = user_name.clone();
            spawn_local(async move {
                let req = api::SendRoomMessageRequest {
                    sender_id: uid,
                    sender_name: uname,
                    content: text,
                    message_type: "chat".to_string(),
                    reply_to: None,
                };
                if let Ok(msg) = api::send_room_message(&rid, &req).await {
                    messages.update(|m| m.push(msg));
                    scroll_chat_to_bottom();
                }
                chat_loading.set(false);
            });
        }
    };
    let send_message2 = send_message.clone();

    // ── Voice: record & transcribe ───────────────────────
    let toggle_recording = {
        let is_recording = is_recording;
        let voice_status = voice_status;
        let chat_input = chat_input;
        move |_| {
            if is_recording.get() {
                // Stop recording — handled by JS MediaRecorder onstop
                is_recording.set(false);
                voice_status.set(String::new());
            } else {
                is_recording.set(true);
                voice_status.set("Recording...".to_string());
                // Start MediaRecorder
                spawn_local(async move {
                    let result = record_and_transcribe().await;
                    is_recording.set(false);
                    match result {
                        Ok(text) => {
                            voice_status.set(String::new());
                            if !text.trim().is_empty() {
                                chat_input.set(text);
                            }
                        }
                        Err(e) => voice_status.set(format!("STT error: {}", e)),
                    }
                });
            }
        }
    };

    // ── TTS speak button ─────────────────────────────────
    let speak_last = {
        let messages = messages;
        let voice_status = voice_status;
        move |_| {
            let msgs = messages.get();
            if let Some(last) = msgs.last() {
                let text = last.content.clone();
                voice_status.set("Speaking...".to_string());
                spawn_local(async move {
                    match api::tts_synthesize(&text).await {
                        Ok(audio) => {
                            play_audio_bytes(&audio);
                            voice_status.set(String::new());
                        }
                        Err(e) => voice_status.set(format!("TTS error: {}", e)),
                    }
                });
            }
        }
    };

    // NOTE: zeusOfficeUpdateAgents, zeusOfficeMakeAllDraggable, zeusOfficeShowMemo
    // were removed — these JS functions don't exist in game.js (dead interop, T3/S96)

    // SSE stream: connect to /v1/office/stream for real-time office events
    spawn_local(async move {
        let es = web_sys::EventSource::new("/v1/office/stream").ok();
        if let Some(es) = es {
            let on_msg = wasm_bindgen::closure::Closure::wrap(Box::new(move |e: web_sys::MessageEvent| {
                if let Some(data) = e.data().as_string() {
                    let code = format!(
                        "if(window.zeusOfficeSSEEvent)window.zeusOfficeSSEEvent({})",
                        data
                    );
                    let _ = js_sys::eval(&code);
                }
            }) as Box<dyn FnMut(_)>);
            es.set_onmessage(Some(on_msg.as_ref().unchecked_ref()));
            on_msg.forget();
        }
    });

    // T4/S96: Load Phaser scripts dynamically so they work on SPA route transitions.
    // Static <script> tags only fire on full page load; this approach checks if scripts
    // are already loaded (by src URL) and injects them in sequence if not.
    spawn_local(async move {
        let window = web_sys::window().unwrap();
        let document = window.document().unwrap();
        let head = document.head().unwrap();

        let scripts = [
            "/office-game/phaser-3.80.1.min.js",
            "/office-game/layout.js",
            "/office-game/game.js",
        ];

        for src in &scripts {
            // Skip if already loaded
            let already_loaded = {
                let existing = document.query_selector(&format!("script[src=x27{}x27]", src)).ok().flatten();
                existing.is_some()


            };
            if already_loaded { continue; }

            // Create and inject script tag
            let script = document.create_element("script").unwrap();
            script.set_attribute("src", src).unwrap();

            // Wait for this script to load before injecting the next one (order matters)
            let (tx, rx) = futures::channel::oneshot::channel::<()>();
            let tx = std::cell::Cell::new(Some(tx));
            let onload = Closure::wrap(Box::new(move || {
                if let Some(t) = tx.take() { let _ = t.send(()); }
            }) as Box<dyn FnMut()>);
            let script_el = script.dyn_ref::<web_sys::HtmlElement>().unwrap();
            script_el.set_onload(Some(onload.as_ref().unchecked_ref()));
            onload.forget();

            head.append_child(&script).unwrap();
            let _ = rx.await;
        }
    });

    view! {
        <div style="display: flex; height: 100vh; background: #020810; overflow: hidden;">

            // ════════════════════════════════════════════
            // LEFT PANEL — Agent Floor
            // ════════════════════════════════════════════
            <div style="flex: 1; display: flex; flex-direction: column; overflow: hidden;\
                border-right: 1px solid #0a1525;">

                // ── Header ─────────────────────────────
                <div style="padding: 16px 20px; border-bottom: 1px solid #0a1525;\
                    background: linear-gradient(90deg, rgba(0,255,136,0.03), rgba(0,204,255,0.03));">
                    <div style="display: flex; align-items: center; justify-content: space-between;">
                        <div>
                            <h1 style="font-family: 'Orbitron', monospace; font-size: 18px; font-weight: 800;\
                                color: #e0e8f0; letter-spacing: 3px; margin: 0;\
                                text-shadow: 0 0 20px rgba(0,255,136,0.3);">
                                "\u{1f3e2} ZEUS OFFICE"
                            </h1>
                            <p style="font-size: 10px; color: #445566; margin: 4px 0 0 0;\
                                font-family: 'Orbitron', monospace; letter-spacing: 2px;">
                                "FLEET COMMAND CENTER \u{2014} STAR OFFICE"
                            </p>
                        </div>
                        <div style="display: flex; align-items: center; gap: 12px;">
                            <span style="font-size: 9px; color: #334455; font-family: 'JetBrains Mono', monospace;">
                                {move || {
                                    let t = now.get();
                                    if t.is_empty() { String::new() } else { format!("\u{1f4e1} {}", t) }
                                }}
                            </span>
                            <button
                                style="background: rgba(0,255,136,0.08); border: 1px solid rgba(0,255,136,0.2);\
                                    color: #00ff88; padding: 6px 14px; border-radius: 8px; cursor: pointer;\
                                    font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 1px;\
                                    transition: all 0.2s ease; box-shadow: 0 0 8px rgba(0,255,136,0.1);"
                                on:click=move |_| load_agents()
                            >
                                "REFRESH"
                            </button>
                        </div>
                    </div>
                </div>

                // ── Fleet stats bar ────────────────────
                <div style="padding: 12px 20px; border-bottom: 1px solid #0a1525;\
                    background: rgba(0,0,0,0.3);">
                    {move || {
                        let a = agents.get();
                        let total = a.len();
                        let online = a.iter().filter(|d| matches!(d.status.to_lowercase().as_str(), "online" | "active" | "healthy")).count();
                        let busy = a.iter().filter(|d| matches!(d.status.to_lowercase().as_str(), "busy" | "working" | "executing")).count();
                        let offline = a.iter().filter(|d| matches!(d.status.to_lowercase().as_str(), "offline" | "down" | "error")).count();
                        let avg_load = if total > 0 { a.iter().map(|d| d.load).sum::<f64>() / total as f64 * 100.0 } else { 0.0 };

                        view! {
                            <div style="display: flex; gap: 24px; align-items: center;">
                                <div>
                                    <div class="font-orbitron text-[10px] tracking-[3px] text-white/70 uppercase">"FLEET"</div>
                                    <div style="font-size: 20px; font-weight: 800; color: #e0e8f0;\
                                        font-family: 'Orbitron', monospace;">{total}</div>
                                </div>
                                <div>
                                    <div class="font-orbitron text-[10px] tracking-[3px] text-white/70 uppercase">"ONLINE"</div>
                                    <div style="font-size: 20px; font-weight: 800; color: #00ff88;\
                                        font-family: 'Orbitron', monospace; text-shadow: 0 0 10px rgba(0,255,136,0.4);">{online}</div>
                                </div>
                                <div>
                                    <div class="font-orbitron text-[10px] tracking-[3px] text-white/70 uppercase">"BUSY"</div>
                                    <div style="font-size: 20px; font-weight: 800; color: #ffcc00;\
                                        font-family: 'Orbitron', monospace; text-shadow: 0 0 10px rgba(255,204,0,0.4);">{busy}</div>
                                </div>
                                <div>
                                    <div class="font-orbitron text-[10px] tracking-[3px] text-white/70 uppercase">"OFFLINE"</div>
                                    <div style="font-size: 20px; font-weight: 800; color: #ff4466;\
                                        font-family: 'Orbitron', monospace;">{offline}</div>
                                </div>
                                <div style="margin-left: auto;">
                                    <div class="font-orbitron text-[10px] tracking-[3px] text-white/70 uppercase">"AVG LOAD"</div>
                                    <div style="font-size: 20px; font-weight: 800; color: #00ccff;\
                                        font-family: 'Orbitron', monospace; text-shadow: 0 0 10px rgba(0,204,255,0.3);">{format!("{:.0}%", avg_load)}</div>
                                </div>
                            </div>
                        }
                    }}
                </div>

                // ── Phaser canvas + Detail panel ──────────
                <div style="flex: 1; display: flex; flex-direction: column; padding: 16px; min-height: 0;">
                    // Loading / Error
                    {move || {
                        if loading.get() && agents.get().is_empty() {
                            return Some(view! {
                                <div style="text-align: center; padding: 60px 0; color: #445566;\
                                    font-family: 'Orbitron', monospace; font-size: 12px; letter-spacing: 3px;">
                                    <div style="font-size: 36px; margin-bottom: 12px; animation: neonPulse 1.5s infinite;">"\u{1f4e1}"</div>
                                    "SCANNING FLEET..."
                                </div>
                            }.into_any());
                        }
                        if let Some(e) = error.get() {
                            return Some(view! {
                                <div style="text-align: center; padding: 24px; color: #ff4466;\
                                    background: rgba(255,68,102,0.05); border: 1px solid rgba(255,68,102,0.2);\
                                    border-radius: 12px; font-family: 'JetBrains Mono', monospace; font-size: 12px;">
                                    {format!("\u{26a0}\u{fe0f} {}", e)}
                                </div>
                            }.into_any());
                        }
                        None
                    }}

                    // Detail panel (when agent selected)
                    {move || {
                        let sel = selected_agent.get();
                        let a = agents.get();
                        if let Some(ref sid) = sel {
                            if let Some(agent) = a.iter().find(|d| d.id == *sid) {
                                return Some(view! {
                                    <div style="margin-bottom: 16px;">
                                        <AgentDetailPanel agent=agent.clone() />
                                    </div>
                                }.into_any());
                            }
                        }
                        None
                    }}

                    // S62: Phaser pixel art canvas — agents render as walking sprites here.
                    // Hoisted OUT of the reactive block so Phaser's mount target is stable
                    // and doesn't re-create on every agents.set(). flex:1 fills the parent
                    // flex column; agents listed textually in the right AGENTS panel.
                    <div id="zeus-office-canvas"
                        style="flex: 1; width: 100%; min-height: 0; border-radius: 12px; overflow: hidden;\
                               border: 1px solid rgba(0,255,136,0.1); background: #020810;" />

                    // Empty-state overlay (only shown when no agents AND not loading)
                    {move || {
                        let a = agents.get();
                        if a.is_empty() && !loading.get() {
                            return Some(view! {
                                <div style="text-align: center; padding: 16px 0; color: #334455;\
                                    font-family: 'Orbitron', monospace; font-size: 11px; letter-spacing: 2px;">
                                    "NO AGENTS DETECTED — START A GATEWAY"
                                </div>
                            }.into_any());
                        }
                        None
                    }}
                </div>
            </div>

            // ════════════════════════════════════════════
            // RIGHT PANEL — Office Chat (Pantheon)
            // ════════════════════════════════════════════
            <div style="width: 380px; display: flex; flex-direction: column;\
                background: rgba(5,8,18,0.98); border-left: 1px solid #0a1525;">

                // ── Chat header ────────────────────────
                <div style="padding: 14px 16px; border-bottom: 1px solid #0a1525;\
                    background: linear-gradient(90deg, rgba(0,204,255,0.04), rgba(204,68,255,0.04));">
                    <div style="display: flex; align-items: center; justify-content: space-between;">
                        <div>
                            <div style="display: flex; gap: 8px; align-items: center;">
                                <button
                                    style=move || format!("font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 1px; padding: 4px 10px; border-radius: 6px; cursor: pointer; border: 1px solid {}; background: {}; color: {};",
                                        if right_panel_tab.get() == "chat" { "rgba(0,204,255,0.4)" } else { "rgba(255,255,255,0.05)" },
                                        if right_panel_tab.get() == "chat" { "rgba(0,204,255,0.1)" } else { "transparent" },
                                        if right_panel_tab.get() == "chat" { "#00ccff" } else { "#556688" })
                                    on:click=move |_| right_panel_tab.set("chat".into())
                                >"\u{1f4ac} Chat"</button>
                                <button
                                    style=move || format!("font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 1px; padding: 4px 10px; border-radius: 6px; cursor: pointer; border: 1px solid {}; background: {}; color: {};",
                                        if right_panel_tab.get() == "rooms" { "rgba(0,255,136,0.4)" } else { "rgba(255,255,255,0.05)" },
                                        if right_panel_tab.get() == "rooms" { "rgba(0,255,136,0.1)" } else { "transparent" },
                                        if right_panel_tab.get() == "rooms" { "#00ff88" } else { "#556688" })
                                    on:click=move |_| right_panel_tab.set("rooms".into())
                                >"\u{1f3e0} Rooms"</button>
                                <button
                                    style=move || format!("font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 1px; padding: 4px 10px; border-radius: 6px; cursor: pointer; border: 1px solid {}; background: {}; color: {};",
                                        if right_panel_tab.get() == "missions" { "rgba(255,170,0,0.4)" } else { "rgba(255,255,255,0.05)" },
                                        if right_panel_tab.get() == "missions" { "rgba(255,170,0,0.1)" } else { "transparent" },
                                        if right_panel_tab.get() == "missions" { "#ffaa00" } else { "#556688" })
                                    on:click=move |_| right_panel_tab.set("missions".into())
                                >"\u{1f3af} Missions"</button>
                            </div>
                            <div style="font-size: 9px; color: #334455; font-family: monospace; margin-top: 2px;">
                                {move || {
                                    let count = messages.get().len();
                                    format!("{} messages", count)
                                }}
                            </div>
                        </div>
                        // Auto-speak toggle
                        <button
                            style={move || format!(
                                "background: {}; border: 1px solid {}; color: {};\
                                 padding: 4px 10px; border-radius: 6px; cursor: pointer;\
                                 font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 1px;\
                                 transition: all 0.2s ease;",
                                if auto_speak.get() { "rgba(0,255,136,0.15)" } else { "rgba(255,255,255,0.03)" },
                                if auto_speak.get() { "rgba(0,255,136,0.3)" } else { "rgba(255,255,255,0.08)" },
                                if auto_speak.get() { "#00ff88" } else { "#556677" },
                            )}
                            on:click=move |_| auto_speak.update(|v| *v = !*v)
                        >
                            {move || if auto_speak.get() { "\u{1f50a} AUTO" } else { "\u{1f507} MUTE" }}
                        </button>
                    </div>
                </div>

                // ── Panel content (conditional) ─────────────────
                <Show when=move || right_panel_tab.get() == "chat">
                <div id="office-chat-feed" style="flex: 1; overflow-y: auto; padding: 12px 16px;">
                    {move || {
                        let msgs = messages.get();
                        if msgs.is_empty() {
                            return view! {
                                <div style="text-align: center; padding: 40px 0; color: #334455;\
                                    font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 2px;">
                                    <div style="font-size: 28px; margin-bottom: 10px; opacity: 0.5;">"\u{1f4ac}"</div>
                                    "WAITING FOR TRANSMISSIONS"
                                </div>
                            }.into_any();
                        }
                        view! {
                            <div>
                                {msgs.into_iter().map(|m| {
                                    view! { <ChatBubble msg=m /> }
                                }).collect::<Vec<_>>()}
                            </div>
                        }.into_any()
                    }}
                </div>

                </Show>

                // ── Rooms panel ─────────────────
                <Show when=move || right_panel_tab.get() == "rooms">
                    <div style="flex: 1; overflow-y: auto; padding: 12px 16px;">
                        <RoomsPanel />
                    </div>
                </Show>

                // ── Missions panel ─────────────────
                <Show when=move || right_panel_tab.get() == "missions">
                    <div style="flex: 1; overflow-y: auto; padding: 12px 16px;">
                        <MissionsPanel />
                    </div>
                </Show>

                // ── Voice status ───────────────────────
                {move || {
                    let vs = voice_status.get();
                    (!vs.is_empty()).then(|| view! {
                        <div style="padding: 4px 16px; font-size: 9px; color: #00ccff;\
                            font-family: 'Orbitron', monospace; letter-spacing: 1px;\
                            background: rgba(0,204,255,0.05); border-top: 1px solid rgba(0,204,255,0.1);">
                            {vs}
                        </div>
                    })
                }}

                // ── Chat input bar ─────────────────────
                <div style="padding: 12px 16px; border-top: 1px solid #0a1525;\
                    background: rgba(0,0,0,0.3);">
                    <div style="display: flex; align-items: center; gap: 8px;">
                        // Mic button (STT)
                        <button
                            style={move || format!(
                                "width: 36px; height: 36px; border-radius: 50%; border: 1px solid {};\
                                 background: {}; cursor: pointer; display: flex; align-items: center;\
                                 justify-content: center; font-size: 16px; transition: all 0.2s ease;\
                                 box-shadow: 0 0 {}px {};",
                                if is_recording.get() { "rgba(255,68,102,0.5)" } else { "rgba(0,204,255,0.2)" },
                                if is_recording.get() { "rgba(255,68,102,0.15)" } else { "rgba(0,204,255,0.05)" },
                                if is_recording.get() { "12" } else { "4" },
                                if is_recording.get() { "rgba(255,68,102,0.3)" } else { "rgba(0,204,255,0.1)" },
                            )}
                            on:click=toggle_recording
                            title="Record voice (STT)"
                        >
                            {move || if is_recording.get() { "\u{1f534}" } else { "\u{1f3a4}" }}
                        </button>

                        // Text input
                        <input
                            type="text"
                            placeholder="Transmit to office..."
                            prop:value=move || chat_input.get()
                            on:input=move |ev| chat_input.set(event_target_value(&ev))
                            on:keydown=move |ev: web_sys::KeyboardEvent| {
                                if ev.key() == "Enter" && !ev.shift_key() {
                                    ev.prevent_default();
                                    send_message2(());
                                }
                            }
                            style="flex: 1; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,255,255,0.08);\
                                border-radius: 10px; padding: 8px 14px; color: #b0c0d0; font-size: 12px;\
                                font-family: 'Rajdhani', 'JetBrains Mono', sans-serif;\
                                outline: none; transition: border-color 0.2s ease;"
                        />

                        // TTS speak button
                        <button
                            style="width: 36px; height: 36px; border-radius: 50%;\
                                border: 1px solid rgba(204,68,255,0.2); background: rgba(204,68,255,0.05);\
                                cursor: pointer; display: flex; align-items: center; justify-content: center;\
                                font-size: 16px; transition: all 0.2s ease;\
                                box-shadow: 0 0 4px rgba(204,68,255,0.1);"
                            on:click=speak_last
                            title="Speak last message (TTS)"
                        >
                            "\u{1f50a}"
                        </button>

                        // Send button
                        <button
                            style=move || format!("width: 36px; height: 36px; border-radius: 50%;\
                                border: 1px solid rgba(0,255,136,0.3); background: {};\
                                cursor: {}; display: flex; align-items: center; justify-content: center;\
                                font-size: 16px; transition: all 0.2s ease;\
                                box-shadow: 0 0 6px rgba(0,255,136,0.15); opacity: {};",
                                if chat_loading.get() { "rgba(0,255,136,0.05)" } else { "rgba(0,255,136,0.1)" },
                                if chat_loading.get() { "not-allowed" } else { "pointer" },
                                if chat_loading.get() { "0.5" } else { "1.0" },
                            )
                            on:click=move |_| { if !chat_loading.get() { send_message(()); } }
                            title="Send message"
                            disabled=move || chat_loading.get()
                        >
                            {move || if chat_loading.get() { "⏳" } else { "\u{1f680}" }}
                        </button>
                    </div>
                </div>
            </div>
        </div>

        // ── Global Styles ─────────────────────────────────
        <style>"
            @keyframes neonPulse {
                0%, 100% { opacity: 1; }
                50% { opacity: 0.4; }
            }
            @keyframes fadeIn {
                from { opacity: 0; transform: translateY(4px); }
                to { opacity: 1; transform: translateY(0); }
            }
            .agent-card:hover > div {
                transform: translateY(-3px);
                box-shadow: 0 0 24px rgba(0,255,136,0.15), inset 0 1px 0 rgba(255,255,255,0.06) !important;
            }
            #office-chat-feed::-webkit-scrollbar { width: 4px; }
            #office-chat-feed::-webkit-scrollbar-track { background: transparent; }
            #office-chat-feed::-webkit-scrollbar-thumb { background: #1a2535; border-radius: 2px; }
        "</style>
    }
}

// ── Voice Helpers ───────────────────────────────────────────

fn play_audio_bytes(audio: &[u8]) {
    let array = js_sys::Uint8Array::new_with_length(audio.len() as u32);
    array.copy_from(audio);
    let blob_parts = js_sys::Array::new();
    blob_parts.push(&array.buffer());
    if let Ok(blob) = web_sys::Blob::new_with_u8_array_sequence_and_options(
        &blob_parts,
        web_sys::BlobPropertyBag::new().type_("audio/mp3"),
    ) {
        if let Ok(url) = web_sys::Url::create_object_url_with_blob(&blob)
            && let Ok(audio) = web_sys::HtmlAudioElement::new_with_src(&url) {
                let _ = audio.play();
            }
    }
}

async fn record_and_transcribe() -> Result<String, String> {
    use wasm_bindgen::JsValue;
    use js_sys::Promise;
    use wasm_bindgen_futures::JsFuture;

    // Get user media
    let window = web_sys::window().ok_or("No window")?;
    let navigator = window.navigator();
    let media_devices = navigator.media_devices().map_err(|_| "No media devices")?;

    let mut constraints = web_sys::MediaStreamConstraints::new();
    constraints.audio(&JsValue::TRUE);
    constraints.video(&JsValue::FALSE);

    let stream_promise: Promise = media_devices
        .get_user_media_with_constraints(&constraints)
        .map_err(|_| "getUserMedia failed")?;
    let stream: web_sys::MediaStream = JsFuture::from(stream_promise)
        .await
        .map_err(|_| "Stream error")?
        .dyn_into()
        .map_err(|_| "Not a MediaStream")?;

    // Record for 5 seconds
    let recorder = web_sys::MediaRecorder::new_with_media_stream(&stream)
        .map_err(|_| "MediaRecorder failed")?;

    let chunks: std::rc::Rc<std::cell::RefCell<Vec<JsValue>>> =
        std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));

    let chunks_clone = chunks.clone();
    let ondataavailable = Closure::wrap(Box::new(move |ev: web_sys::BlobEvent| {
        if let Some(blob) = ev.data() {
            chunks_clone.borrow_mut().push(blob.into());
        }
    }) as Box<dyn FnMut(web_sys::BlobEvent)>);
    recorder.set_ondataavailable(Some(ondataavailable.as_ref().unchecked_ref()));
    ondataavailable.forget();

    recorder.start().map_err(|_| "Start failed")?;

    // Wait 5s then stop
    gloo_timers::future::sleep(std::time::Duration::from_secs(5)).await;

    // Stop recording (ignore error if already stopped)
    let _ = recorder.stop();

    // Small delay for onstop to fire
    gloo_timers::future::sleep(std::time::Duration::from_millis(300)).await;

    // Stop all tracks
    for track in stream.get_tracks().iter() {
        if let Ok(track) = track.dyn_into::<web_sys::MediaStreamTrack>() {
            track.stop();
        }
    }

    // Build blob from chunks
    let parts = js_sys::Array::new();
    for chunk in chunks.borrow().iter() {
        parts.push(chunk);
    }

    let blob = web_sys::Blob::new_with_blob_sequence_and_options(
        &parts,
        web_sys::BlobPropertyBag::new().type_("audio/webm"),
    ).map_err(|_| "Blob creation failed")?;

    // Read blob as ArrayBuffer
    let ab_promise = blob.array_buffer();
    let ab = JsFuture::from(ab_promise).await.map_err(|_| "ArrayBuffer failed")?;
    let uint8 = js_sys::Uint8Array::new(&ab);
    let mut audio_bytes = vec![0u8; uint8.length() as usize];
    uint8.copy_to(&mut audio_bytes);

    if audio_bytes.is_empty() {
        return Err("No audio captured".to_string());
    }

    // Send to Whisper STT
    api::stt_transcribe_with_mime(&audio_bytes, "audio/webm").await
}

// ═══════════════════════════════════════════════════
// S63 Track A: Pantheon Rooms + Missions panels
// ═══════════════════════════════════════════════════

#[component]
fn RoomsPanel() -> impl IntoView {
    let rooms = RwSignal::new(Vec::<api::PantheonRoom>::new());
    let loading = RwSignal::new(true);

    spawn_local(async move {
        if let Ok(r) = api::fetch_pantheon_rooms().await {
            rooms.set(r);
        }
        loading.set(false);
    });

    view! {
        <div>
            <div style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px;\
                color: rgba(0,255,136,0.6); margin-bottom: 12px;">"WAR ROOMS"</div>
            {move || {
                if loading.get() {
                    return view! { <div style="color: #445566; font-size: 11px;">"Loading rooms..."</div> }.into_any();
                }
                let rs = rooms.get();
                if rs.is_empty() {
                    return view! { <div style="color: #445566; font-size: 11px;">"No rooms yet. Create one in Pantheon."</div> }.into_any();
                }
                view! {
                    <div style="display: flex; flex-direction: column; gap: 6px;">
                        {rs.into_iter().map(|r| {
                            let room_type_icon = match r.room_type.as_str() {
                                "public" => "\u{1F30D}",
                                "private" => "\u{1F512}",
                                "mission" => "\u{1F3AF}",
                                _ => "\u{1F4AC}",
                            };
                            let name = r.name.clone();
                            let id_click = r.id.clone();
                            let member_count = r.member_count;
                            view! {
                                <div style="display: flex; align-items: center; gap: 8px; padding: 8px 10px;\
                                    background: rgba(255,255,255,0.02); border-radius: 6px;\
                                    border: 1px solid rgba(0,255,136,0.06); cursor: pointer;"
                                    on:click=move |_| {
                                        let _ = web_sys::window().and_then(|w| w.location().assign(&format!("/pantheon?room={}", id_click)).ok());
                                    }
                                >
                                    <span style="font-size: 14px;">{room_type_icon}</span>
                                    <div style="flex: 1;">
                                        <div style="font-size: 12px; color: rgba(255,245,240,0.85); font-weight: 600;">{name}</div>
                                        <div style="font-size: 9px; color: #445566;">{format!("{} members", member_count)}</div>
                                    </div>
                                </div>
                            }
                        }).collect::<Vec<_>>()}
                    </div>
                }.into_any()
            }}
        </div>
    }
}

#[component]
fn MissionsPanel() -> impl IntoView {
    let missions = RwSignal::new(Vec::<api::PantheonMission>::new());
    let loading = RwSignal::new(true);
    let show_modal = RwSignal::new(false);
    let new_title = RwSignal::new(String::new());
    let new_desc = RwSignal::new(String::new());
    let creating = RwSignal::new(false);
    let create_error = RwSignal::new(Option::<String>::None);

    let load_missions = move || {
        spawn_local(async move {
            if let Ok(m) = api::fetch_pantheon_missions().await {
                missions.set(m);
            }
            loading.set(false);
        });
    };
    load_missions();

    let create_mission = move |_| {
        let title = new_title.get();
        let desc = new_desc.get();
        if title.trim().is_empty() {
            create_error.set(Some("Title is required".into()));
            return;
        }
        creating.set(true);
        create_error.set(None);
        spawn_local(async move {
            let body = serde_json::json!({
                "goal": title,
                "description": desc,
                "status": "planning",
            });
            let result: Result<serde_json::Value, String> = api::post_json("/v1/pantheon/missions", &body).await;
            creating.set(false);
            match result {
                Ok(_) => {
                    show_modal.set(false);
                    new_title.set(String::new());
                    new_desc.set(String::new());
                    loading.set(true);
                    if let Ok(m) = api::fetch_pantheon_missions().await {
                        missions.set(m);
                    }
                    loading.set(false);
                }
                Err(e) => create_error.set(Some(e)),
            }
        });
    };

    view! {
        <div>
            // Header row with "New Mission" button
            <div style="display: flex; align-items: center; justify-content: space-between; margin-bottom: 12px;">
                <div style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px;\
                    color: rgba(255,170,0,0.6);">"ACTIVE MISSIONS"</div>
                <button
                    style="padding: 4px 12px; border-radius: 8px; font-size: 9px; font-weight: 700;\
                        font-family: 'Orbitron', monospace; letter-spacing: 1px; cursor: pointer;\
                        background: rgba(255,170,0,0.12); border: 1px solid rgba(255,170,0,0.35);\
                        color: #ffaa00; transition: all 0.2s ease;\
                        box-shadow: 0 0 8px rgba(255,170,0,0.1);"
                    on:click=move |_| { show_modal.set(true); create_error.set(None); }
                >
                    "+ NEW MISSION"
                </button>
            </div>

            // ── New Mission Modal ──────────────────────────────
            {move || show_modal.get().then(|| view! {
                <div style="position: fixed; inset: 0; z-index: 9999; display: flex;\
                    align-items: center; justify-content: center;\
                    background: rgba(0,0,0,0.75); backdrop-filter: blur(4px);">
                    <div style="background: linear-gradient(135deg, rgba(12,18,36,0.99) 0%, rgba(6,10,24,1) 100%);\
                        border: 1px solid rgba(255,170,0,0.3); border-radius: 16px; padding: 24px;\
                        width: 360px; box-shadow: 0 0 40px rgba(255,170,0,0.12);">

                        // Modal header
                        <div style="display: flex; align-items: center; justify-content: space-between; margin-bottom: 20px;">
                            <div style="font-family: 'Orbitron', monospace; font-size: 13px; font-weight: 700;\
                                color: #ffaa00; letter-spacing: 2px;">"\u{1f3af} NEW MISSION"</div>
                            <button
                                style="background: none; border: none; color: #556688; cursor: pointer; font-size: 18px;"
                                on:click=move |_| show_modal.set(false)
                            >"\u{00d7}"</button>
                        </div>

                        // Title input
                        <div style="margin-bottom: 12px;">
                            <label style="display: block; font-family: 'Orbitron', monospace; font-size: 8px;\
                                color: #556688; letter-spacing: 2px; margin-bottom: 6px;">"MISSION TITLE *"</label>
                            <input
                                type="text"
                                placeholder="e.g. Deploy star office v2"
                                prop:value=move || new_title.get()
                                on:input=move |ev| new_title.set(event_target_value(&ev))
                                style="width: 100%; box-sizing: border-box; background: rgba(255,255,255,0.03);\
                                    border: 1px solid rgba(255,170,0,0.2); border-radius: 8px;\
                                    padding: 8px 12px; color: #d0d8e8; font-size: 12px;\
                                    font-family: 'JetBrains Mono', monospace; outline: none;"
                            />
                        </div>

                        // Description textarea
                        <div style="margin-bottom: 16px;">
                            <label style="display: block; font-family: 'Orbitron', monospace; font-size: 8px;\
                                color: #556688; letter-spacing: 2px; margin-bottom: 6px;">"DESCRIPTION"</label>
                            <textarea
                                placeholder="Describe the mission objective..."
                                prop:value=move || new_desc.get()
                                on:input=move |ev| new_desc.set(event_target_value(&ev))
                                rows="3"
                                style="width: 100%; box-sizing: border-box; background: rgba(255,255,255,0.03);\
                                    border: 1px solid rgba(255,170,0,0.15); border-radius: 8px;\
                                    padding: 8px 12px; color: #d0d8e8; font-size: 12px;\
                                    font-family: 'JetBrains Mono', monospace; outline: none;\
                                    resize: vertical; min-height: 72px;"
                            />
                        </div>

                        // Error
                        {move || create_error.get().map(|e| view! {
                            <div style="margin-bottom: 10px; padding: 6px 10px; border-radius: 6px;\
                                background: rgba(255,68,102,0.08); border: 1px solid rgba(255,68,102,0.2);\
                                color: #ff4466; font-size: 10px; font-family: 'JetBrains Mono', monospace;">
                                {e}
                            </div>
                        })}

                        // Action buttons
                        <div style="display: flex; gap: 8px; justify-content: flex-end;">
                            <button
                                style="padding: 7px 16px; border-radius: 8px; font-size: 10px;\
                                    font-family: 'Orbitron', monospace; letter-spacing: 1px; cursor: pointer;\
                                    background: transparent; border: 1px solid rgba(255,255,255,0.08); color: #556688;"
                                on:click=move |_| show_modal.set(false)
                            >"CANCEL"</button>
                            <button
                                style=move || format!("padding: 7px 20px; border-radius: 8px; font-size: 10px;\
                                    font-family: 'Orbitron', monospace; letter-spacing: 1px; cursor: pointer;\
                                    background: {}; border: 1px solid rgba(255,170,0,0.4); color: #0a0f1e; font-weight: 800;\
                                    box-shadow: 0 0 12px rgba(255,170,0,0.2);",
                                    if creating.get() { "rgba(255,170,0,0.4)" } else { "#ffaa00" })
                                on:click=create_mission
                                prop:disabled=move || creating.get()
                            >
                                {move || if creating.get() { "CREATING..." } else { "CREATE" }}
                            </button>
                        </div>
                    </div>
                </div>
            })}

            {move || {
                if loading.get() {
                    return view! { <div style="color: #445566; font-size: 11px;">"Loading missions..."</div> }.into_any();
                }
                let ms = missions.get();
                if ms.is_empty() {
                    return view! { <div style="color: #445566; font-size: 11px;">"No active missions. Launch one in Pantheon."</div> }.into_any();
                }
                view! {
                    <div style="display: flex; flex-direction: column; gap: 6px;">
                        {ms.into_iter().map(|m| {
                            let status_color = match m.status.as_str() {
                                "active" => "#00ff88",
                                "planning" | "assembling" => "#ffaa00",
                                "reviewing" => "#00ccff",
                                "completed" => "#22c55e",
                                _ => "#556688",
                            };
                            let goal = m.goal.clone();
                            let status = m.status.clone();
                            let mid = m.id.clone();
                            view! {
                                <div style="padding: 8px 10px; background: rgba(255,255,255,0.02);\
                                    border-radius: 6px; border: 1px solid rgba(255,170,0,0.06); cursor: pointer;"
                                    on:click=move |_| {
                                        let _ = web_sys::window().and_then(|w| w.location().assign(&format!("/missions/{}", mid)).ok());
                                    }
                                >
                                    <div style="display: flex; align-items: center; gap: 6px;">
                                        <div style={format!("width: 6px; height: 6px; border-radius: 50%; background: {};", status_color)} />
                                        <div style="font-size: 12px; color: rgba(255,245,240,0.85); font-weight: 600; flex: 1;\
                                            overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">
                                            {goal}
                                        </div>
                                    </div>
                                    <div style="font-size: 9px; color: #556688; margin-top: 3px; text-transform: uppercase;\
                                        letter-spacing: 1px; font-family: 'Orbitron', monospace;">
                                        {status}
                                    </div>
                                </div>
                            }
                        }).collect::<Vec<_>>()}
                    </div>
                }.into_any()
            }}
        </div>
    }
}
