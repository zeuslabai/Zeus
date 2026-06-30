// ═══════════════════════════════════════════════════════════
// ZEUS — Agents Page — Phase 3: Dynamic Agent Spawning
// Persona/soul config, tool selection, task dispatch, status mgmt
// ═══════════════════════════════════════════════════════════

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;
use crate::api;
use crate::components::design::*;
use crate::components::sentient_orb::SentientOrb;

#[component]
pub fn AgentsPage() -> impl IntoView {
    let agents = RwSignal::new(Vec::<api::NetworkAgent>::new());
    let loading = RwSignal::new(true);
    let show_spawn = RwSignal::new(false);
    let show_detail = RwSignal::new(Option::<api::NetworkAgent>::None);
    let show_task = RwSignal::new(Option::<String>::None); // agent id for task dispatch
    let available_tools = RwSignal::new(Vec::<api::ToolDef>::new());

    // Spawn form signals
    let a_name = RwSignal::new(String::new());
    let a_role = RwSignal::new(String::new());
    let a_model = RwSignal::new(String::new()); // No hardcoded default — uses gateway config
    let a_auto = RwSignal::new("standard".to_string());
    let a_persona = RwSignal::new(String::new());
    let a_soul = RwSignal::new(String::new());
    let a_tools = RwSignal::new(Vec::<String>::new());
    let saving = RwSignal::new(false);
    let err = RwSignal::new(String::new());

    // Task dispatch signals
    let task_msg = RwSignal::new(String::new());
    let dispatching = RwSignal::new(false);
    let dispatch_result = RwSignal::new(String::new());

    // Hire modal signals
    let show_hire = RwSignal::new(Option::<String>::None);
    let hire_task = RwSignal::new(String::new());
    let hire_credits = RwSignal::new(String::new());
    let hiring = RwSignal::new(false);
    let hire_result = RwSignal::new(String::new());

    // Create team modal signals
    let show_team = RwSignal::new(false);
    let team_name = RwSignal::new(String::new());
    let team_sup = RwSignal::new(String::new());
    let team_err = RwSignal::new(String::new());
    let creating_team = RwSignal::new(false);

    // Run task modal signals
    let show_run_task = RwSignal::new(false);
    let rt_task = RwSignal::new(String::new());
    let rt_wait = RwSignal::new(false);
    let rt_running = RwSignal::new(false);
    let rt_result = RwSignal::new(String::new());

    // Fetch agents + tools on mount
    {
        spawn_local(async move {
            if let Ok(r) = api::fetch_agents().await { agents.set(r.agents); }
            if let Ok(t) = api::fetch_tools().await { available_tools.set(t.tools); }
            loading.set(false);
        });
    }

    let reload_agents = move || {
        spawn_local(async move {
            if let Ok(r) = api::fetch_agents().await { agents.set(r.agents); }
        });
    };

    // S67-D5: Auto-refresh agent list every 10 seconds
    {
        let agents_poll = agents;
        spawn_local(async move {
            loop {
                gloo_timers::future::sleep(std::time::Duration::from_secs(10)).await;
                if let Ok(r) = api::fetch_agents().await {
                    agents_poll.set(r.agents);
                }
            }
        });
    }

    let reset_form = move || {
        a_name.set(String::new());
        a_role.set(String::new());
        a_model.set(String::new());
        a_auto.set("standard".to_string());
        a_persona.set(String::new());
        a_soul.set(String::new());
        a_tools.set(Vec::new());
        err.set(String::new());
    };

    let do_spawn = move |_| {
        let n = a_name.get_untracked();
        if n.trim().is_empty() { err.set("Name is required".into()); return; }
        let r = a_role.get_untracked();
        let m = a_model.get_untracked();
        let au = a_auto.get_untracked();
        let p = a_persona.get_untracked();
        let s = a_soul.get_untracked();
        let tools = a_tools.get_untracked();
        saving.set(true); err.set(String::new());
        spawn_local(async move {
            let req = api::CreateAgentReq {
                name: n.clone(),
                role: if r.is_empty() { None } else { Some(r.clone()) },
                model: if m.is_empty() { None } else { Some(m.clone()) },
                autonomy_level: Some(au.clone()),
                persona: if p.is_empty() { None } else { Some(p.clone()) },
                tools: if tools.is_empty() { None } else { Some(tools.clone()) },
            };
            match api::create_agent(&req).await {
                Ok(_) => {
                    let sreq = api::SpawnAgentReq {
                        name: n,
                        role: if r.is_empty() { None } else { Some(r) },
                        model: if m.is_empty() { None } else { Some(m) },
                        autonomy: Some(au),
                        persona: if p.is_empty() { None } else { Some(p) },
                        soul: if s.is_empty() { None } else { Some(s) },
                        tools: if tools.is_empty() { None } else { Some(tools) },
                    };
                    if let Err(e) = api::spawn_agent(&sreq).await { web_sys::console::error_1(&format!("Spawn failed: {}", e).into()); }
                    show_spawn.set(false);
                    if let Ok(r2) = api::fetch_agents().await { agents.set(r2.agents); }
                }
                Err(e) => { err.set(format!("Error: {}", e)); }
            }
            saving.set(false);
        });
    };

    let toggle_tool = move |tool_name: String| {
        let mut current = a_tools.get_untracked();
        if current.contains(&tool_name) {
            current.retain(|t| t != &tool_name);
        } else {
            current.push(tool_name);
        }
        a_tools.set(current);
    };

    let do_dispatch = move |_| {
        let agent_id = match show_task.get_untracked() {
            Some(id) => id,
            None => return,
        };
        let msg = task_msg.get_untracked();
        if msg.trim().is_empty() { return; }
        dispatching.set(true);
        dispatch_result.set(String::new());
        spawn_local(async move {
            let req = api::DispatchMissionReq {
                message: format!("[Agent:{}] {}", agent_id, msg),
                session_id: None,
                system_prompt: None,
            };
            match api::dispatch_mission(&req).await {
                Ok(r) => {
                    dispatch_result.set(format!("Dispatched. Response: {}", &r.response[..120.min(r.response.len())]));
                    task_msg.set(String::new());
                }
                Err(e) => dispatch_result.set(format!("Error: {}", e)),
            }
            dispatching.set(false);
        });
    };

    let update_agent_status = move |agent_id: String, new_status: String| {
        spawn_local(async move {
            let req = api::UpdateAgentReq {
                status: Some(new_status),
                autonomy: None, model: None, name: None,
                role: None, persona: None, soul: None,
            };
            if let Err(e) = api::update_agent(&agent_id, &req).await { web_sys::console::error_1(&format!("Update failed: {}", e).into()); }
            if let Ok(r) = api::fetch_agents().await { agents.set(r.agents); }
        });
    };

    let do_hire = move |_| {
        let agent_id = match show_hire.get_untracked() { Some(id) => id, None => return };
        let task = hire_task.get_untracked();
        if task.trim().is_empty() { hire_result.set("Task is required".into()); return; }
        let credits: u64 = hire_credits.get_untracked().parse().unwrap_or(100);
        hiring.set(true);
        hire_result.set(String::new());
        spawn_local(async move {
            match api::hire_agent(&agent_id, &task, None, credits).await {
                Ok(_) => { hire_result.set("Agent hired successfully!".into()); hire_task.set(String::new()); }
                Err(e) => hire_result.set(format!("Error: {}", e)),
            }
            hiring.set(false);
        });
    };

    let do_create_team = move |_| {
        let name = team_name.get_untracked();
        if name.trim().is_empty() { team_err.set("Team name required".into()); return; }
        creating_team.set(true);
        team_err.set(String::new());
        spawn_local(async move {
            let sup = team_sup.get_untracked();
            let body = if sup.trim().is_empty() {
                serde_json::json!({ "name": name, "agents": [] })
            } else {
                serde_json::json!({ "name": name, "supervisor_id": sup, "agents": [] })
            };
            match api::create_agent_team(&body).await {
                Ok(_) => { show_team.set(false); team_name.set(String::new()); team_sup.set(String::new()); }
                Err(e) => team_err.set(format!("Error: {}", e)),
            }
            creating_team.set(false);
        });
    };

    let do_run_task = move |_| {
        let task = rt_task.get_untracked();
        if task.trim().is_empty() { rt_result.set("Task is required".into()); return; }
        let wait = rt_wait.get_untracked();
        rt_running.set(true);
        rt_result.set(String::new());
        spawn_local(async move {
            match api::run_agent_task(&task, None, None, wait).await {
                Ok(r) => rt_result.set(format!("Run queued: {}", r.get("run_id").and_then(|v| v.as_str()).unwrap_or("ok"))),
                Err(e) => rt_result.set(format!("Error: {}", e)),
            }
            rt_running.set(false);
        });
    };

    let input_style = "width: 100%; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 8px; padding: 10px 14px; color: rgba(255,245,240,0.9); font-family: 'Rajdhani', sans-serif; font-size: 14px; box-sizing: border-box; outline: none;";
    let label_style = "font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,245,240,0.7); margin-bottom: 6px;";
    let textarea_style = "width: 100%; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 8px; padding: 10px 14px; color: rgba(255,245,240,0.9); font-family: 'Rajdhani', sans-serif; font-size: 13px; box-sizing: border-box; outline: none; resize: vertical; line-height: 1.5;";

    view! {
        // ── Enhanced Spawn Modal ─────────────────────────────
        <Show when=move || show_spawn.get()>
            <div style="position: fixed; inset: 0; background: rgba(0,0,0,0.78); z-index: 1000; display: flex; align-items: center; justify-content: center; overflow-y: auto;">
                <div style="background: #0d0704; border: 1px solid rgba(255,60,20,0.25); border-radius: 16px; padding: 32px; width: 560px; max-width: 92vw; max-height: 90vh; overflow-y: auto; box-shadow: 0 0 60px rgba(255,60,20,0.15);">
                    <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 24px;">
                        <div style="font-family: 'Orbitron', monospace; font-size: 11px; letter-spacing: 5px; color: rgba(255,245,240,0.9);">"SPAWN AGENT"</div>
                        <SentientOrb size=36 mode="waking" />
                    </div>

                    <div style="display: flex; flex-direction: column; gap: 14px;">
                        // Name + Role row
                        <div style="display: grid; grid-template-columns: 1fr 1fr; gap: 12px;">
                            <div>
                                <div style=label_style>"NAME *"</div>
                                <input type="text" placeholder="e.g. Researcher" style=input_style
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
                        </div>

                        // Model + Autonomy row
                        <div style="display: grid; grid-template-columns: 1fr 1fr; gap: 12px;">
                            <div>
                                <div style=label_style>"MODEL"</div>
                                // Model input — no hardcoded list, uses configured model as default
                                <input type="text"
                                    style="width: 100%; background: #0d0704; border: 1px solid rgba(255,60,20,0.1); border-radius: 8px; padding: 10px 14px; color: rgba(255,245,240,0.9); font-family: 'JetBrains Mono', monospace; font-size: 13px; box-sizing: border-box; outline: none;"
                                    placeholder="provider/model-name (e.g. anthropic/claude-sonnet-4-6)"
                                    prop:value=move || a_model.get()
                                    on:input=move |ev| a_model.set(event_target_value(&ev))
                                />
                            </div>
                            <div>
                                <div style=label_style>"AUTONOMY"</div>
                                <select style="width: 100%; background: #0d0704; border: 1px solid rgba(255,60,20,0.1); border-radius: 8px; padding: 10px 14px; color: rgba(255,245,240,0.9); font-family: 'Rajdhani', sans-serif; font-size: 14px; box-sizing: border-box; outline: none;"
                                    on:change=move |ev| a_auto.set(event_target_value(&ev))
                                >
                                    <option value="minimal">"Minimal \u{2014} every action approved"</option>
                                    <option value="standard" selected>"Standard \u{2014} semi-autonomous"</option>
                                    <option value="full">"Full \u{2014} autonomous execution"</option>
                                </select>
                            </div>
                        </div>

                        // Persona
                        <div>
                            <div style=label_style>"PERSONA"</div>
                            <textarea rows=3 placeholder="Define the agent's personality, communication style, and behavioral traits..." style=textarea_style
                                prop:value=move || a_persona.get()
                                on:input=move |ev| a_persona.set(event_target_value(&ev))
                            />
                            <div style="font-size: 10px; color: rgba(255,245,240,0.2); margin-top: 2px;">"How the agent presents itself and communicates"</div>
                        </div>

                        // Soul
                        <div>
                            <div style=label_style>"SOUL"</div>
                            <textarea rows=3 placeholder="Define the agent's core purpose, values, and decision-making principles..." style=textarea_style
                                prop:value=move || a_soul.get()
                                on:input=move |ev| a_soul.set(event_target_value(&ev))
                            />
                            <div style="font-size: 10px; color: rgba(255,245,240,0.2); margin-top: 2px;">"The agent's inner directive \u{2014} guides autonomous decisions"</div>
                        </div>

                        // Tool selection
                        <div>
                            <div style=label_style>{move || format!("TOOLS ({})", a_tools.get().len())}</div>
                            <div style="max-height: 160px; overflow-y: auto; border: 1px solid rgba(255,60,20,0.08); border-radius: 8px; padding: 8px;">
                                {move || {
                                    let tools = available_tools.get();
                                    let selected = a_tools.get();
                                    // Group by category, show top categories
                                    let categories = vec!["system", "files", "browser", "messaging", "git", "other"];
                                    categories.into_iter().map(|cat| {
                                        let cat_tools: Vec<_> = tools.iter()
                                            .filter(|t| t.category == cat)
                                            .collect();
                                        if cat_tools.is_empty() { return view! { <div /> }.into_any(); }
                                        view! {
                                            <div style="margin-bottom: 6px;">
                                                <div style="font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 2px; color: rgba(255,60,20,0.4); margin-bottom: 4px; text-transform: uppercase;">{cat}</div>
                                                <div style="display: flex; flex-wrap: wrap; gap: 4px;">
                                                    {cat_tools.into_iter().map(|t| {
                                                        let name = t.name.clone();
                                                        let name2 = name.clone();
                                                        let is_sel = selected.contains(&name);
                                                        let bg = if is_sel { "rgba(255,60,20,0.15)" } else { "rgba(255,255,255,0.02)" };
                                                        let border = if is_sel { "rgba(255,60,20,0.4)" } else { "rgba(255,60,20,0.08)" };
                                                        let color = if is_sel { "rgba(255,140,80,1)" } else { "rgba(255,245,240,0.5)" };
                                                        view! {
                                                            // design: left raw — stateful selection toggle (bg/border/color flip on is_sel)
                                                            <button
                                                                on:click={let toggle = toggle_tool; move |_| toggle(name2.clone())}
                                                                style={format!("font-family: 'Rajdhani', sans-serif; font-size: 10px; padding: 2px 8px; border-radius: 4px; cursor: pointer; background: {}; border: 1px solid {}; color: {}; transition: all 0.15s;", bg, border, color)}
                                                            >{name}</button>
                                                        }
                                                    }).collect::<Vec<_>>()}
                                                </div>
                                            </div>
                                        }.into_any()
                                    }).collect::<Vec<_>>()
                                }}
                            </div>
                        </div>
                    </div>

                    <Show when=move || !err.get().is_empty()>
                        <div style="margin-top: 12px; font-family: 'Rajdhani', sans-serif; font-size: 13px; color: rgba(255,60,20,0.9);">{move || err.get()}</div>
                    </Show>

                    <div style="display: flex; gap: 10px; margin-top: 20px; justify-content: flex-end;">
                        <Button on_click=Some(Callback::new(move |_| { show_spawn.set(false); reset_form(); }))>"Cancel"</Button>
                        <Button primary=true on_click=Some(Callback::new(do_spawn))>
                            {move || if saving.get() { "Spawning..." } else { "Spawn Agent" }}
                        </Button>
                    </div>
                </div>
            </div>
        </Show>

        // ── Task Dispatch Modal ──────────────────────────────
        <Show when=move || show_task.get().is_some()>
            <div style="position: fixed; inset: 0; background: rgba(0,0,0,0.78); z-index: 1000; display: flex; align-items: center; justify-content: center;">
                <div style="background: #0d0704; border: 1px solid rgba(255,60,20,0.25); border-radius: 16px; padding: 32px; width: 520px; max-width: 92vw; box-shadow: 0 0 60px rgba(255,60,20,0.15);">
                    <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 20px;">
                        <div style="font-family: 'Orbitron', monospace; font-size: 11px; letter-spacing: 5px; color: rgba(255,245,240,0.9);">"DISPATCH TASK"</div>
                        <div style="font-size: 11px; color: rgba(255,245,240,0.7);">
                            {move || show_task.get().unwrap_or_default()}
                        </div>
                    </div>
                    <textarea
                        rows=5
                        placeholder="Describe the task for this agent...&#10;&#10;The agent will execute autonomously based on its configuration."
                        style=textarea_style
                        prop:value=move || task_msg.get()
                        on:input=move |ev| task_msg.set(event_target_value(&ev))
                    />
                    <Show when=move || !dispatch_result.get().is_empty()>
                        <div style="margin-top: 10px; font-size: 12px; color: rgba(255,245,240,0.6); background: rgba(255,255,255,0.02); padding: 10px; border-radius: 6px; max-height: 120px; overflow-y: auto;">
                            {move || dispatch_result.get()}
                        </div>
                    </Show>
                    <div style="display: flex; gap: 10px; margin-top: 16px; justify-content: flex-end;">
                        <Button on_click=Some(Callback::new(move |_| { show_task.set(None); task_msg.set(String::new()); dispatch_result.set(String::new()); }))>"Close"</Button>
                        <Button primary=true on_click=Some(Callback::new(do_dispatch))>
                            {move || if dispatching.get() { "Dispatching..." } else { "Dispatch" }}
                        </Button>
                    </div>
                </div>
            </div>
        </Show>

        // ── Agent Detail Overlay ─────────────────────────────
        <Show when=move || show_detail.get().is_some()>
            <div style="position: fixed; inset: 0; background: rgba(0,0,0,0.78); z-index: 1000; display: flex; align-items: center; justify-content: center; overflow-y: auto;">
                <div style="background: #0d0704; border: 1px solid rgba(255,60,20,0.25); border-radius: 16px; padding: 32px; width: 600px; max-width: 92vw; max-height: 90vh; overflow-y: auto; box-shadow: 0 0 60px rgba(255,60,20,0.15);">
                    {move || {
                        let a = match show_detail.get() {
                            Some(a) => a,
                            None => return view! { <div /> }.into_any(),
                        };
                        let is_active = a.status == "active";
                        let agent_id = a.id.clone();
                        let agent_id2 = a.id.clone();
                                                view! {
                            <div>
                                // Header with orb
                                <div style="display: flex; align-items: center; gap: 16px; margin-bottom: 24px;">
                                    <SentientOrb size=64 mode={if is_active { "active" } else { "dormant" }} />
                                    <div style="flex: 1;">
                                        <div style="display: flex; align-items: center; gap: 10px;">
                                            <span style="font-family: 'Orbitron', monospace; font-size: 16px; color: rgba(255,245,240,0.95); font-weight: 700;">{a.name.clone()}</span>
                                            <Badge text={a.status.clone()} color={if is_active { "#22c55e".to_string() } else { "#eab308".to_string() }} />
                                        </div>
                                        <div style="font-size: 13px; color: rgba(255,245,240,0.5); margin-top: 4px;">{a.role.clone()}</div>
                                    </div>
                                    <Button small=true on_click=Some(Callback::new(move |_| show_detail.set(None))) style="padding: 4px 10px;">"CLOSE"</Button>
                                </div>

                                // Stat grid
                                <div style="display: grid; grid-template-columns: 1fr 1fr 1fr; gap: 10px; margin-bottom: 20px;">
                                    <div style="padding: 12px; background: rgba(255,255,255,0.02); border-radius: 8px;">
                                        <div style="font-family: 'Orbitron', monospace; font-size: 8px; color: rgba(255,245,240,0.5); letter-spacing: 2px; margin-bottom: 4px;">"MODEL"</div>
                                        <div style="font-size: 12px; color: rgba(255,245,240,0.9);">
                                            {if a.model.is_empty() { "\u{2014}".to_string() } else { a.model.clone() }}
                                        </div>
                                    </div>
                                    <div style="padding: 12px; background: rgba(255,255,255,0.02); border-radius: 8px;">
                                        <div style="font-family: 'Orbitron', monospace; font-size: 8px; color: rgba(255,245,240,0.5); letter-spacing: 2px; margin-bottom: 4px;">"AUTONOMY"</div>
                                        <div style="font-size: 12px; color: rgba(255,245,240,0.9);">{if a.autonomy.is_empty() { "standard".to_string() } else { a.autonomy.clone() }}</div>
                                    </div>
                                    <div style="padding: 12px; background: rgba(255,255,255,0.02); border-radius: 8px;">
                                        <div style="font-family: 'Orbitron', monospace; font-size: 8px; color: rgba(255,245,240,0.5); letter-spacing: 2px; margin-bottom: 4px;">"TASKS"</div>
                                        <div style="font-size: 12px; color: rgba(255,245,240,0.9);">{a.tasks.to_string()}</div>
                                    </div>
                                </div>

                                // Persona
                                {(!a.persona.is_empty()).then(|| view! {
                                    <div style="margin-bottom: 16px;">
                                        <div style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,245,240,0.5); margin-bottom: 6px;">"PERSONA"</div>
                                        <div style="font-size: 13px; color: rgba(255,245,240,0.7); background: rgba(255,255,255,0.02); padding: 12px; border-radius: 8px; line-height: 1.6; border-left: 3px solid rgba(255,60,20,0.2);">
                                            {a.persona.clone()}
                                        </div>
                                    </div>
                                })}

                                // Soul
                                {(!a.soul.is_empty()).then(|| view! {
                                    <div style="margin-bottom: 16px;">
                                        <div style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,245,240,0.5); margin-bottom: 6px;">"SOUL"</div>
                                        <div style="font-size: 13px; color: rgba(255,245,240,0.7); background: rgba(255,255,255,0.02); padding: 12px; border-radius: 8px; line-height: 1.6; border-left: 3px solid rgba(255,140,80,0.3);">
                                            {a.soul.clone()}
                                        </div>
                                    </div>
                                })}

                                // Tools
                                {(!a.tools.is_empty()).then(|| view! {
                                    <div style="margin-bottom: 16px;">
                                        <div style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,245,240,0.5); margin-bottom: 6px;">
                                            {format!("TOOLS ({})", a.tools.len())}
                                        </div>
                                        <div style="display: flex; flex-wrap: wrap; gap: 4px;">
                                            {a.tools.iter().map(|t| view! {
                                                <span style="font-size: 10px; padding: 2px 8px; border-radius: 4px; background: rgba(255,60,20,0.08); border: 1px solid rgba(255,60,20,0.15); color: rgba(255,140,80,0.8);">{t.clone()}</span>
                                            }).collect::<Vec<_>>()}
                                        </div>
                                    </div>
                                })}

                                // Created + ID
                                <div style="font-size: 10px; color: rgba(255,245,240,0.2); margin-bottom: 20px;">
                                    {format!("ID: {} \u{2022} Created: {}", a.id, a.created)}
                                </div>

                                // Actions
                                <div style="display: flex; gap: 8px; flex-wrap: wrap;">
                                    <Button primary=true on_click=Some(Callback::new({let aid = agent_id.clone(); move |_| {
                                        show_detail.set(None);
                                        show_task.set(Some(aid.clone()));
                                    }}))>
                                        <Icon name="zap" size=12 /> " Dispatch Task"
                                    </Button>
                                    {if is_active {
                                        let id = agent_id2.clone();
                                        let update = update_agent_status;
                                        view! {
                                            <Button on_click=Some(Callback::new(move |_| {
                                                update(id.clone(), "inactive".to_string());
                                                show_detail.set(None);
                                            }))>" Deactivate"</Button>
                                        }.into_any()
                                    } else {
                                        let id = agent_id2.clone();
                                        let update = update_agent_status;
                                        view! {
                                            <Button on_click=Some(Callback::new(move |_| {
                                                update(id.clone(), "active".to_string());
                                                show_detail.set(None);
                                            }))>" Activate"</Button>
                                        }.into_any()
                                    }}
                                    <Button small=true on_click=Some(Callback::new({let aid = agent_id2.clone(); move |_| {
                                            let url = format!("/studio?agent_id={}", aid);
                                            let _ = web_sys::window().unwrap().location().assign(&url);
                                        }})) style="padding: 6px 14px;"><Icon name="chat" size=12 /> " Chat"</Button>
                                </div>
                            </div>
                        }.into_any()
                    }}
                </div>
            </div>
        </Show>

        // ── Hire Agent Modal ────────────────────────────────
        <Show when=move || show_hire.get().is_some()>
            <div style="position: fixed; inset: 0; background: rgba(0,0,0,0.78); z-index: 1000; display: flex; align-items: center; justify-content: center;">
                <div style="background: #0d0704; border: 1px solid rgba(59,130,246,0.25); border-radius: 16px; padding: 32px; width: 460px; max-width: 92vw; box-shadow: 0 0 60px rgba(59,130,246,0.1);">
                    <div style="font-family: 'Orbitron', monospace; font-size: 11px; letter-spacing: 5px; color: rgba(255,245,240,0.9); margin-bottom: 6px;">"HIRE AGENT"</div>
                    <div style="font-size: 11px; color: rgba(255,245,240,0.4); margin-bottom: 20px;">{move || show_hire.get().unwrap_or_default()}</div>
                    <div style="display: flex; flex-direction: column; gap: 12px;">
                        <div>
                            <div style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,245,240,0.5); margin-bottom: 6px;">"TASK *"</div>
                            <textarea rows=3 placeholder="Describe the task for this agent..." style="width: 100%; background: rgba(255,255,255,0.03); border: 1px solid rgba(59,130,246,0.15); border-radius: 8px; padding: 10px 14px; color: rgba(255,245,240,0.9); font-family: 'Rajdhani', sans-serif; font-size: 13px; box-sizing: border-box; outline: none; resize: vertical;"
                                prop:value=move || hire_task.get()
                                on:input=move |ev| hire_task.set(event_target_value(&ev))
                            />
                        </div>
                        <div>
                            <div style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,245,240,0.5); margin-bottom: 6px;">"MAX CREDITS (default 100)"</div>
                            <input type="number" placeholder="100" style="width: 100%; background: rgba(255,255,255,0.03); border: 1px solid rgba(59,130,246,0.15); border-radius: 8px; padding: 10px 14px; color: rgba(255,245,240,0.9); font-family: 'Rajdhani', sans-serif; font-size: 13px; box-sizing: border-box; outline: none;"
                                prop:value=move || hire_credits.get()
                                on:input=move |ev| hire_credits.set(event_target_value(&ev))
                            />
                        </div>
                    </div>
                    <Show when=move || !hire_result.get().is_empty()>
                        <div style={move || format!("margin-top: 10px; font-size: 12px; color: {};", if hire_result.get().starts_with("Error") { "rgba(239,68,68,0.8)" } else { "rgba(34,197,94,0.8)" })}>
                            {move || hire_result.get()}
                        </div>
                    </Show>
                    <div style="display: flex; gap: 10px; margin-top: 20px; justify-content: flex-end;">
                        <Button on_click=Some(Callback::new(move |_| { show_hire.set(None); hire_task.set(String::new()); hire_result.set(String::new()); }))>"Cancel"</Button>
                        <Button primary=true on_click=Some(Callback::new(do_hire))>
                            {move || if hiring.get() { "Hiring..." } else { "Hire Agent" }}
                        </Button>
                    </div>
                </div>
            </div>
        </Show>

        // ── Create Team Modal ────────────────────────────────
        <Show when=move || show_team.get()>
            <div style="position: fixed; inset: 0; background: rgba(0,0,0,0.78); z-index: 1000; display: flex; align-items: center; justify-content: center;">
                <div style="background: #0d0704; border: 1px solid rgba(168,85,247,0.25); border-radius: 16px; padding: 32px; width: 440px; max-width: 92vw; box-shadow: 0 0 60px rgba(168,85,247,0.1);">
                    <div style="font-family: 'Orbitron', monospace; font-size: 11px; letter-spacing: 5px; color: rgba(255,245,240,0.9); margin-bottom: 20px;">"CREATE TEAM"</div>
                    <div style="display: flex; flex-direction: column; gap: 12px;">
                        <div>
                            <div style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,245,240,0.5); margin-bottom: 6px;">"TEAM NAME *"</div>
                            <input type="text" placeholder="e.g. Research Team" style="width: 100%; background: rgba(255,255,255,0.03); border: 1px solid rgba(168,85,247,0.15); border-radius: 8px; padding: 10px 14px; color: rgba(255,245,240,0.9); font-family: 'Rajdhani', sans-serif; font-size: 14px; box-sizing: border-box; outline: none;"
                                prop:value=move || team_name.get()
                                on:input=move |ev| team_name.set(event_target_value(&ev))
                            />
                        </div>
                        <div>
                            <div style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,245,240,0.5); margin-bottom: 6px;">"SUPERVISOR AGENT ID (optional)"</div>
                            <input type="text" placeholder="agent-uuid or leave blank" style="width: 100%; background: rgba(255,255,255,0.03); border: 1px solid rgba(168,85,247,0.15); border-radius: 8px; padding: 10px 14px; color: rgba(255,245,240,0.9); font-family: 'Rajdhani', sans-serif; font-size: 14px; box-sizing: border-box; outline: none;"
                                prop:value=move || team_sup.get()
                                on:input=move |ev| team_sup.set(event_target_value(&ev))
                            />
                        </div>
                    </div>
                    <Show when=move || !team_err.get().is_empty()>
                        <div style="margin-top: 10px; font-size: 12px; color: rgba(239,68,68,0.8);">{move || team_err.get()}</div>
                    </Show>
                    <div style="display: flex; gap: 10px; margin-top: 20px; justify-content: flex-end;">
                        <Button on_click=Some(Callback::new(move |_| { show_team.set(false); team_name.set(String::new()); team_err.set(String::new()); }))>"Cancel"</Button>
                        <Button primary=true on_click=Some(Callback::new(do_create_team))>
                            {move || if creating_team.get() { "Creating..." } else { "Create Team" }}
                        </Button>
                    </div>
                </div>
            </div>
        </Show>

        // ── Run Task Modal ───────────────────────────────────
        <Show when=move || show_run_task.get()>
            <div style="position: fixed; inset: 0; background: rgba(0,0,0,0.78); z-index: 1000; display: flex; align-items: center; justify-content: center;">
                <div style="background: #0d0704; border: 1px solid rgba(234,179,8,0.25); border-radius: 16px; padding: 32px; width: 480px; max-width: 92vw; box-shadow: 0 0 60px rgba(234,179,8,0.1);">
                    <div style="font-family: 'Orbitron', monospace; font-size: 11px; letter-spacing: 5px; color: rgba(255,245,240,0.9); margin-bottom: 6px;">"RUN TASK"</div>
                    <div style="font-size: 11px; color: rgba(255,245,240,0.4); margin-bottom: 20px;">"Ad-hoc autonomous task execution"</div>
                    <div style="display: flex; flex-direction: column; gap: 12px;">
                        <div>
                            <div style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,245,240,0.5); margin-bottom: 6px;">"TASK *"</div>
                            <textarea rows=4 placeholder="Describe the task to execute autonomously..." style="width: 100%; background: rgba(255,255,255,0.03); border: 1px solid rgba(234,179,8,0.15); border-radius: 8px; padding: 10px 14px; color: rgba(255,245,240,0.9); font-family: 'Rajdhani', sans-serif; font-size: 13px; box-sizing: border-box; outline: none; resize: vertical;"
                                prop:value=move || rt_task.get()
                                on:input=move |ev| rt_task.set(event_target_value(&ev))
                            />
                        </div>
                        <div style="display: flex; align-items: center; gap: 10px;">
                            <input type="checkbox" id="rt_wait"
                                prop:checked=move || rt_wait.get()
                                on:change=move |ev| {
                                    use wasm_bindgen::JsCast;
                                    let checked = ev.target().and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok()).map(|i| i.checked()).unwrap_or(false);
                                    rt_wait.set(checked);
                                }
                            />
                            <label for="rt_wait" style="font-size: 13px; color: rgba(255,245,240,0.7); cursor: pointer;">"Wait for result (synchronous)"</label>
                        </div>
                    </div>
                    <Show when=move || !rt_result.get().is_empty()>
                        <div style={move || format!("margin-top: 10px; font-size: 12px; color: {};", if rt_result.get().starts_with("Error") { "rgba(239,68,68,0.8)" } else { "rgba(34,197,94,0.8)" })}>
                            {move || rt_result.get()}
                        </div>
                    </Show>
                    <div style="display: flex; gap: 10px; margin-top: 20px; justify-content: flex-end;">
                        <Button on_click=Some(Callback::new(move |_| { show_run_task.set(false); rt_task.set(String::new()); rt_result.set(String::new()); }))>"Cancel"</Button>
                        <Button primary=true on_click=Some(Callback::new(do_run_task))>
                            {move || if rt_running.get() { "Running..." } else { "Run Task" }}
                        </Button>
                    </div>
                </div>
            </div>
        </Show>

        // ── Main Page ────────────────────────────────────────
        <div style="padding: 32px;">
            // Header
            <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 24px;">
                <div>
                    <h1 style="font-family: 'Orbitron', monospace; font-size: 14px; letter-spacing: 6px; color: rgba(255,245,240,0.9); margin: 0;">"AGENTS"</h1>
                    <p style="font-size: 12px; color: rgba(255,245,240,0.7); margin: 4px 0 0;">{move || {
                        let a = agents.get();
                        let active = a.iter().filter(|ag| ag.status == "active").count();
                        if loading.get() { "Loading agents...".to_string() }
                        else if a.is_empty() { "No agents configured".to_string() }
                        else { format!("{} agents \u{2022} {} active \u{2022} {} tools available", a.len(), active, available_tools.get().len()) }
                    }}</p>
                </div>
                <div style="display: flex; gap: 8px;">
                    <Button small=true on_click=Some(Callback::new(move |_| { rt_task.set(String::new()); rt_result.set(String::new()); show_run_task.set(true); })) style="background: rgba(234,179,8,0.1); border: 1px solid rgba(234,179,8,0.3); color: rgba(234,179,8,0.9); padding: 8px 14px; border-radius: 8px;">"⚡ RUN TASK"</Button>
                    <Button small=true on_click=Some(Callback::new(move |_| { team_name.set(String::new()); team_err.set(String::new()); show_team.set(true); })) style="background: rgba(168,85,247,0.1); border: 1px solid rgba(168,85,247,0.3); color: rgba(168,85,247,0.9); padding: 8px 14px; border-radius: 8px;">"⬡ CREATE TEAM"</Button>
                    <Button primary=true on_click=Some(Callback::new(move |_| { reset_form(); show_spawn.set(true); }))>
                        <Icon name="plus" size=12 /> " Spawn Agent"
                    </Button>
                </div>
            </div>

            // Empty state
            <Show when=move || !loading.get() && agents.get().is_empty()>
                <div style="display: flex; flex-direction: column; align-items: center; justify-content: center; min-height: 300px; gap: 16px;">
                    <SentientOrb size=80 mode="dormant" />
                    <div style="text-align: center;">
                        <div style="font-family: 'Orbitron', monospace; font-size: 12px; letter-spacing: 4px; color: rgba(255,245,240,0.9); margin-bottom: 8px;">"NO AGENTS SPAWNED"</div>
                        <div style="font-size: 13px; color: rgba(255,245,240,0.7); max-width: 400px; line-height: 1.6;">
                            "Spawn agents with custom personas, souls, and tool access to build autonomous pipelines."
                        </div>
                    </div>
                    <Button primary=true on_click=Some(Callback::new(move |_| { reset_form(); show_spawn.set(true); }))>
                        <Icon name="plus" size=12 /> " Spawn First Agent"
                    </Button>
                </div>
            </Show>

            // Agent grid
            <Show when=move || !agents.get().is_empty()>
                <div style="display: grid; grid-template-columns: repeat(auto-fill, minmax(340px, 1fr)); gap: 16px;">
                    {move || agents.get().into_iter().map(|a| {
                        let is_active = a.status == "active";
                        let orb_mode = if is_active { "active" } else { "dormant" };
                        let status_str = a.status.clone();
                        let agent_for_detail = a.clone();
                        let agent_id_del = a.id.clone();
                        let agent_id_task = a.id.clone();
                        let agent_id_hire = a.id.clone();
                        let has_persona = !a.persona.is_empty();
                        let has_tools = !a.tools.is_empty();
                        let tool_count = a.tools.len();
                        view! {
                            <Card glow=is_active>
                                // Header
                                <div style="display: flex; align-items: center; gap: 14px; margin-bottom: 14px;">
                                    <div
                                        style="cursor: pointer;"
                                        on:click={let ad = agent_for_detail.clone(); move |_| show_detail.set(Some(ad.clone()))}
                                    >
                                        <SentientOrb size=48 mode=orb_mode />
                                    </div>
                                    <div style="flex: 1; min-width: 0;">
                                        <div style="display: flex; align-items: center; gap: 8px;">
                                            <span
                                                style="font-family: 'Orbitron', monospace; font-size: 13px; color: rgba(255,245,240,0.95); font-weight: 700; cursor: pointer;"
                                                on:click={let ad = agent_for_detail.clone(); move |_| show_detail.set(Some(ad.clone()))}
                                            >{a.name.clone()}</span>
                                            <StatusDot status=status_str />
                                        </div>
                                        <div style="font-size: 12px; color: rgba(255,245,240,0.7); overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">{a.role.clone()}</div>
                                    </div>
                                </div>

                                // Stats row
                                <div style="display: grid; grid-template-columns: 1fr 1fr 1fr; gap: 6px; margin-bottom: 12px;">
                                    <div style="padding: 6px 8px; background: rgba(255,255,255,0.02); border-radius: 6px; text-align: center;">
                                        <div style="font-family: 'Orbitron', monospace; font-size: 7px; color: rgba(255,245,240,0.2); letter-spacing: 1px;">"MODEL"</div>
                                        <div style="font-size: 10px; color: rgba(255,245,240,0.8); margin-top: 2px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">
                                            {if a.model.is_empty() { "\u{2014}".to_string() } else {
                                                // Show short model name
                                                a.model.split('/').next_back().unwrap_or(&a.model).to_string()
                                            }}
                                        </div>
                                    </div>
                                    <div style="padding: 6px 8px; background: rgba(255,255,255,0.02); border-radius: 6px; text-align: center;">
                                        <div style="font-family: 'Orbitron', monospace; font-size: 7px; color: rgba(255,245,240,0.2); letter-spacing: 1px;">"TASKS"</div>
                                        <div style="font-size: 10px; color: rgba(255,245,240,0.8); margin-top: 2px;">{a.tasks.to_string()}</div>
                                    </div>
                                    <div style="padding: 6px 8px; background: rgba(255,255,255,0.02); border-radius: 6px; text-align: center;">
                                        <div style="font-family: 'Orbitron', monospace; font-size: 7px; color: rgba(255,245,240,0.2); letter-spacing: 1px;">"TOOLS"</div>
                                        <div style="font-size: 10px; color: rgba(255,245,240,0.8); margin-top: 2px;">{tool_count.to_string()}</div>
                                    </div>
                                </div>

                                // Persona preview
                                {has_persona.then(|| view! {
                                    <div style="font-size: 11px; color: rgba(255,245,240,0.5); margin-bottom: 10px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; padding: 6px 8px; background: rgba(255,255,255,0.015); border-radius: 4px; border-left: 2px solid rgba(255,60,20,0.15);">
                                        {a.persona[..80.min(a.persona.len())].to_string()}
                                    </div>
                                })}

                                // Tool tags (first 6)
                                {has_tools.then(|| view! {
                                    <div style="display: flex; flex-wrap: wrap; gap: 3px; margin-bottom: 10px;">
                                        {a.tools.iter().take(6).map(|t| view! {
                                            <span style="font-size: 9px; padding: 1px 6px; border-radius: 3px; background: rgba(255,60,20,0.06); border: 1px solid rgba(255,60,20,0.1); color: rgba(255,140,80,0.7);">{t.clone()}</span>
                                        }).collect::<Vec<_>>()}
                                        {(a.tools.len() > 6).then(|| view! {
                                            <span style="font-size: 9px; padding: 1px 6px; color: rgba(255,245,240,0.3);">{format!("+{}", a.tools.len() - 6)}</span>
                                        })}
                                    </div>
                                })}

                                // Action buttons
                                <div style="display: flex; gap: 4px;">
                                    <Button primary=true small=true on_click=Some(Callback::new({let id = agent_id_task.clone(); move |_| show_task.set(Some(id.clone()))})) style="padding: 5px 10px; flex: 1;">"Dispatch"</Button>
                                    <Button small=true on_click=Some(Callback::new({let id = agent_id_hire.clone(); move |_| { show_hire.set(Some(id.clone())); hire_task.set(String::new()); hire_result.set(String::new()); }})) style="background: rgba(59,130,246,0.08); border: 1px solid rgba(59,130,246,0.2); color: rgba(59,130,246,0.8); padding: 5px 10px;">"Hire"</Button>
                                    <Button small=true on_click=Some(Callback::new({let ad = agent_for_detail.clone(); move |_| show_detail.set(Some(ad.clone()))})) style="padding: 5px 10px; flex: 1;">"Details"</Button>
                                    <Button small=true on_click=Some(Callback::new(move |_| {
                                            let _ = web_sys::window().unwrap().location().assign("/studio");
                                        })) style="padding: 5px 10px;">"Chat"</Button>
                                    <Button small=true on_click=Some(Callback::new({let del_id = agent_id_del.clone(); move |_| {
                                            let del_id = del_id.clone();
                                            // P0 fix: confirmation before delete
                                            if let Some(win) = web_sys::window() {
                                                if win.confirm_with_message(&format!("Delete agent '{}'? This cannot be undone.", del_id)).unwrap_or(false) {
                                                    spawn_local(async move {
                                                        if let Err(e) = api::delete_agent(&del_id).await { web_sys::console::error_1(&format!("Delete failed: {}", e).into()); }
                                                        reload_agents();
                                                    });
                                                }
                                            }
                                        }})) style="background: rgba(239,68,68,0.06); border: 1px solid rgba(239,68,68,0.12); color: rgba(239,68,68,0.5); padding: 5px 8px; border-radius: 5px;">"DEL"</Button>
                                </div>
                            </Card>
                        }
                    }).collect::<Vec<_>>()}
                </div>
            </Show>
        </div>
    }
}
