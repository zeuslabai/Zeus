// ═══════════════════════════════════════════════════════════
// ZEUS — Agent Editor — Full agent management with soul + tools + status + channel binding
// ═══════════════════════════════════════════════════════════

use leptos::prelude::*;
use leptos_router::hooks::use_params_map;
use wasm_bindgen_futures::spawn_local;
use crate::api;
use crate::components::design::*;

// ── T8: Channel Binding Section ──────────────────────────────────────────────

#[allow(unused_variables)]
#[component]
fn ChannelBindingSection(agent_id: RwSignal<String>) -> impl IntoView {
    let channels = RwSignal::new(Vec::<api::Channel>::new());
    let bound_ids = RwSignal::new(std::collections::HashSet::<String>::new());
    let action_msg = RwSignal::new(String::new());
    let expanded = RwSignal::new(false);

    // Fetch channels on mount
    spawn_local(async move {
        if let Ok(resp) = api::fetch_channels().await {
            channels.set(resp.channels);
        }
    });

    let label_style = "font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,245,240,0.7); margin-bottom: 6px;";

    view! {
        <Card style="margin-top: 16px;">
            <div
                style="display: flex; justify-content: space-between; align-items: center; cursor: pointer;"
                on:click=move |_| expanded.set(!expanded.get())
            >
                <div style=label_style>"CHANNEL BINDINGS"</div>
                <span style="font-size: 12px; color: rgba(255,245,240,0.5);">
                    {move || if expanded.get() { "\u{25B2}" } else { "\u{25BC}" }}
                </span>
            </div>

            <Show when=move || expanded.get()>
                <div style="margin-top: 12px;">
                    {move || {
                        let ch = channels.get();
                        if ch.is_empty() {
                            return view! {
                                <div style="font-size: 12px; color: rgba(255,245,240,0.5); font-family: 'Rajdhani', sans-serif;">
                                    "No channels configured. Add channels in Settings first."
                                </div>
                            }.into_any();
                        }
                        let bound = bound_ids.get();
                        view! {
                            <div style="display: flex; flex-direction: column; gap: 8px;">
                                {ch.into_iter().map(|c| {
                                    let cid = c.id.clone();
                                    let cname = c.name.clone();
                                    let ctype = c.channel_type.clone();
                                    let is_bound = bound.contains(&c.id);

                                    view! {
                                        <div style="display: flex; align-items: center; justify-content: space-between; padding: 8px 12px; background: rgba(255,255,255,0.02); border-radius: 6px; border: 1px solid rgba(255,60,20,0.08);">
                                            <div style="display: flex; align-items: center; gap: 8px;">
                                                <Badge text=ctype />
                                                <span style="font-family: 'Rajdhani', sans-serif; font-size: 13px; color: rgba(255,245,240,0.9);">
                                                    {cname}
                                                </span>
                                            </div>
                                            <button
                                                style={format!("font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 1px; padding: 4px 10px; border-radius: 6px; cursor: pointer; border: 1px solid {}; background: {}; color: {};",
                                                    if is_bound { "rgba(255,60,20,0.3)" } else { "rgba(80,200,120,0.3)" },
                                                    if is_bound { "rgba(255,60,20,0.08)" } else { "rgba(80,200,120,0.08)" },
                                                    if is_bound { "rgba(255,60,20,0.8)" } else { "rgba(80,200,120,0.8)" }
                                                )}
                                                on:click=move |_| {
                                                    let cid = cid.clone();
                                                    let action_msg = action_msg;
                                                    let bound_ids = bound_ids;
                                                    spawn_local(async move {
                                                        let mut current = bound_ids.get_untracked();
                                                        if current.contains(&cid) {
                                                            match api::disconnect_channel(&cid).await {
                                                                Ok(_) => {
                                                                    current.remove(&cid);
                                                                    bound_ids.set(current);
                                                                    action_msg.set("\u{2713} Unbound".into());
                                                                }
                                                                Err(e) => action_msg.set(format!("Error: {}", e)),
                                                            }
                                                        } else {
                                                            match api::connect_channel(&cid).await {
                                                                Ok(_) => {
                                                                    current.insert(cid);
                                                                    bound_ids.set(current);
                                                                    action_msg.set("\u{2713} Bound".into());
                                                                }
                                                                Err(e) => action_msg.set(format!("Error: {}", e)),
                                                            }
                                                        }
                                                    });
                                                }
                                            >
                                                {if is_bound { "Unbind" } else { "Bind" }}
                                            </button>
                                        </div>
                                    }
                                }).collect::<Vec<_>>()}
                            </div>
                        }.into_any()
                    }}

                    {move || {
                        let m = action_msg.get();
                        (!m.is_empty()).then(|| view! {
                            <div style="margin-top: 8px; font-size: 11px; color: rgba(80,200,120,0.8); font-family: 'JetBrains Mono', monospace;">
                                {m}
                            </div>
                        })
                    }}
                </div>
            </Show>
        </Card>
    }
}

// ── T7: Live Status Polling ─────────────────────────────────────────────────

#[component]
fn LiveStatusSection(agent_id: RwSignal<String>) -> impl IntoView {
    let status_text = RwSignal::new(String::new());
    let last_active = RwSignal::new(String::new());
    let msg_count = RwSignal::new(0u32);
    let polling = RwSignal::new(false);
    let expanded = RwSignal::new(false);

    let label_style = "font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,245,240,0.7); margin-bottom: 6px;";

    let do_poll = move || {
        let aid = agent_id.get_untracked();
        if aid.is_empty() { return; }
        polling.set(true);
        spawn_local(async move {
            match api::fetch_agent_status(&aid).await {
                Ok(s) => {
                    status_text.set(if s.status.is_empty() { "unknown".into() } else { s.status });
                    last_active.set(if s.last_active.is_empty() { "n/a".into() } else { s.last_active });
                    msg_count.set(s.message_count);
                }
                Err(e) => {
                    status_text.set(format!("Error: {}", e));
                }
            }
            polling.set(false);
        });
    };

    // Initial poll
    do_poll();

    view! {
        <Card style="margin-top: 16px;">
            <div
                style="display: flex; justify-content: space-between; align-items: center; cursor: pointer;"
                on:click=move |_| expanded.set(!expanded.get())
            >
                <div style=label_style>"LIVE STATUS"</div>
                <div style="display: flex; align-items: center; gap: 8px;">
                    {move || {
                        let s = status_text.get();
                        let color = match s.as_str() {
                            "active" | "running" => "rgba(80,200,120,0.9)",
                            "idle" => "rgba(255,200,0,0.9)",
                            _ => "rgba(255,245,240,0.5)",
                        };
                        view! {
                            <span style={format!("font-size: 11px; font-family: 'JetBrains Mono', monospace; color: {};", color)}>
                                {s}
                            </span>
                        }
                    }}
                    <span style="font-size: 12px; color: rgba(255,245,240,0.5);">
                        {move || if expanded.get() { "\u{25B2}" } else { "\u{25BC}" }}
                    </span>
                </div>
            </div>

            <Show when=move || expanded.get()>
                <div style="margin-top: 12px; display: grid; grid-template-columns: 1fr 1fr 1fr; gap: 12px;">
                    <div>
                        <div style="font-size: 8px; color: rgba(255,245,240,0.5); font-family: 'Orbitron', monospace; letter-spacing: 1px;">"LAST ACTIVE"</div>
                        <div style="font-size: 13px; color: rgba(255,245,240,0.9); font-family: 'JetBrains Mono', monospace; margin-top: 4px;">
                            {move || last_active.get()}
                        </div>
                    </div>
                    <div>
                        <div style="font-size: 8px; color: rgba(255,245,240,0.5); font-family: 'Orbitron', monospace; letter-spacing: 1px;">"MESSAGES"</div>
                        <div style="font-size: 13px; color: rgba(255,245,240,0.9); font-family: 'JetBrains Mono', monospace; margin-top: 4px;">
                            {move || msg_count.get().to_string()}
                        </div>
                    </div>
                    <div>
                        <button
                            style="font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 1px; padding: 4px 10px; border-radius: 6px; cursor: pointer; border: 1px solid rgba(0,204,255,0.3); background: rgba(0,204,255,0.08); color: #00ccff; margin-top: 12px;"
                            on:click=move |_| do_poll()
                        >
                            {move || if polling.get() { "Polling..." } else { "\u{21BB} Refresh" }}
                        </button>
                    </div>
                </div>
            </Show>
        </Card>
    }
}

// ── T9: Agent-to-Agent Messaging ────────────────────────────────────────────

#[component]
fn AgentMessagingSection(agent_id: RwSignal<String>) -> impl IntoView {
    let agents = RwSignal::new(Vec::<api::NetworkAgent>::new());
    let msg_input = RwSignal::new(String::new());
    let target_agent = RwSignal::new(String::new());
    let send_status = RwSignal::new(String::new());
    let expanded = RwSignal::new(false);

    // Fetch agent list
    spawn_local(async move {
        if let Ok(resp) = api::fetch_agents().await {
            agents.set(resp.agents);
        }
    });

    let label_style = "font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,245,240,0.7); margin-bottom: 6px;";
    let input_style = "width: 100%; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.15); border-radius: 8px; padding: 10px 14px; color: rgba(255,245,240,0.9); font-family: 'Rajdhani', sans-serif; font-size: 14px; box-sizing: border-box; outline: none;";

    view! {
        <Card style="margin-top: 16px;">
            <div
                style="display: flex; justify-content: space-between; align-items: center; cursor: pointer;"
                on:click=move |_| expanded.set(!expanded.get())
            >
                <div style=label_style>"AGENT MESSAGING"</div>
                <span style="font-size: 12px; color: rgba(255,245,240,0.5);">
                    {move || if expanded.get() { "\u{25B2}" } else { "\u{25BC}" }}
                </span>
            </div>

            <Show when=move || expanded.get()>
                <div style="margin-top: 12px;">
                    <div style="margin-bottom: 8px;">
                        <div style="font-size: 8px; color: rgba(255,245,240,0.5); font-family: 'Orbitron', monospace; letter-spacing: 1px; margin-bottom: 4px;">"TARGET AGENT"</div>
                        <select
                            style="width: 100%; background: #0d0704; border: 1px solid rgba(255,60,20,0.15); border-radius: 8px; padding: 8px 12px; color: rgba(255,245,240,0.9); font-family: 'Rajdhani', sans-serif; font-size: 13px;"
                            prop:value=move || target_agent.get()
                            on:change=move |ev| target_agent.set(event_target_value(&ev))
                        >
                            <option value="">"Select agent..."</option>
                            {move || agents.get().into_iter()
                                .filter(|a| a.id != agent_id.get_untracked())
                                .map(|a| {
                                    let id = a.id.clone();
                                    let label = format!("{} ({})", a.name, a.role);
                                    view! { <option value=id>{label}</option> }
                                }).collect::<Vec<_>>()
                            }
                        </select>
                    </div>
                    <div style="display: flex; gap: 8px;">
                        <input type="text" placeholder="Message to send..."
                            style=input_style
                            prop:value=move || msg_input.get()
                            on:input=move |ev| msg_input.set(event_target_value(&ev))
                        />
                        <button
                            style="font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 1px; padding: 8px 16px; border-radius: 8px; cursor: pointer; border: 1px solid rgba(80,200,120,0.3); background: rgba(80,200,120,0.08); color: rgba(80,200,120,0.9); white-space: nowrap;"
                            on:click=move |_| {
                                let target = target_agent.get_untracked();
                                let message = msg_input.get_untracked();
                                if target.is_empty() || message.trim().is_empty() {
                                    send_status.set("Select a target agent and enter a message".into());
                                    return;
                                }
                                send_status.set("Sending...".into());
                                spawn_local(async move {
                                    match api::agent_send(&target, &message).await {
                                        Ok(_) => {
                                            send_status.set("\u{2713} Message sent".into());
                                            msg_input.set(String::new());
                                        }
                                        Err(e) => send_status.set(format!("Error: {}", e)),
                                    }
                                });
                            }
                        >"Send"</button>
                    </div>
                    {move || {
                        let s = send_status.get();
                        (!s.is_empty()).then(|| view! {
                            <div style="margin-top: 8px; font-size: 11px; color: rgba(80,200,120,0.8); font-family: 'JetBrains Mono', monospace;">
                                {s}
                            </div>
                        })
                    }}
                </div>
            </Show>
        </Card>
    }
}

#[component]
pub fn AgentEditorPage() -> impl IntoView {
    let params = use_params_map();
    let id_str = params.with_untracked(|p| p.get("id").map(|s| s.to_string()).unwrap_or_default());
    let agent_id = RwSignal::new(id_str);

    let a_name    = RwSignal::new(String::new());
    let a_role    = RwSignal::new(String::new());
    let a_model   = RwSignal::new(String::new());
    let a_auto    = RwSignal::new("standard".to_string());
    let a_persona = RwSignal::new(String::new());
    let a_soul    = RwSignal::new(String::new());
    let a_status  = RwSignal::new(String::new());
    let a_tools   = RwSignal::new(Vec::<String>::new());
    let a_tasks   = RwSignal::new(0u32);
    let a_created = RwSignal::new(String::new());
    let loaded    = RwSignal::new(false);
    let saving    = RwSignal::new(false);
    let deleting  = RwSignal::new(false);
    let msg       = RwSignal::new(String::new());
    let is_err    = RwSignal::new(false);
    let show_chat = RwSignal::new(false);
    let chat_input = RwSignal::new(String::new());
    let chat_response = RwSignal::new(String::new());

    spawn_local(async move {
        let id = agent_id.get_untracked();
        if !id.is_empty()
            && let Ok(a) = api::fetch_agent(&id).await {
                a_name.set(a.name);
                a_role.set(a.role);
                a_model.set(a.model);
                if !a.autonomy.is_empty() { a_auto.set(a.autonomy); }
                a_persona.set(a.persona);
                a_soul.set(a.soul);
                a_status.set(a.status);
                a_tools.set(a.tools);
                a_tasks.set(a.tasks);
                a_created.set(a.created);
            }
        loaded.set(true);
    });

    let do_save = move |_| {
        let n = a_name.get_untracked();
        if n.trim().is_empty() { is_err.set(true); msg.set("Name is required".into()); return; }
        let r  = a_role.get_untracked();
        let m  = a_model.get_untracked();
        let au = a_auto.get_untracked();
        let pe = a_persona.get_untracked();
        let so = a_soul.get_untracked();
        let id = agent_id.get_untracked();
        saving.set(true); is_err.set(false); msg.set(String::new());
        spawn_local(async move {
            let req = api::UpdateAgentReq {
                name: Some(n),
                role: if r.is_empty() { None } else { Some(r) },
                model: if m.is_empty() { None } else { Some(m) },
                autonomy: Some(au),
                persona: if pe.is_empty() { None } else { Some(pe) },
                soul: if so.is_empty() { None } else { Some(so) },
                status: None,
            };
            match api::update_agent(&id, &req).await {
                Ok(_) => { msg.set("Saved successfully".into()); is_err.set(false); }
                Err(e) => { msg.set(format!("Error: {}", e)); is_err.set(true); }
            }
            saving.set(false);
        });
    };

    let toggle_status = move |_| {
        let id = agent_id.get_untracked();
        let current = a_status.get_untracked();
        let new_status = if current == "active" { "inactive".to_string() } else { "active".to_string() };
        let new_s2 = new_status.clone();
        spawn_local(async move {
            let req = api::UpdateAgentReq {
                name: None, role: None, model: None, autonomy: None,
                persona: None, soul: None,
                status: Some(new_status),
            };
            if api::update_agent(&id, &req).await.is_ok() {
                a_status.set(new_s2);
            }
        });
    };

    let input_style = "width: 100%; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.15); border-radius: 8px; padding: 10px 14px; color: rgba(255,245,240,0.9); font-family: 'Rajdhani', sans-serif; font-size: 14px; box-sizing: border-box; outline: none;";
    let label_style = "font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,245,240,0.7); margin-bottom: 6px;";

    view! {
        <div style="padding: 32px; max-width: 720px;">
            <div style="display: flex; align-items: center; gap: 16px; margin-bottom: 24px;">
                <button
                    style="font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 2px; text-transform: uppercase; background: transparent; border: 1px solid rgba(255,60,20,0.1); color: rgba(255,245,240,0.7); padding: 6px 12px; border-radius: 6px; cursor: pointer;"
                    on:click=move |_| { let _ = web_sys::window().unwrap().location().assign("/agents"); }
                >"\u{2190} Agents"</button>
                <h1 style="font-family: 'Orbitron', monospace; font-size: 14px; letter-spacing: 6px; color: rgba(255,245,240,0.9); margin: 0;">"AGENT EDITOR"</h1>
                {move || {
                    let s = a_status.get();
                    (!s.is_empty()).then(|| view! {
                        <StatusDot status=s.clone() />
                        <Badge text=s />
                    })
                }}
            </div>

            <Show when=move || !loaded.get()>
                <div style="font-family: 'Rajdhani', sans-serif; font-size: 14px; color: rgba(255,245,240,0.7);">"Loading..."</div>
            </Show>

            <Show when=move || loaded.get()>
                // Info bar
                <div style="display: flex; gap: 12px; margin-bottom: 16px; flex-wrap: wrap;">
                    <MetricCard label="Tasks" value={a_tasks.get_untracked().to_string()} icon="missions" />
                    <MetricCard label="Tools" value={a_tools.get_untracked().len().to_string()} icon="tools" />
                    <MetricCard label="Created" value={{ let c = a_created.get_untracked(); if c.len() > 10 { c[..10].to_string() } else { c } }} icon="activity" />
                </div>

                <Card>
                    <div style="display: grid; grid-template-columns: 1fr 1fr; gap: 20px;">
                        <div>
                            <div style=label_style>"NAME *"</div>
                            <input type="text" placeholder="Agent name" style=input_style
                                prop:value=move || a_name.get()
                                on:input=move |ev| a_name.set(event_target_value(&ev))
                            />
                        </div>
                        <div>
                            <div style=label_style>"ROLE"</div>
                            <input type="text" placeholder="e.g. Research Assistant" style=input_style
                                prop:value=move || a_role.get()
                                on:input=move |ev| a_role.set(event_target_value(&ev))
                            />
                        </div>
                        <div>
                            <div style=label_style>"MODEL"</div>
                            <select
                                style="width: 100%; background: #0d0704; border: 1px solid rgba(255,60,20,0.15); border-radius: 8px; padding: 10px 14px; color: rgba(255,245,240,0.9); font-family: 'Rajdhani', sans-serif; font-size: 14px; box-sizing: border-box; outline: none;"
                                prop:value=move || a_model.get()
                                on:change=move |ev| a_model.set(event_target_value(&ev))
                            >
                                <option value="anthropic/claude-opus-4-8">"Claude Opus 4.8"</option>
                                <option value="anthropic/claude-sonnet-4-6">"Claude Sonnet 4.6"</option>
                                <option value="anthropic/claude-haiku-4-5">"Claude Haiku 4.5"</option>
                                <option value="openai/gpt-5.2">"GPT 5.2"</option>
                                <option value="google/gemini-3.1-pro">"Gemini 3.1 Pro"</option>
                                <option value="ollama/llama4">"Llama 4 (Ollama)"</option>
                            </select>
                        </div>
                        <div>
                            <div style=label_style>"AUTONOMY"</div>
                            <select
                                style="width: 100%; background: #0d0704; border: 1px solid rgba(255,60,20,0.15); border-radius: 8px; padding: 10px 14px; color: rgba(255,245,240,0.9); font-family: 'Rajdhani', sans-serif; font-size: 14px; box-sizing: border-box; outline: none;"
                                prop:value=move || a_auto.get()
                                on:change=move |ev| a_auto.set(event_target_value(&ev))
                            >
                                <option value="minimal">"Minimal \u{2014} supervised"</option>
                                <option value="standard">"Standard \u{2014} semi-autonomous"</option>
                                <option value="full">"Full \u{2014} autonomous"</option>
                            </select>
                        </div>
                    </div>
                    <div style="margin-top: 20px;">
                        <div style=label_style>"PERSONA"</div>
                        <textarea
                            placeholder="Personality traits and behavioral guidelines..."
                            style="width: 100%; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.15); border-radius: 8px; padding: 10px 14px; color: rgba(255,245,240,0.9); font-family: 'Rajdhani', sans-serif; font-size: 14px; box-sizing: border-box; outline: none; resize: vertical; min-height: 80px;"
                            prop:value=move || a_persona.get()
                            on:input=move |ev| a_persona.set(event_target_value(&ev))
                        />
                    </div>
                    <div style="margin-top: 16px;">
                        <div style=label_style>"SOUL"</div>
                        <textarea
                            placeholder="Deep identity, values, and core directives..."
                            style="width: 100%; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.15); border-radius: 8px; padding: 10px 14px; color: rgba(255,245,240,0.9); font-family: 'Rajdhani', sans-serif; font-size: 14px; box-sizing: border-box; outline: none; resize: vertical; min-height: 80px;"
                            prop:value=move || a_soul.get()
                            on:input=move |ev| a_soul.set(event_target_value(&ev))
                        />
                    </div>

                    // Tools display
                    <Show when=move || !a_tools.get().is_empty()>
                        <div style="margin-top: 16px;">
                            <div style=label_style>"ASSIGNED TOOLS"</div>
                            <div style="display: flex; flex-wrap: wrap; gap: 6px; margin-top: 6px;">
                                {move || a_tools.get().into_iter().map(|t| view! {
                                    <Badge text=t />
                                }).collect::<Vec<_>>()}
                            </div>
                        </div>
                    </Show>

                    <Show when=move || !msg.get().is_empty()>
                        <div style={move || format!("margin-top: 14px; font-family: 'Rajdhani', sans-serif; font-size: 13px; color: {};",
                            if is_err.get() { "rgba(255,60,20,0.9)" } else { "rgba(80,200,120,0.9)" }
                        )}>{move || msg.get()}</div>
                    </Show>

                    <div style="display: flex; justify-content: space-between; align-items: center; margin-top: 24px; padding-top: 20px; border-top: 1px solid rgba(255,60,20,0.15);">
                        <div style="display: flex; gap: 8px;">
                            <button
                                style="font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 2px; text-transform: uppercase; background: transparent; border: 1px solid rgba(255,60,20,0.2); color: rgba(255,60,20,0.6); padding: 6px 12px; border-radius: 6px; cursor: pointer;"
                                on:click=move |_| {
                                    let id = agent_id.get_untracked();
                                    deleting.set(true);
                                    spawn_local(async move {
                                        let _ = api::delete_agent(&id).await;
                                        let _ = web_sys::window().unwrap().location().assign("/agents");
                                    });
                                }
                            >
                                {move || if deleting.get() { "Deleting..." } else { "Delete Agent" }}
                            </button>
                            <Button on_click=Some(Callback::new(toggle_status))>
                                {move || if a_status.get() == "active" { "Deactivate" } else { "Activate" }}
                            </Button>
                        </div>
                        <div style="display: flex; gap: 8px;">
                            <Button on_click=Some(Callback::new(move |_| show_chat.set(!show_chat.get())))>"Chat"</Button>
                            <Button primary=true on_click=Some(Callback::new(do_save))>
                                {move || if saving.get() { "Saving..." } else { "Save Changes" }}
                            </Button>
                        </div>
                    </div>
                </Card>

                // Channel Binding (T8)
                <ChannelBindingSection agent_id=agent_id />

                // Live Status (T7)
                <LiveStatusSection agent_id=agent_id />

                // Agent Messaging (T9)
                <AgentMessagingSection agent_id=agent_id />

                // Chat panel
                <Show when=move || show_chat.get()>
                    <Card style="margin-top: 16px;">
                        <SectionTitle>"Chat with Agent"</SectionTitle>
                        <div style="display: flex; gap: 8px; margin-bottom: 12px;">
                            <input type="text" placeholder="Send a message to this agent..."
                                style=input_style
                                prop:value=move || chat_input.get()
                                on:input=move |ev| chat_input.set(event_target_value(&ev))
                                on:keydown=move |ev| {
                                    if ev.key() == "Enter" {
                                        let id = agent_id.get_untracked();
                                        let message = chat_input.get_untracked();
                                        if message.trim().is_empty() { return; }
                                        chat_response.set("Thinking...".to_string());
                                        spawn_local(async move {
                                            match api::agent_chat(&id, &message).await {
                                                Ok(r) => {
                                                    let resp = r.get("response").and_then(|v| v.as_str()).unwrap_or("No response").to_string();
                                                    chat_response.set(resp);
                                                }
                                                Err(e) => chat_response.set(format!("Error: {}", e)),
                                            }
                                        });
                                    }
                                }
                            />
                            <Button primary=true on_click=Some(Callback::new(move |_| {
                                let id = agent_id.get_untracked();
                                let message = chat_input.get_untracked();
                                if message.trim().is_empty() { return; }
                                chat_response.set("Thinking...".to_string());
                                spawn_local(async move {
                                    match api::agent_chat(&id, &message).await {
                                        Ok(r) => {
                                            let resp = r.get("response").and_then(|v| v.as_str()).unwrap_or("No response").to_string();
                                            chat_response.set(resp);
                                        }
                                        Err(e) => chat_response.set(format!("Error: {}", e)),
                                    }
                                });
                            }))>"Send"</Button>
                        </div>
                        <Show when=move || !chat_response.get().is_empty()>
                            <div style="padding: 12px 16px; background: rgba(255,255,255,0.02); border-radius: 8px; border: 1px solid rgba(255,60,20,0.06); font-size: 13px; color: rgba(255,245,240,0.9); line-height: 1.5; white-space: pre-wrap;">
                                {move || chat_response.get()}
                            </div>
                        </Show>
                    </Card>
                </Show>
            </Show>
        </div>
    }
}
