// ═══════════════════════════════════════════════════════════
// ZEUS — Network Page — Full inter-agent network coverage
// Agents · Discover (mDNS) · Messages inbox · Send · Broadcast
// ═══════════════════════════════════════════════════════════

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;
use crate::api;
use crate::components::design::*;

#[component]
pub fn NetworkPage() -> impl IntoView {
    let agents = RwSignal::new(Vec::<api::NetworkAgent>::new());
    let discover = RwSignal::new(Option::<serde_json::Value>::None);
    let messages = RwSignal::new(Vec::<api::NetworkMessage>::new());
    let search = RwSignal::new(String::new());
    let active_tab = RwSignal::new(0u8); // 0=agents 1=discover 2=messages 3=send 4=broadcast

    // Send form
    let send_host = RwSignal::new(String::new());
    let send_from = RwSignal::new(String::new());
    let send_to = RwSignal::new(String::new());
    let send_content = RwSignal::new(String::new());
    let send_result = RwSignal::new(String::new());
    let sending = RwSignal::new(false);

    // Broadcast form
    let bc_from = RwSignal::new(String::new());
    let bc_content = RwSignal::new(String::new());
    let bc_result = RwSignal::new(String::new());
    let broadcasting = RwSignal::new(false);

    // Load agents + messages on mount
    spawn_local(async move {
        if let Ok(resp) = api::fetch_network_agents().await { agents.set(resp.agents); }
        if let Ok(resp) = api::fetch_network_messages().await { messages.set(resp.messages); }
    });

    // Load discover when tab selected
    let load_discover = move || {
        spawn_local(async move {
            if let Ok(v) = api::fetch_network_discover().await {
                if let Ok(j) = serde_json::to_value(&v) {
                    discover.set(Some(j));
                }
            }
        });
    };

    let do_send = move |_: leptos::ev::MouseEvent| {
        let host = send_host.get_untracked();
        if host.trim().is_empty() { send_result.set("Host required".into()); return; }
        let content = send_content.get_untracked();
        if content.trim().is_empty() { send_result.set("Message required".into()); return; }
        let from = send_from.get_untracked();
        let to = send_to.get_untracked();
        sending.set(true);
        send_result.set(String::new());
        spawn_local(async move {
            let to_opt = if to.is_empty() { None } else { Some(to.as_str()) };
            match api::network_send(&host, None, &from, to_opt, &content).await {
                Ok(v) => send_result.set(format!("✓ {}", v.get("ok").and_then(|x| x.as_bool()).unwrap_or(false))),
                Err(e) => send_result.set(format!("Error: {}", e)),
            }
            sending.set(false);
        });
    };

    let do_broadcast = move |_: leptos::ev::MouseEvent| {
        let content = bc_content.get_untracked();
        if content.trim().is_empty() { bc_result.set("Message required".into()); return; }
        let from = bc_from.get_untracked();
        broadcasting.set(true);
        bc_result.set(String::new());
        spawn_local(async move {
            match api::network_broadcast(&from, &content).await {
                Ok(v) => {
                    let count = v.get("broadcast_count").and_then(|x| x.as_u64()).unwrap_or(0);
                    bc_result.set(format!("✓ Broadcast to {} peers", count));
                }
                Err(e) => bc_result.set(format!("Error: {}", e)),
            }
            broadcasting.set(false);
        });
    };

    let tab_style = move |idx: u8| move || format!(
        "padding: 7px 14px; border-radius: 8px; border: none; cursor: pointer; font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 2px; background: {}; color: {};",
        if active_tab.get() == idx { "rgba(255,60,20,0.2)" } else { "transparent" },
        if active_tab.get() == idx { "rgba(255,140,80,1)" } else { "rgba(255,245,240,0.5)" },
    );

    view! {
        <div style="padding: 32px; max-width: 1100px;">
            // Header
            <div style="margin-bottom: 20px;">
                <h1 style="font-family: 'Orbitron', monospace; font-size: 14px; letter-spacing: 6px; color: rgba(255,245,240,0.9); margin: 0 0 4px;">"NETWORK"</h1>
                <p style="font-size: 12px; color: rgba(255,245,240,0.5); margin: 0;">"Inter-agent mesh — discovery · messaging · broadcast"</p>
            </div>

            // Tabs
            <div style="display: flex; gap: 4px; background: rgba(255,255,255,0.02); border-radius: 10px; padding: 4px; width: fit-content; margin-bottom: 20px;">
                {[("AGENTS", 0u8), ("DISCOVER", 1u8), ("INBOX", 2u8), ("SEND", 3u8), ("BROADCAST", 4u8)].iter().map(|(label, idx)| {
                    let idx = *idx;
                    let label = *label;
                    let load = load_discover.clone();
                    view! {
                        <button
                            on:click=move |_| {
                                active_tab.set(idx);
                                if idx == 1 { load(); }
                            }
                            style=tab_style(idx)
                        >{label}</button>
                    }
                }).collect_view()}
            </div>

            // Tab: Agents
            {move || (active_tab.get() == 0).then(|| view! {
                <div>
                    <SearchBar placeholder="Search agents..." value=search />
                    <div style="display: grid; grid-template-columns: repeat(auto-fill, minmax(300px, 1fr)); gap: 12px; margin-top: 16px;">
                        {move || {
                            let q = search.get().to_lowercase();
                            agents.get().into_iter()
                                .filter(|a| q.is_empty() || a.name.to_lowercase().contains(&q) || a.role.to_lowercase().contains(&q))
                                .map(|a| {
                                    let status_color = match a.status.as_str() {
                                        "active" | "connected" => "rgba(34,197,94,1)",
                                        "idle" => "rgba(234,179,8,1)",
                                        _ => "rgba(239,68,68,1)",
                                    };
                                    view! {
                                        <Card>
                                            <div style="display: flex; align-items: flex-start; gap: 12px;">
                                                <div style="width: 44px; height: 44px; border-radius: 10px; background: rgba(255,60,20,0.1); display: flex; align-items: center; justify-content: center; flex-shrink: 0;">
                                                    <Icon name="agents" size=20 color="rgba(255,60,20,0.6)".to_string() />
                                                </div>
                                                <div style="flex: 1; min-width: 0;">
                                                    <div style="display: flex; align-items: center; gap: 8px; margin-bottom: 4px;">
                                                        <span style="font-size: 14px; color: rgba(255,245,240,0.9); font-weight: 600;">{a.name.clone()}</span>
                                                        <StatusDot status={a.status.clone()} />
                                                    </div>
                                                    <div style="font-size: 11px; color: rgba(255,245,240,0.7); margin-bottom: 8px;">{a.role.clone()}</div>
                                                    <div style="display: flex; gap: 8px; flex-wrap: wrap;">
                                                        {(!a.model.is_empty()).then(|| view! { <Badge text={a.model.clone()} /> })}
                                                        {(!a.agent_type.is_empty()).then(|| view! { <Badge text={a.agent_type.clone()} color={status_color.to_string()} /> })}
                                                        {(a.tasks > 0).then(|| view! { <Badge text={format!("{} tasks", a.tasks)} color="rgba(59,130,246,0.7)".to_string() /> })}
                                                    </div>
                                                    {(!a.address.is_empty()).then(|| view! {
                                                        <div style="font-size: 10px; color: rgba(255,245,240,0.5); margin-top: 6px; font-family: 'Orbitron', monospace;">{a.address.clone()}</div>
                                                    })}
                                                </div>
                                            </div>
                                        </Card>
                                    }
                                }).collect::<Vec<_>>()
                        }}
                    </div>
                </div>
            })}

            // Tab: Discover (mDNS)
            {move || (active_tab.get() == 1).then(|| view! {
                <div>
                    {move || match discover.get() {
                        None => view! { <div style="color: rgba(255,245,240,0.4); font-size: 13px;">"Loading mDNS peers..."</div> }.into_any(),
                        Some(v) => {
                            let broadcasting = v.get("broadcasting").and_then(|x| x.as_bool()).unwrap_or(false);
                            let count = v.get("peer_count").and_then(|x| x.as_u64()).unwrap_or(0);
                            let peers = v.get("mdns").and_then(|x| x.as_array()).cloned().unwrap_or_default();
                            view! {
                                <div>
                                    <div style="display: flex; gap: 16px; margin-bottom: 20px;">
                                        <div style="padding: 14px 20px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.12); border-radius: 10px; text-align: center;">
                                            <div style="font-family: 'Orbitron', monospace; font-size: 18px; color: rgba(255,245,240,0.9);">{count.to_string()}</div>
                                            <div style="font-size: 10px; color: rgba(255,245,240,0.4); margin-top: 4px;">"PEERS"</div>
                                        </div>
                                        <div style="padding: 14px 20px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.12); border-radius: 10px; text-align: center;">
                                            <div style=format!("font-family: 'Orbitron', monospace; font-size: 14px; color: {};", if broadcasting { "rgba(34,197,94,0.9)" } else { "rgba(239,68,68,0.7)" })>
                                                {if broadcasting { "ACTIVE" } else { "INACTIVE" }}
                                            </div>
                                            <div style="font-size: 10px; color: rgba(255,245,240,0.4); margin-top: 4px;">"mDNS BROADCAST"</div>
                                        </div>
                                    </div>
                                    {if peers.is_empty() {
                                        view! { <div style="color: rgba(255,245,240,0.3); font-size: 13px;">"No mDNS peers discovered on LAN"</div> }.into_any()
                                    } else {
                                        view! {
                                            <div style="display: flex; flex-direction: column; gap: 8px;">
                                                {peers.into_iter().map(|p| {
                                                    let name = p.get("name").and_then(|x| x.as_str()).unwrap_or("—").to_string();
                                                    let addr = p.get("address").and_then(|x| x.as_str()).unwrap_or("").to_string();
                                                    let port = p.get("port").and_then(|x| x.as_u64()).unwrap_or(8080);
                                                    view! {
                                                        <div style="padding: 12px 16px; background: rgba(255,255,255,0.02); border: 1px solid rgba(255,60,20,0.1); border-radius: 8px; display: flex; align-items: center; gap: 12px;">
                                                            <div style="width: 8px; height: 8px; border-radius: 50%; background: rgba(34,197,94,0.8); flex-shrink: 0;"></div>
                                                            <span style="font-size: 13px; color: rgba(255,245,240,0.85); font-weight: 600;">{name}</span>
                                                            <span style="font-family: monospace; font-size: 11px; color: rgba(255,245,240,0.4);">{format!("{}:{}", addr, port)}</span>
                                                        </div>
                                                    }
                                                }).collect_view()}
                                            </div>
                                        }.into_any()
                                    }}
                                </div>
                            }.into_any()
                        }
                    }}
                </div>
            })}

            // Tab: Inbox
            {move || (active_tab.get() == 2).then(|| view! {
                <div>
                    <div style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,60,20,0.6); margin-bottom: 12px;">
                        {move || format!("{} MESSAGES", messages.get().len())}
                    </div>
                    {move || {
                        let msgs = messages.get();
                        if msgs.is_empty() {
                            view! { <div style="color: rgba(255,245,240,0.3); font-size: 13px;">"No messages in inbox"</div> }.into_any()
                        } else {
                            view! {
                                <div style="display: flex; flex-direction: column; gap: 8px;">
                                    {msgs.into_iter().map(|m| view! {
                                        <div style="padding: 12px 16px; background: rgba(255,255,255,0.02); border: 1px solid rgba(255,60,20,0.1); border-radius: 8px;">
                                            <div style="display: flex; gap: 8px; margin-bottom: 6px; font-size: 10px; color: rgba(255,245,240,0.4);">
                                                <span style="color: rgba(255,140,80,0.7);">{m.from.clone()}</span>
                                                {(!m.to.is_empty()).then(|| view! { <span>"→ "{m.to.clone()}</span> })}
                                                <span style="margin-left: auto;">{m.timestamp.clone()}</span>
                                            </div>
                                            <div style="font-size: 13px; color: rgba(255,245,240,0.85); line-height: 1.5;">{m.content.clone()}</div>
                                        </div>
                                    }).collect_view()}
                                </div>
                            }.into_any()
                        }
                    }}
                </div>
            })}

            // Tab: Send
            {move || (active_tab.get() == 3).then(|| view! {
                <div style="max-width: 480px;">
                    <div style="display: flex; flex-direction: column; gap: 14px;">
                        <div>
                            <div style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,245,240,0.6); margin-bottom: 6px;">"TARGET HOST *"</div>
                            <input type="text" placeholder="192.168.1.112"
                                style="width: 100%; padding: 10px 14px; background: rgba(0,0,0,0.3); border: 1px solid rgba(255,60,20,0.15); border-radius: 8px; color: rgba(255,245,240,0.85); font-family: monospace; font-size: 13px; box-sizing: border-box; outline: none;"
                                prop:value=move || send_host.get()
                                on:input=move |e| send_host.set(event_target_value(&e))
                            />
                        </div>
                        <div>
                            <div style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,245,240,0.6); margin-bottom: 6px;">"FROM AGENT"</div>
                            <input type="text" placeholder="zeus_106"
                                style="width: 100%; padding: 10px 14px; background: rgba(0,0,0,0.3); border: 1px solid rgba(255,60,20,0.15); border-radius: 8px; color: rgba(255,245,240,0.85); font-family: 'Rajdhani', sans-serif; font-size: 13px; box-sizing: border-box; outline: none;"
                                prop:value=move || send_from.get()
                                on:input=move |e| send_from.set(event_target_value(&e))
                            />
                        </div>
                        <div>
                            <div style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,245,240,0.6); margin-bottom: 6px;">"TO AGENT (optional)"</div>
                            <input type="text" placeholder="zeus_112"
                                style="width: 100%; padding: 10px 14px; background: rgba(0,0,0,0.3); border: 1px solid rgba(255,60,20,0.15); border-radius: 8px; color: rgba(255,245,240,0.85); font-family: 'Rajdhani', sans-serif; font-size: 13px; box-sizing: border-box; outline: none;"
                                prop:value=move || send_to.get()
                                on:input=move |e| send_to.set(event_target_value(&e))
                            />
                        </div>
                        <div>
                            <div style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,245,240,0.6); margin-bottom: 6px;">"MESSAGE *"</div>
                            <textarea rows="4"
                                style="width: 100%; padding: 10px 14px; background: rgba(0,0,0,0.3); border: 1px solid rgba(255,60,20,0.15); border-radius: 8px; color: rgba(255,245,240,0.85); font-family: 'Rajdhani', sans-serif; font-size: 13px; box-sizing: border-box; outline: none; resize: vertical;"
                                prop:value=move || send_content.get()
                                on:input=move |e| send_content.set(event_target_value(&e))
                            />
                        </div>
                        <button
                            disabled=move || sending.get()
                            on:click=do_send
                            style="padding: 10px 24px; background: rgba(255,60,20,0.15); border: 1px solid rgba(255,60,20,0.4); border-radius: 8px; color: rgba(255,140,80,1); font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; cursor: pointer; width: fit-content;"
                        >{move || if sending.get() { "SENDING..." } else { "▶ SEND" }}</button>
                        {move || (!send_result.get().is_empty()).then(|| view! {
                            <div style="font-size: 12px; color: rgba(255,245,240,0.7); font-family: monospace;">{send_result.get()}</div>
                        })}
                    </div>
                </div>
            })}

            // Tab: Broadcast
            {move || (active_tab.get() == 4).then(|| view! {
                <div style="max-width: 480px;">
                    <div style="font-size: 12px; color: rgba(255,245,240,0.4); margin-bottom: 16px;">"Fan-out message to all mDNS-discovered peers on the LAN."</div>
                    <div style="display: flex; flex-direction: column; gap: 14px;">
                        <div>
                            <div style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,245,240,0.6); margin-bottom: 6px;">"FROM AGENT"</div>
                            <input type="text" placeholder="zeus_106"
                                style="width: 100%; padding: 10px 14px; background: rgba(0,0,0,0.3); border: 1px solid rgba(255,60,20,0.15); border-radius: 8px; color: rgba(255,245,240,0.85); font-family: 'Rajdhani', sans-serif; font-size: 13px; box-sizing: border-box; outline: none;"
                                prop:value=move || bc_from.get()
                                on:input=move |e| bc_from.set(event_target_value(&e))
                            />
                        </div>
                        <div>
                            <div style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,245,240,0.6); margin-bottom: 6px;">"MESSAGE *"</div>
                            <textarea rows="4"
                                style="width: 100%; padding: 10px 14px; background: rgba(0,0,0,0.3); border: 1px solid rgba(255,60,20,0.15); border-radius: 8px; color: rgba(255,245,240,0.85); font-family: 'Rajdhani', sans-serif; font-size: 13px; box-sizing: border-box; outline: none; resize: vertical;"
                                prop:value=move || bc_content.get()
                                on:input=move |e| bc_content.set(event_target_value(&e))
                            />
                        </div>
                        <button
                            disabled=move || broadcasting.get()
                            on:click=do_broadcast
                            style="padding: 10px 24px; background: rgba(255,60,20,0.15); border: 1px solid rgba(255,60,20,0.4); border-radius: 8px; color: rgba(255,140,80,1); font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; cursor: pointer; width: fit-content;"
                        >{move || if broadcasting.get() { "BROADCASTING..." } else { "◈ BROADCAST ALL" }}</button>
                        {move || (!bc_result.get().is_empty()).then(|| view! {
                            <div style="font-size: 12px; color: rgba(255,245,240,0.7); font-family: monospace;">{bc_result.get()}</div>
                        })}
                    </div>
                </div>
            })}
        </div>
    }
}
