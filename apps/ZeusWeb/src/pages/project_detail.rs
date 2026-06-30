// ═══════════════════════════════════════════════════════════
// ZEUS — Project Detail Page — Phase 2: Wired to API
// ═══════════════════════════════════════════════════════════

use leptos::prelude::*;
use leptos_router::hooks::use_params_map;
use wasm_bindgen_futures::spawn_local;
use crate::api;
use crate::components::design::*;

#[component]
pub fn ProjectDetailPage() -> impl IntoView {
    let params = use_params_map();
    let project = RwSignal::new(Option::<api::Project>::None);
    let agents = RwSignal::new(Vec::<api::NetworkAgent>::new());
    let loading = RwSignal::new(true);
    let error = RwSignal::new(String::new());
    let editing = RwSignal::new(false);
    let assigning = RwSignal::new(false);
    let selected_agents = RwSignal::new(Vec::<String>::new());
    let edit_name = RwSignal::new(String::new());
    let edit_desc = RwSignal::new(String::new());
    let edit_status = RwSignal::new(String::new());

    {
        let project = project;
        let agents = agents;
        let loading = loading;
        let error = error;
        let id = params.get_untracked().get("id").unwrap_or_default().to_string();
        spawn_local(async move {
            if id.is_empty() {
                error.set("No project ID provided".to_string());
                loading.set(false);
                return;
            }
            match api::fetch_project(&id).await {
                Ok(p) => { project.set(Some(p)); }
                Err(e) => { error.set(e); }
            }
            if let Ok(a) = api::fetch_agents().await {
                agents.set(a.agents);
            }
            loading.set(false);
        });
    }

    view! {
        <div style="padding: 32px;">
            {move || {
                if loading.get() {
                    return view! {
                        <div style="display: flex; align-items: center; justify-content: center; min-height: 300px; color: rgba(255,245,240,0.7); font-size: 13px;">
                            "Loading project..."
                        </div>
                    }.into_any();
                }
                if !error.get().is_empty() {
                    return view! {
                        <div style="padding: 20px; color: #ef4444; font-size: 13px;">{error.get()}</div>
                    }.into_any();
                }
                let Some(p) = project.get() else {
                    return view! {
                        <div style="padding: 20px; color: rgba(255,245,240,0.7); font-size: 13px;">"Project not found"</div>
                    }.into_any();
                };
                let budget_pct = if p.budget > 0.0 { (p.spent / p.budget) * 100.0 } else { 0.0 };
                let status_str = p.status.clone();
                let all_agents = agents.get();
                view! {
                    <div>
                        // Edit modal
                        <Show when=move || editing.get()>
                            <div style="position: fixed; inset: 0; background: rgba(0,0,0,0.78); z-index: 1000; display: flex; align-items: center; justify-content: center;">
                                <div style="background: #0d0704; border: 1px solid rgba(255,60,20,0.25); border-radius: 16px; padding: 32px; width: 480px; max-width: 92vw;">
                                    <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 20px;">
                                        <div style="font-family: 'Orbitron', monospace; font-size: 11px; letter-spacing: 5px; color: rgba(255,245,240,0.9);">"EDIT PROJECT"</div>
                                        <button style="background: transparent; border: none; color: rgba(255,245,240,0.7); font-size: 18px; cursor: pointer;" on:click=move |_| editing.set(false)>"\u{00D7}"</button>
                                    </div>
                                    <div style="display: flex; flex-direction: column; gap: 12px;">
                                        <div>
                                            <div style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,245,240,0.7); margin-bottom: 6px;">"NAME"</div>
                                            <input style="width: 100%; padding: 10px 14px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 14px; outline: none; box-sizing: border-box;"
                                                prop:value=move || edit_name.get()
                                                on:input=move |ev| edit_name.set(event_target_value(&ev))
                                            />
                                        </div>
                                        <div>
                                            <div style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,245,240,0.7); margin-bottom: 6px;">"DESCRIPTION"</div>
                                            <input style="width: 100%; padding: 10px 14px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 14px; outline: none; box-sizing: border-box;"
                                                prop:value=move || edit_desc.get()
                                                on:input=move |ev| edit_desc.set(event_target_value(&ev))
                                            />
                                        </div>
                                        <div>
                                            <div style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,245,240,0.7); margin-bottom: 6px;">"STATUS"</div>
                                            <select style="width: 100%; background: #0d0704; border: 1px solid rgba(255,60,20,0.1); border-radius: 8px; padding: 10px 14px; color: rgba(255,245,240,0.9); font-size: 14px; box-sizing: border-box;"
                                                prop:value=move || edit_status.get()
                                                on:change=move |ev| edit_status.set(event_target_value(&ev))
                                            >
                                                <option value="active">"Active"</option>
                                                <option value="paused">"Paused"</option>
                                                <option value="completed">"Completed"</option>
                                                <option value="archived">"Archived"</option>
                                            </select>
                                        </div>
                                    </div>
                                    <div style="display: flex; gap: 10px; margin-top: 20px; justify-content: flex-end;">
                                        <Button on_click=Some(Callback::new(move |_| editing.set(false)))>"Cancel"</Button>
                                        <Button primary=true on_click=Some(Callback::new(move |_| {
                                            // project edit save
                                            let name = edit_name.get_untracked();
                                            let desc = edit_desc.get_untracked();
                                            let status = edit_status.get_untracked();
                                            editing.set(false);
                                            let pid = params.get_untracked().get("id").unwrap_or_default().to_string();
                                            spawn_local(async move {
                                                let req = api::UpdateProjectReq {
                                                    name: Some(name),
                                                    description: Some(desc),
                                                    status: Some(status),
                                                    budget: None,
                                                };
                                                match api::update_project(&pid, &req).await {
                                                    Ok(_) => {
                                                        if let Ok(p) = api::fetch_project(&pid).await { project.set(Some(p)); }
                                                    }
                                                    Err(e) => web_sys::console::warn_1(&format!("Update failed: {}", e).into()),
                                                }
                                            });
                                        }))>"Save"</Button>
                                    </div>
                                </div>
                            </div>
                        </Show>

                        // Assign modal
                        <Show when=move || assigning.get()>
                            <div style="position: fixed; inset: 0; background: rgba(0,0,0,0.78); z-index: 1000; display: flex; align-items: center; justify-content: center;">
                                <div style="background: #0d0704; border: 1px solid rgba(255,60,20,0.25); border-radius: 16px; padding: 32px; width: 480px; max-width: 92vw;">
                                    <div style="font-family: 'Orbitron', monospace; font-size: 11px; letter-spacing: 5px; color: rgba(255,245,240,0.9); margin-bottom: 20px;">"ASSIGN AGENTS"</div>
                                    <div style="display: flex; flex-direction: column; gap: 6px; max-height: 300px; overflow-y: auto;">
                                        {move || {
                                            let all = agents.get();
                                            let sel = selected_agents.get();
                                            all.into_iter().map(|a| {
                                                let aid = a.id.clone();
                                                let aid2 = aid.clone();
                                                let is_sel = sel.contains(&aid);
                                                view! {
                                                    <div
                                                        on:click=move |_| {
                                                            let mut cur = selected_agents.get_untracked();
                                                            if cur.contains(&aid2) { cur.retain(|x| x != &aid2); }
                                                            else { cur.push(aid2.clone()); }
                                                            selected_agents.set(cur);
                                                        }
                                                        style={format!("display: flex; align-items: center; gap: 10px; padding: 10px 14px; border-radius: 8px; cursor: pointer; border: 1px solid {}; background: {};",
                                                            if is_sel { "rgba(255,60,20,0.3)" } else { "rgba(255,60,20,0.08)" },
                                                            if is_sel { "rgba(255,60,20,0.06)" } else { "transparent" },
                                                        )}
                                                    >
                                                        <div style={format!("width: 18px; height: 18px; border-radius: 4px; border: 1.5px solid {}; display: flex; align-items: center; justify-content: center;",
                                                            if is_sel { "rgba(255,60,20,0.6)" } else { "rgba(255,60,20,0.15)" }
                                                        )}>
                                                            {is_sel.then(|| view! { <div style="width: 8px; height: 8px; border-radius: 2px; background: #ff3c14;" /> })}
                                                        </div>
                                                        <StatusDot status=a.status.clone() />
                                                        <div style="flex: 1;">
                                                            <div style="font-size: 13px; color: rgba(255,245,240,0.9);">{a.name.clone()}</div>
                                                            <div style="font-size: 10px; color: rgba(255,245,240,0.35);">{a.role.clone()}</div>
                                                        </div>
                                                    </div>
                                                }
                                            }).collect::<Vec<_>>()
                                        }}
                                    </div>
                                    <div style="display: flex; gap: 10px; margin-top: 20px; justify-content: flex-end;">
                                        <Button on_click=Some(Callback::new(move |_| assigning.set(false)))>"Cancel"</Button>
                                        <Button primary=true on_click=Some(Callback::new(move |_| {
                                            let agent_ids = selected_agents.get_untracked();
                                            let pid = params.get_untracked().get("id").unwrap_or_default().to_string();
                                            assigning.set(false);
                                            spawn_local(async move {
                                                let req = api::AssignAgentsReq { agents: agent_ids };
                                                let _ = api::assign_project_agents(&pid, &req).await;
                                                if let Ok(p) = api::fetch_project(&pid).await { project.set(Some(p)); }
                                            });
                                        }))>"Save"</Button>
                                    </div>
                                </div>
                            </div>
                        </Show>

                        // Header
                        <div style="display: flex; justify-content: space-between; align-items: flex-start; margin-bottom: 24px;">
                            <div>
                                <h1 style="font-family: 'Orbitron', monospace; font-size: 14px; letter-spacing: 6px; color: rgba(255,245,240,0.9); margin: 0;">{p.name.clone()}</h1>
                                <p style="font-size: 12px; color: rgba(255,245,240,0.7); margin: 4px 0 0;">{p.description.clone()}</p>
                            </div>
                            <div style="display: flex; gap: 8px; align-items: center;">
                                <StatusDot status=status_str />
                                <Button primary=true on_click=Some(Callback::new(move |_| {
                                    if let Some(proj) = project.get_untracked() {
                                        edit_name.set(proj.name.clone());
                                        edit_desc.set(proj.description.clone());
                                        edit_status.set(proj.status.clone());
                                        editing.set(true);
                                    }
                                }))>"Edit Project"</Button>
                            </div>
                        </div>

                        // Metrics
                        <div style="display: grid; grid-template-columns: repeat(4, 1fr); gap: 12px; margin-bottom: 24px;">
                            <div style="padding: 16px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 10px;">
                                <div style="font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 2px; color: rgba(255,245,240,0.7); margin-bottom: 6px;">"STATUS"</div>
                                <div style="font-size: 14px; color: rgba(255,245,240,0.9); font-weight: 600;">{p.status.clone()}</div>
                            </div>
                            <div style="padding: 16px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 10px;">
                                <div style="font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 2px; color: rgba(255,245,240,0.7); margin-bottom: 6px;">"AGENTS"</div>
                                <div style="font-size: 14px; color: rgba(255,245,240,0.9); font-weight: 600;">{p.agents.len()}</div>
                            </div>
                            <div style="padding: 16px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 10px;">
                                <div style="font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 2px; color: rgba(255,245,240,0.7); margin-bottom: 6px;">"MISSIONS"</div>
                                <div style="font-size: 14px; color: rgba(255,245,240,0.9); font-weight: 600;">{p.mission_count}</div>
                            </div>
                            <div style="padding: 16px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 10px;">
                                <div style="font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 2px; color: rgba(255,245,240,0.7); margin-bottom: 6px;">"BUDGET USED"</div>
                                <div style="font-size: 14px; color: rgba(255,245,240,0.9); font-weight: 600;">{format!("{:.0}%", budget_pct)}</div>
                            </div>
                        </div>

                        // Budget progress
                        {(p.budget > 0.0).then(|| view! {
                            <Card>
                                <SectionTitle>"Budget"</SectionTitle>
                                <div style="display: flex; justify-content: space-between; font-size: 12px; color: rgba(255,245,240,0.7); margin-bottom: 8px;">
                                    <span>{format!("${:.2} spent", p.spent)}</span>
                                    <span>{format!("${:.2} total budget", p.budget)}</span>
                                </div>
                                <ProgressBar value=budget_pct color={if budget_pct > 80.0 { "#ef4444".to_string() } else { "#ff3c14".to_string() }} />
                            </Card>
                        })}

                        // Assigned agents
                        <Card>
                            <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 12px;">
                                <SectionTitle>"Assigned Agents"</SectionTitle>
                                <Button small=true on_click=Some(Callback::new(move |_| {
                                    if let Some(proj) = project.get_untracked() {
                                        selected_agents.set(proj.agents.clone());
                                    }
                                    assigning.set(true);
                                }))>"Assign Agent"</Button>
                            </div>
                            {if p.agents.is_empty() {
                                view! {
                                    <div style="padding: 16px; text-align: center; color: rgba(255,245,240,0.7); font-size: 13px;">
                                        "No agents assigned"
                                    </div>
                                }.into_any()
                            } else {
                                let assigned_ids = p.agents.clone();
                                let matched: Vec<_> = all_agents.into_iter()
                                    .filter(|a| assigned_ids.contains(&a.id))
                                    .collect();
                                view! {
                                    <div style="display: flex; flex-direction: column; gap: 8px;">
                                        {matched.into_iter().map(|a| {
                                            let status_str = a.status.clone();
                                            view! {
                                                <div style="display: flex; align-items: center; gap: 12px; padding: 10px 0; border-bottom: 1px solid rgba(255,60,20,0.1);">
                                                    <StatusDot status=status_str />
                                                    <div style="flex: 1;">
                                                        <div style="font-size: 13px; color: rgba(255,245,240,0.9); font-weight: 500;">{a.name.clone()}</div>
                                                        <div style="font-size: 11px; color: rgba(255,245,240,0.7);">{a.role.clone()}</div>
                                                    </div>
                                                    <Badge text=a.model.clone() />
                                                </div>
                                            }
                                        }).collect::<Vec<_>>()}
                                    </div>
                                }.into_any()
                            }}
                        </Card>
                    </div>
                }.into_any()
            }}
        </div>
    }
}
