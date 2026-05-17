// ═══════════════════════════════════════════════════════════
// ZEUS — Teams Page — Multi-Agent Team Coordination
// ═══════════════════════════════════════════════════════════

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;
use crate::api;
use crate::components::design::*;

#[component]
pub fn TeamsPage() -> impl IntoView {
    let teams = RwSignal::new(Vec::<api::Team>::new());
    let loading = RwSignal::new(true);
    let show_create = RwSignal::new(false);
    let new_name = RwSignal::new(String::new());
    let new_desc = RwSignal::new(String::new());
    let new_strategy = RwSignal::new("round_robin".to_string());
    let message = RwSignal::new(String::new());
    let show_recommend = RwSignal::new(false);
    let rec_goal = RwSignal::new(String::new());
    let rec_result = RwSignal::new(Option::<api::TeamRecommendation>::None);
    let rec_loading = RwSignal::new(false);

    // Fetch teams on mount
    {
        spawn_local(async move {
            if let Ok(t) = api::fetch_teams().await {
                teams.set(t.teams);
            }
            loading.set(false);
        });
    }

    let create_team = move |_| {
        let name = new_name.get();
        let desc = new_desc.get();
        let strategy = new_strategy.get();
        if name.trim().is_empty() { return; }
        spawn_local(async move {
            match api::create_team(&name, &desc, &strategy).await {
                Ok(_) => {
                    message.set("Team created".to_string());
                    show_create.set(false);
                    new_name.set(String::new());
                    new_desc.set(String::new());
                    // Refresh
                    if let Ok(t) = api::fetch_teams().await {
                        teams.set(t.teams);
                    }
                }
                Err(e) => message.set(format!("Error: {}", e)),
            }
        });
    };

    view! {
        <div style="padding: 32px;">
            <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 24px;">
                <div>
                    <h1 style="font-family: 'Orbitron', monospace; font-size: 14px; letter-spacing: 6px; color: rgba(255,245,240,0.9); margin: 0;">"TEAMS"</h1>
                    <p style="color: rgba(255,245,240,0.7); font-size: 12px;">
                        {move || {
                            if loading.get() { "Loading teams...".to_string() }
                            else {
                                let t = teams.get();
                                let active = t.iter().filter(|tm| tm.status == "active").count();
                                format!("{} teams • {} active", t.len(), active)
                            }
                        }}
                    </p>
                </div>
                <div style="display: flex; gap: 8px; align-items: center;">
                    {move || {
                        let msg = message.get();
                        (!msg.is_empty()).then(|| view! {
                            <Badge text=msg color="#22c55e".to_string() />
                        })
                    }}
                    <Button on_click=Some(Callback::new(move |_| show_recommend.set(true)))>"Recommend"</Button>
                    <Button primary=true on_click=Some(Callback::new(move |_| show_create.set(!show_create.get())))>
                        <Icon name="plus" size=12 /> " New Team"
                    </Button>
                </div>
            </div>

            // Create team form
            <Show when=move || show_create.get()>
                <Card style="margin-bottom: 16px;">
                    <div style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 3px; color: rgba(255,245,240,0.7); margin-bottom: 12px;">"CREATE TEAM"</div>
                    <div style="display: flex; flex-direction: column; gap: 10px;">
                        <input
                            type="text"
                            placeholder="Team name"
                            prop:value=move || new_name.get()
                            on:input=move |ev| new_name.set(event_target_value(&ev))
                            style="background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 6px; padding: 8px 12px; color: rgba(255,245,240,0.9); font-size: 12px;"
                        />
                        <input
                            type="text"
                            placeholder="Description"
                            prop:value=move || new_desc.get()
                            on:input=move |ev| new_desc.set(event_target_value(&ev))
                            style="background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 6px; padding: 8px 12px; color: rgba(255,245,240,0.9); font-size: 12px;"
                        />
                        <div style="display: flex; gap: 8px; align-items: center;">
                            <span style="font-size: 10px; color: rgba(255,245,240,0.5);">"Strategy:"</span>
                            {["round_robin", "priority", "load_balanced", "broadcast"].into_iter().map(|s| {
                                let strategy = s.to_string();
                                let s_clone = s.to_string();
                                view! {
                                    <button
                                        on:click=move |_| new_strategy.set(s_clone.clone())
                                        style={move || format!(
                                            "padding: 4px 10px; border-radius: 4px; font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 1px; cursor: pointer; border: 1px solid {}; background: {}; color: {};",
                                            if new_strategy.get() == strategy { "rgba(255,60,20,0.4)" } else { "rgba(255,60,20,0.1)" },
                                            if new_strategy.get() == strategy { "rgba(255,60,20,0.1)" } else { "transparent" },
                                            if new_strategy.get() == strategy { "rgba(255,245,240,0.9)" } else { "rgba(255,245,240,0.6)" },
                                        )}
                                    >{s.to_uppercase()}</button>
                                }
                            }).collect::<Vec<_>>()}
                        </div>
                        <div style="display: flex; gap: 8px; justify-content: flex-end;">
                            <Button on_click=Some(Callback::new(move |_| show_create.set(false)))>"Cancel"</Button>
                            <Button primary=true on_click=Some(Callback::new(create_team))>"Create"</Button>
                        </div>
                    </div>
                </Card>
            </Show>

            // Recommend modal
            <Show when=move || show_recommend.get()>
                <Card style="margin-bottom: 16px;">
                    <div style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 3px; color: rgba(255,245,240,0.7); margin-bottom: 12px;">"RECOMMEND TEAM FOR GOAL"</div>
                    <div style="display: flex; gap: 8px;">
                        <input
                            type="text" placeholder="Describe the goal or task..."
                            prop:value=move || rec_goal.get()
                            on:input=move |ev| rec_goal.set(event_target_value(&ev))
                            style="flex: 1; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 6px; padding: 8px 12px; color: rgba(255,245,240,0.9); font-size: 12px;"
                        />
                        <Button primary=true on_click=Some(Callback::new(move |_| {
                            let goal = rec_goal.get_untracked();
                            if goal.trim().is_empty() { return; }
                            rec_loading.set(true);
                            rec_result.set(None);
                            spawn_local(async move {
                                match api::recommend_team(&goal).await {
                                    Ok(r) => rec_result.set(Some(r)),
                                    Err(e) => message.set(format!("Error: {}", e)),
                                }
                                rec_loading.set(false);
                            });
                        }))>{move || if rec_loading.get() { "Analyzing..." } else { "Analyze" }}</Button>
                        <Button on_click=Some(Callback::new(move |_| { show_recommend.set(false); rec_result.set(None); }))>"Close"</Button>
                    </div>
                    {move || rec_result.get().map(|r| view! {
                        <div style="margin-top: 12px; padding: 12px; background: rgba(255,255,255,0.02); border-radius: 8px; border: 1px solid rgba(255,60,20,0.08);">
                            <div style="font-size: 13px; color: rgba(255,245,240,0.9); font-weight: 600; margin-bottom: 6px;">{r.team_name.clone()}</div>
                            <div style="font-size: 12px; color: rgba(255,245,240,0.7); margin-bottom: 8px;">{r.rationale.clone()}</div>
                            <div style="display: flex; gap: 6px; flex-wrap: wrap;">
                                <Badge text={r.estimated_complexity.clone()} color="#3b82f6".to_string() />
                                <Badge text={format!("{} steps", r.estimated_steps)} color="#eab308".to_string() />
                                <Badge text={format!("{} coordinators", r.coordinators.len())} color="rgba(255,60,20,0.6)".to_string() />
                                <Badge text={format!("{} workers", r.workers.len())} color="#22c55e".to_string() />
                            </div>
                        </div>
                    })}
                </Card>
            </Show>

            // Team list
            <div style="display: grid; grid-template-columns: repeat(auto-fill, minmax(320px, 1fr)); gap: 12px;">
                {move || teams.get().into_iter().map(|t| {
                    let status_color = match t.status.as_str() {
                        "active" => "#22c55e",
                        "idle" => "#eab308",
                        _ => "rgba(255,245,240,0.5)",
                    };
                    view! {
                        <Card>
                            <div style="display: flex; align-items: flex-start; gap: 12px;">
                                <div style="width: 40px; height: 40px; border-radius: 8px; background: rgba(255,60,20,0.15); display: flex; align-items: center; justify-content: center; flex-shrink: 0;">
                                    <Icon name="users" size=20 />
                                </div>
                                <div style="flex: 1; min-width: 0;">
                                    <div style="display: flex; align-items: center; gap: 8px; margin-bottom: 4px;">
                                        <span style="font-size: 14px; font-weight: 500; color: rgba(255,245,240,0.9);">{t.name.clone()}</span>
                                        <StatusDot status=t.status.clone() />
                                    </div>
                                    {(!t.description.is_empty()).then(|| view! {
                                        <div style="font-size: 11px; color: rgba(255,245,240,0.6); margin-bottom: 8px;">{t.description.clone()}</div>
                                    })}
                                    <div style="display: flex; gap: 8px; flex-wrap: wrap;">
                                        <Badge text={t.routing_strategy.clone()} color="#3b82f6".to_string() />
                                        <Badge text={format!("{} agents", t.agents.len())} color=status_color.to_string() />
                                        {(!t.supervisor_id.is_empty()).then(|| view! {
                                            <Badge text={format!("sup: {}", &t.supervisor_id[..8.min(t.supervisor_id.len())])} color="rgba(255,60,20,0.6)".to_string() />
                                        })}
                                    </div>
                                    {(!t.agents.is_empty()).then(|| view! {
                                        <div style="margin-top: 8px; display: flex; gap: 4px; flex-wrap: wrap;">
                                            {t.agents.iter().map(|a| {
                                                view! {
                                                    <span style="font-size: 9px; padding: 2px 6px; background: rgba(255,255,255,0.03); border-radius: 3px; color: rgba(255,245,240,0.5); font-family: 'Orbitron', monospace;">
                                                        {a[..8.min(a.len())].to_string()}
                                                    </span>
                                                }
                                            }).collect::<Vec<_>>()}
                                        </div>
                                    })}
                                    <div style="display: flex; justify-content: flex-end; margin-top: 10px; padding-top: 8px; border-top: 1px solid rgba(255,60,20,0.1);">
                                        <button
                                            style="font-family: 'Orbitron', monospace; font-size: 7px; letter-spacing: 1px; padding: 4px 10px; border-radius: 5px; cursor: pointer; background: rgba(239,68,68,0.08); border: 1px solid rgba(239,68,68,0.15); color: rgba(239,68,68,0.6);"
                                            on:click={
                                                let tid = t.id.clone();
                                                move |_| {
                                                    let tid = tid.clone();
                                                    spawn_local(async move {
                                                        let _ = api::delete_team(&tid).await;
                                                        if let Ok(tr) = api::fetch_teams().await { teams.set(tr.teams); }
                                                    });
                                                }
                                            }
                                        >"DELETE"</button>
                                    </div>
                                </div>
                            </div>
                        </Card>
                    }
                }).collect::<Vec<_>>()}
            </div>

            <Show when=move || !loading.get() && teams.get().is_empty()>
                <Card>
                    <div style="text-align: center; padding: 32px;">
                        <div style="font-family: 'Orbitron', monospace; font-size: 11px; letter-spacing: 3px; color: rgba(255,245,240,0.5); margin-bottom: 8px;">"NO TEAMS"</div>
                        <div style="font-size: 12px; color: rgba(255,245,240,0.5);">"Create a team to coordinate multiple agents on parallel tasks"</div>
                    </div>
                </Card>
            </Show>
        </div>
    }
}
