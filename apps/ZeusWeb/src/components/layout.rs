// ═══════════════════════════════════════════════════════════
// ZEUS — Layout Shell (Sidebar + Main Content)
// v3 — Full inline styles matching JSX Sidebar (lines 274-340)
// Zero CSS class dependencies
// ═══════════════════════════════════════════════════════════

use leptos::prelude::*;
use leptos_router::components::Outlet;
use leptos_router::hooks::use_location;
use wasm_bindgen_futures::spawn_local;
use wasm_bindgen::prelude::*;
use std::rc::Rc;
use std::cell::RefCell;

use crate::api;
use super::design::Icon;
use super::sentient_orb::SentientOrb;

// ─── SIDEBAR ITEM ─────────────────────────────────────────
// JSX: flex, gap:10, padding:9px 14px, radius:8, active=ember left border + bg

#[component]
fn SidebarItem(
    #[prop(into)] icon: String,
    #[prop(into)] label: String,
    #[prop(into)] href: String,
    #[prop(default = None)] badge: Option<String>,
) -> impl IntoView {
    let location = use_location();
    let href_c = href.clone();
    let is_active = Memo::new(move |_| {
        let path = location.pathname.get();
        if href_c == "/" {
            path == "/" || path.is_empty()
        } else {
            path == href_c || path.starts_with(&format!("{}/", href_c))
        }
    });

    view! {
        <a
            href={href}
            style=move || {
                let active = is_active.get();
                format!(
                    "display: flex; align-items: center; gap: 10px; padding: 9px 14px; border-radius: 8px; cursor: pointer; text-decoration: none; background: {}; border-left: 2px solid {}; transition: all 0.2s; margin-bottom: 1px;",
                    if active { "rgba(255,60,20,0.08)" } else { "transparent" },
                    if active { "rgba(255,60,20,0.6)" } else { "transparent" },
                )
            }
        >
            <div style=move || format!(
                "color: {}; flex-shrink: 0;",
                if is_active.get() { "rgba(255,60,20,0.6)" } else { "rgba(255,245,240,0.5)" }
            )>
                <Icon name={icon} size=16 />
            </div>
            <span style=move || format!(
                "font-family: 'Rajdhani', sans-serif; font-size: 13px; font-weight: {}; color: {}; flex: 1;",
                if is_active.get() { "600" } else { "400" },
                if is_active.get() { "rgba(255,245,240,0.9)" } else { "rgba(255,245,240,0.7)" },
            )>{label}</span>
            {badge.map(|b| view! {
                <span style="font-family: 'Orbitron', monospace; font-size: 8px; color: rgba(255,60,20,0.6); background: rgba(255,60,20,0.1); padding: 1px 6px; border-radius: 4px;">{b}</span>
            })}
        </a>
    }
}

// ─── SIDEBAR SECTION ──────────────────────────────────────
// JSX: marginBottom:16, title Orbitron 8px, ls:3, muted, uppercase

#[component]
fn SidebarSection(
    #[prop(into)] title: String,
    children: Children,
) -> impl IntoView {
    view! {
        <div style="margin-bottom: 16px;">
            <div style="font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 3px; color: rgba(255,245,240,0.5); text-transform: uppercase; padding: 4px 14px; margin-bottom: 4px;">{title}</div>
            {children()}
        </div>
    }
}

// ─── SIDEBAR (with live counts from API) ──────────────────
// JSX: width:220, height:100vh, bg:rgba(8,8,12,0.95), borderRight, flex column

#[component]
fn Sidebar() -> impl IntoView {
    let session_count = RwSignal::new("—".to_string());
    let agent_count = RwSignal::new("—".to_string());
    let tool_count = RwSignal::new("—".to_string());
    let channel_count = RwSignal::new("—".to_string());

    // Fetch counts for sidebar badges
    {
        let session_count = session_count;
        spawn_local(async move { if let Ok(s) = api::fetch_sessions().await { session_count.set(s.total.to_string()); } });
    }
    {
        let agent_count = agent_count;
        spawn_local(async move { if let Ok(a) = api::fetch_agents().await { agent_count.set(a.agents.len().to_string()); } });
    }
    {
        let tool_count = tool_count;
        spawn_local(async move { if let Ok(t) = api::get_tools().await { tool_count.set(t.tools.len().to_string()); } });
    }
    {
        let channel_count = channel_count;
        spawn_local(async move { if let Ok(c) = api::fetch_channels().await { channel_count.set(c.channels.len().to_string()); } });
    }

    view! {
        <div style="width: 220px; height: 100vh; background: rgba(8,8,12,0.95); border-right: 1px solid rgba(255,60,20,0.1); display: flex; flex-direction: column; overflow: hidden; flex-shrink: 0; position: fixed; top: 0; left: 0; z-index: 100;">
            // Header: Orb + ZEUS title
            <div style="padding: 20px 14px 16px; display: flex; align-items: center; gap: 10px; border-bottom: 1px solid rgba(255,60,20,0.1);">
                <SentientOrb size=36 mode="active" />
                <div>
                    <div style="font-family: 'Orbitron', monospace; font-size: 13px; font-weight: 900; letter-spacing: 4px; color: rgba(255,245,240,0.9);">
                        "ZEUS"
                    </div>
                    <div style="font-family: 'Rajdhani', sans-serif; font-size: 9px; color: rgba(255,245,240,0.5); letter-spacing: 2px;">
                        "COGNITIVE PLATFORM"
                    </div>
                </div>
            </div>

            // Scrollable nav sections
            <div style="flex: 1; overflow-y: auto; padding: 12px 6px;">
                <SidebarSection title="Core">
                    <SidebarItem icon="dashboard" label="Dashboard" href="/" />
                    <SidebarItem icon="chat" label="Chat" href="/chat" />
                    {move || view! { <SidebarItem icon="sessions" label="Sessions" href="/sessions" badge=Some(session_count.get()) /> }}
                </SidebarSection>

                <SidebarSection title="Intelligence">
                    {move || view! { <SidebarItem icon="agents" label="Agents" href="/agents" badge=Some(agent_count.get()) /> }}
                    <SidebarItem icon="teams" label="Teams" href="/teams" />
                    <SidebarItem icon="pantheon" label="Pantheon" href="/pantheon" />
                    <SidebarItem icon="agents" label="Discover" href="/discover" />
                    <SidebarItem icon="memory" label="Memory" href="/memory" />
                    <SidebarItem icon="analytics" label="Nous" href="/nous" />
                    <SidebarItem icon="search" label="Vector Stores" href="/vector-stores" />
                    {move || view! { <SidebarItem icon="tools" label="Tools" href="/tools" badge=Some(tool_count.get()) /> }}
                </SidebarSection>

                <SidebarSection title="Connectivity">
                    {move || view! { <SidebarItem icon="channels" label="Channels" href="/channels" badge=Some(channel_count.get()) /> }}
                    <SidebarItem icon="mcp" label="MCP Servers" href="/mcp" />
                    <SidebarItem icon="voice" label="Voice" href="/voice" />
                </SidebarSection>

                <SidebarSection title="Operations">
                    <SidebarItem icon="analytics" label="Analytics" href="/analytics" />
                    <SidebarItem icon="security" label="Security" href="/security" />
                    <SidebarItem icon="approvals" label="Approvals" href="/approvals" />
                    <SidebarItem icon="sandbox" label="Sandbox" href="/sandbox" />
                </SidebarSection>

                <SidebarSection title="Ecosystem">
                    <SidebarItem icon="skills" label="Skills" href="/skills" />
                    <SidebarItem icon="projects" label="Projects" href="/projects" />
                    <SidebarItem icon="globe" label="Agora" href="/agora" />
                    <SidebarItem icon="tools" label="Templates" href="/templates" />
                    <SidebarItem icon="deploy" label="Deploy" href="/deploy" />
                    <SidebarItem icon="sessions" label="Batch Jobs" href="/batch" />
                    <SidebarItem icon="approvals" label="Reviews / QA" href="/reviews" />
                    <SidebarItem icon="channels" label="Webhooks" href="/webhooks" />
                    <SidebarItem icon="zap" label="AI Tools" href="/ai-tools" />
                    <SidebarItem icon="upload" label="Uploads" href="/uploads" />
                    <SidebarItem icon="cpu" label="Canvas" href="/canvas" />
                    <SidebarItem icon="agents" label="Office" href="/office" />
                    // Blog Admin removed — part of zeuslab.ai marketing site
                </SidebarSection>
            </div>

            // Footer: Settings + Logout
            <div style="padding: 12px 14px; border-top: 1px solid rgba(255,60,20,0.1);">
                <SidebarItem icon="settings" label="Settings" href="/settings" />
                <button
                    on:click=move |_| {
                        spawn_local(async move {
                            let _ = api::auth_logout().await;
                            api::clear_auth_token();
                            if let Some(win) = web_sys::window() {
                                let _ = win.location().set_href("/login");
                            }
                        });
                    }
                    style="display: flex; align-items: center; gap: 10px; padding: 9px 14px; border-radius: 8px; cursor: pointer; text-decoration: none; background: transparent; border: none; border-left: 2px solid transparent; width: 100%; margin-top: 1px;"
                >
                    <div style="color: rgba(255,245,240,0.4); flex-shrink: 0; display: flex; align-items: center;">
                        <Icon name="logout" size=16 />
                    </div>
                    <span style="font-family: 'Rajdhani', sans-serif; font-size: 13px; font-weight: 400; color: rgba(255,245,240,0.5);">"Logout"</span>
                </button>
            </div>
        </div>
    }
}

// ─── WS NOTIFICATIONS ─────────────────────────────────────
// Global WebSocket listener attached to the Shell.
// Handles: approval_pending toasts, channel_message toasts,
// agent heartbeat badge, activity ring buffer (last 5 events).

#[derive(Clone, Debug)]
struct ApprovalToast {
    id: String,
    tool_name: String,
    agent_id: Option<String>,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
struct ActivityItem {
    kind: String,  // icon hint: "approval" | "channel" | "agent" | "tool" | "info"
    text: String,
}

#[component]
fn WsNotifications() -> impl IntoView {
    let approvals: RwSignal<Vec<ApprovalToast>> = RwSignal::new(vec![]);
    let activity: RwSignal<Vec<ActivityItem>> = RwSignal::new(vec![]);
    let channel_toast: RwSignal<Option<String>> = RwSignal::new(None);
    let show_activity = RwSignal::new(false);

    // push to activity ring (max 5)
    let push_activity = {
        let activity = activity;
        move |kind: &str, text: &str| {
            activity.update(|v| {
                v.insert(0, ActivityItem { kind: kind.to_string(), text: text.to_string() });
                v.truncate(5);
            });
        }
    };

    // WebSocket connection
    let ws_ref: Rc<RefCell<Option<web_sys::WebSocket>>> = Rc::new(RefCell::new(None));
    let ws_ref_c = ws_ref.clone();

    spawn_local(async move {
        gloo_timers::future::TimeoutFuture::new(800).await;

        let window = match web_sys::window() { Some(w) => w, None => return };
        let location = window.location();
        let protocol = if location.protocol().unwrap_or_default() == "https:" { "wss:" } else { "ws:" };
        let host = location.host().unwrap_or_default();
        // Browser WebSocket can't set Authorization headers;
        // server accepts ?token= query param for auth.
        let url = match crate::api::get_auth_token() {
            Some(tok) => format!("{}//{}/v1/ws?token={}", protocol, host, js_sys::encode_uri_component(&tok)),
            None => format!("{}//{}/v1/ws", protocol, host),
        };

        let ws = match web_sys::WebSocket::new(&url) { Ok(w) => w, Err(_) => return };

        // onmessage — parse and dispatch
        let approvals_c = approvals;
        let channel_toast_c = channel_toast;
        let push_c = push_activity.clone();
        let on_message = Closure::wrap(Box::new(move |e: web_sys::MessageEvent| {
            let text = match e.data().as_string() { Some(t) => t, None => return };
            let msg: serde_json::Value = match serde_json::from_str(&text) { Ok(v) => v, Err(_) => return };
            let msg_type = msg.get("type").and_then(|t| t.as_str()).unwrap_or("");

            match msg_type {
                "approval_pending" => {
                    let id = msg.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let tool_name = msg.get("tool_name").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
                    let agent_id = msg.get("agent_id").and_then(|v| v.as_str()).map(|s| s.to_string());
                    let text = format!("⚠ Approval: {} requested", tool_name);
                    push_c("approval", &text);
                    approvals_c.update(|v| {
                        if !v.iter().any(|a| a.id == id) {
                            v.push(ApprovalToast { id, tool_name, agent_id });
                        }
                    });
                }
                "approval_resolved" => {
                    let id = msg.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    approvals_c.update(|v| v.retain(|a| a.id != id));
                    push_c("approval", "✓ Approval resolved");
                }
                "studio_event" => {
                    let event_type = msg.get("event_type").and_then(|v| v.as_str()).unwrap_or("");
                    match event_type {
                        "channel_message" => {
                            let channel = msg.get("data")
                                .and_then(|d| d.get("channel")).and_then(|c| c.as_str())
                                .unwrap_or("channel");
                            let snippet = msg.get("data")
                                .and_then(|d| d.get("text")).and_then(|t| t.as_str())
                                .unwrap_or("New message");
                            let summary = format!("[{}] {}", channel, crate::api::truncate_str(snippet, 60));
                            push_c("channel", &summary);
                            channel_toast_c.set(Some(summary.clone()));
                            // Auto-clear toast after 4s
                            let ct = channel_toast_c;
                            spawn_local(async move {
                                gloo_timers::future::TimeoutFuture::new(4_000).await;
                                ct.set(None);
                            });
                        }
                        "heartbeat" => {
                            let agent = msg.get("data")
                                .and_then(|d| d.get("agent_name")).and_then(|a| a.as_str())
                                .unwrap_or("agent");
                            push_c("agent", &format!("♥ {} heartbeat", agent));
                        }
                        "team_created" | "studio_started" | "studio_complete" => {
                            push_c("info", &format!("◆ {}", event_type.replace('_', " ")));
                        }
                        _ => {}
                    }
                }
                "tool_call" => {
                    let name = msg.get("name").and_then(|v| v.as_str()).unwrap_or("tool");
                    push_c("tool", &format!("⚙ Tool: {}", name));
                }
                _ => {}
            }
        }) as Box<dyn FnMut(_)>);
        ws.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        on_message.forget();

        *ws_ref_c.borrow_mut() = Some(ws);
    });

    // Dismiss an approval toast (without API call — just hide from UI; approvals page handles the action)
    let dismiss_approval = move |id: String| {
        approvals.update(|v| v.retain(|a| a.id != id));
    };

    view! {
        // Floating notifications — fixed bottom-right
        <div style="position: fixed; bottom: 20px; right: 20px; z-index: 9000; display: flex; flex-direction: column; gap: 8px; align-items: flex-end; pointer-events: none;">

            // Channel message toast
            {move || channel_toast.get().map(|msg| view! {
                <div style="background: rgba(8,8,12,0.95); border: 1px solid rgba(255,60,20,0.3); border-radius: 10px; padding: 10px 14px; max-width: 300px; pointer-events: auto; box-shadow: 0 4px 20px rgba(0,0,0,0.5);">
                    <div style="font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 2px; color: rgba(255,60,20,0.6); margin-bottom: 4px;">"CHANNEL MSG"</div>
                    <div style="font-size: 12px; color: rgba(255,245,240,0.8); font-family: 'Rajdhani', sans-serif;">{msg}</div>
                </div>
            })}

            // Approval toasts
            {move || {
                let items = approvals.get();
                let dismiss = dismiss_approval.clone();
                items.into_iter().map(|a| {
                    let id = a.id.clone();
                    let id_dismiss = id.clone();
                    let dismiss2 = dismiss.clone();
                    let label = a.tool_name.clone();
                    let agent = a.agent_id.clone().unwrap_or_default();
                    view! {
                        <div style="background: rgba(8,8,12,0.97); border: 1px solid rgba(255,60,20,0.5); border-radius: 10px; padding: 12px 14px; max-width: 280px; pointer-events: auto; box-shadow: 0 4px 24px rgba(255,60,20,0.15);">
                            <div style="font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 2px; color: rgba(255,140,80,0.9); margin-bottom: 6px;">"⚠ APPROVAL NEEDED"</div>
                            <div style="font-size: 13px; color: rgba(255,245,240,0.9); font-weight: 600; margin-bottom: 2px;">{label}</div>
                            {(!agent.is_empty()).then(|| view! {
                                <div style="font-size: 11px; color: rgba(255,245,240,0.4); margin-bottom: 8px;">"Agent: "{agent}</div>
                            })}
                            <div style="display: flex; gap: 6px; margin-top: 8px;">
                                <a href="/approvals" style="flex: 1; padding: 5px 0; background: rgba(255,60,20,0.15); border: 1px solid rgba(255,60,20,0.4); border-radius: 6px; color: rgba(255,140,80,1); font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 1px; text-align: center; text-decoration: none; cursor: pointer;">"REVIEW"</a>
                                <button
                                    on:click=move |_| dismiss2(id_dismiss.clone())
                                    style="padding: 5px 10px; background: transparent; border: 1px solid rgba(255,245,240,0.1); border-radius: 6px; color: rgba(255,245,240,0.4); font-size: 11px; cursor: pointer;"
                                >"✕"</button>
                            </div>
                        </div>
                    }
                }).collect_view()
            }}

            // Activity feed ticker toggle
            <div style="pointer-events: auto;">
                <button
                    on:click=move |_| show_activity.update(|v| *v = !*v)
                    style="padding: 6px 12px; background: rgba(8,8,12,0.9); border: 1px solid rgba(255,60,20,0.2); border-radius: 20px; color: rgba(255,245,240,0.5); font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 1px; cursor: pointer;"
                >"ACTIVITY"</button>

                {move || show_activity.get().then(|| {
                    let items = activity.get();
                    if items.is_empty() {
                        view! {
                            <div style="margin-top: 4px; background: rgba(8,8,12,0.95); border: 1px solid rgba(255,60,20,0.15); border-radius: 10px; padding: 12px 14px; width: 260px;">
                                <div style="font-size: 12px; color: rgba(255,245,240,0.3); font-family: 'Rajdhani', sans-serif;">"No activity yet"</div>
                            </div>
                        }.into_any()
                    } else {
                        view! {
                            <div style="margin-top: 4px; background: rgba(8,8,12,0.95); border: 1px solid rgba(255,60,20,0.15); border-radius: 10px; padding: 12px 14px; width: 260px;">
                                <div style="font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 2px; color: rgba(255,60,20,0.5); margin-bottom: 8px;">"LIVE ACTIVITY"</div>
                                {items.into_iter().map(|item| view! {
                                    <div style="font-size: 11px; color: rgba(255,245,240,0.65); font-family: 'Rajdhani', sans-serif; padding: 3px 0; border-bottom: 1px solid rgba(255,255,255,0.03);">{item.text}</div>
                                }).collect_view()}
                            </div>
                        }.into_any()
                    }
                })}
            </div>
        </div>
    }
}

// ─── SHELL (Main Layout) ──────────────────────────────────
// Wraps all routes under ParentRoute in main.rs.
// Sidebar (fixed 220px) + main content area with Outlet.

#[component]
pub fn Shell() -> impl IntoView {
    let mobile_nav = RwSignal::new(false);

    // Onboarding gate: check localStorage first (instant, survives gateway restarts),
    // then API. Prevents redirect loop when gateway is restarting after onboarding.
    Effect::new(move |_| {
        // Skip redirect if localStorage says onboarding is done
        if let Some(win) = web_sys::window()
            && let Ok(Some(storage)) = win.local_storage()
            && storage.get_item("zeus_onboarding_complete").ok().flatten() == Some("true".into())
        {
            return; // Already onboarded — don't redirect
        }

        spawn_local(async move {
            match api::fetch_onboarding_status().await {
                Ok(status) => {
                    if !status.completed {
                        if let Some(win) = web_sys::window() {
                            let _ = win.location().set_href("/onboarding");
                        }
                    }
                },
                Err(_) => {
                    // Gateway unreachable + no localStorage flag → first-time user
                    if let Some(win) = web_sys::window() {
                        let _ = win.location().set_href("/onboarding");
                    }
                }
            }
        });
    });

    view! {
        <div style="display: flex; height: 100vh; overflow: hidden; background: #050508;">
            // Mobile hamburger button (shown on small screens via media query)
            <button
                class="zeus-mobile-hamburger"
                style="display: none; position: fixed; top: 12px; left: 12px; z-index: 200; width: 36px; height: 36px; border-radius: 8px; background: rgba(255,60,20,0.15); border: 1px solid rgba(255,60,20,0.3); color: rgba(255,245,240,0.9); font-size: 18px; cursor: pointer; align-items: center; justify-content: center;"
                on:click=move |_| mobile_nav.update(|v| *v = !*v)
            >"\u{2630}"</button>

            // Mobile overlay (only visible when nav open on mobile)
            <Show when=move || mobile_nav.get()>
                <div
                    style="display: none; position: fixed; inset: 0; background: rgba(0,0,0,0.6); z-index: 99;"
                    class="zeus-mobile-overlay"
                    on:click=move |_| mobile_nav.set(false)
                />
            </Show>

            <div
                class="zeus-sidebar-wrapper"
                style=move || (if mobile_nav.get() { "transform: translateX(0);" } else { "" }).to_string()
            >
                <Sidebar />
            </div>
            <main class="zeus-main-content" style="flex: 1; overflow-y: auto; overflow-x: hidden; margin-left: 220px;">
                <Outlet />
            </main>
            <WsNotifications />
        </div>
    }
}
