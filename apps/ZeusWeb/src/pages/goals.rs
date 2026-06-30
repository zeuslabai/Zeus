// ═══════════════════════════════════════════════════════════
// ZEUS — Goals Page — Phase 2: Wired to API
// ═══════════════════════════════════════════════════════════

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;
use crate::api;
use crate::components::design::*;

#[component]
pub fn GoalsPage() -> impl IntoView {
    let goals = RwSignal::new(Vec::<api::GoalResponse>::new());
    let total = RwSignal::new(0usize);
    let show_create = RwSignal::new(false);
    let new_desc = RwSignal::new(String::new());
    let new_priority = RwSignal::new("medium".to_string());

    {
        let goals = goals;
        let total = total;
        spawn_local(async move {
            if let Ok(g) = api::fetch_goals().await {
                total.set(g.total);
                goals.set(g.goals);
            }
        });
    }

    let reload_goals = move || {
        spawn_local(async move {
            if let Ok(g) = api::fetch_goals().await {
                total.set(g.total);
                goals.set(g.goals);
            }
        });
    };

    view! {
        // Create Goal modal
        <Show when=move || show_create.get()>
            <div style="position: fixed; inset: 0; background: rgba(0,0,0,0.78); z-index: 1000; display: flex; align-items: center; justify-content: center;">
                <div style="background: #0d0704; border: 1px solid rgba(255,60,20,0.25); border-radius: 16px; padding: 32px; width: 480px; max-width: 92vw;">
                    <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 20px;">
                        <div style="font-family: 'Orbitron', monospace; font-size: 11px; letter-spacing: 5px; color: rgba(255,245,240,0.9);">"NEW GOAL"</div>
                        <button style="background: transparent; border: none; color: rgba(255,245,240,0.7); font-size: 18px; cursor: pointer;" on:click=move |_| show_create.set(false)>"\u{00D7}"</button>
                    </div>
                    <div style="margin-bottom: 12px;">
                        <label style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 3px; color: rgba(255,245,240,0.7); display: block; margin-bottom: 6px;">"DESCRIPTION"</label>
                        <textarea style="width: 100%; padding: 12px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 8px; color: rgba(255,245,240,0.9); font-family: 'Rajdhani', sans-serif; font-size: 14px; min-height: 80px; resize: vertical; outline: none; box-sizing: border-box;"
                            prop:value=move || new_desc.get()
                            on:input=move |ev| new_desc.set(event_target_value(&ev))
                            placeholder="Describe the goal..."
                        />
                    </div>
                    <div style="margin-bottom: 16px;">
                        <label style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 3px; color: rgba(255,245,240,0.7); display: block; margin-bottom: 6px;">"PRIORITY"</label>
                        <div style="display: flex; gap: 8px;">
                            {["low", "medium", "high", "critical"].iter().map(|p| {
                                let p_str = p.to_string();
                                let p_str2 = p_str.clone();
                                let p_str3 = p_str.clone();
                                view! {
                                    <button style=move || format!("padding: 6px 14px; border-radius: 6px; font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; cursor: pointer; border: 1px solid {}; background: {}; color: {};",
                                        if new_priority.get() == p_str { "rgba(255,60,20,0.4)" } else { "rgba(255,60,20,0.1)" },
                                        if new_priority.get() == p_str { "rgba(255,60,20,0.15)" } else { "transparent" },
                                        if new_priority.get() == p_str { "#ff3c14" } else { "rgba(255,245,240,0.7)" },
                                    )
                                        on:click=move |_| new_priority.set(p_str2.clone())
                                    >{p_str3.to_uppercase()}</button>
                                }
                            }).collect::<Vec<_>>()}
                        </div>
                    </div>
                    <Button primary=true on_click=Some(Callback::new(move |_| {
                        let desc = new_desc.get_untracked();
                        let priority = new_priority.get_untracked();
                        if desc.trim().is_empty() { return; }
                        show_create.set(false);
                        spawn_local(async move {
                            let req = api::CreateGoalRequest { description: desc, priority, source: "web".to_string() };
                            match api::create_goal(&req).await {
                                Ok(_) => { reload_goals(); new_desc.set(String::new()); }
                                Err(e) => web_sys::console::warn_1(&format!("Create goal failed: {}", e).into()),
                            }
                        });
                    }))>"Create Goal"</Button>
                </div>
            </div>
        </Show>

        <div style="padding: 32px;">
            <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 24px;">
                <div style="margin-bottom: 0;">
                    <h1 style="font-family: 'Orbitron', monospace; font-size: 14px; letter-spacing: 6px; color: rgba(255,245,240,0.9); margin: 0;">"GOALS"</h1>
                    <p>{move || {
                        let g = goals.get();
                        let active = g.iter().filter(|gl| gl.status == "active" || gl.status == "in_progress").count();
                        if g.is_empty() { "Loading goals...".to_string() }
                        else { format!("{} goals • {} active", total.get(), active) }
                    }}</p>
                </div>
                <Button primary=true on_click=Some(Callback::new(move |_| show_create.set(true)))>
                    <Icon name="plus" size=12 /> " New Goal"
                </Button>
            </div>
            <div style="display: flex; flex-direction: column; gap: 12px;">
                {move || goals.get().into_iter().map(|g| {
                    let priority_color = match g.priority.as_str() {
                        "high" | "critical" => "rgba(239,68,68,1)",
                        "medium" => "rgba(234,179,8,1)",
                        _ => "rgba(34,197,94,1)",
                    };
                    let status_str = g.status.clone();
                    view! {
                        <Card style="cursor: pointer;">
                            <div style="display: flex; align-items: flex-start; gap: 12px;">
                                <div style={format!("width: 4px; height: 40px; border-radius: 2px; background: {}; flex-shrink: 0; margin-top: 2px;", priority_color)} />
                                <div style="flex: 1;">
                                    <div style="font-size: 14px; color: rgba(255,245,240,0.9); font-weight: 500; margin-bottom: 4px;">{g.description.clone()}</div>
                                    <div style="display: flex; gap: 8px; align-items: center;">
                                        <Badge text=status_str />
                                        {(!g.priority.is_empty()).then(|| view! {
                                            <Badge text={g.priority.clone()} color={priority_color.to_string()} />
                                        })}
                                        {(!g.source.is_empty()).then(|| view! {
                                            <span style="font-size: 10px; color: rgba(255,245,240,0.5);">"via "{g.source.clone()}</span>
                                        })}
                                    </div>
                                </div>
                                <span style="font-size: 10px; color: rgba(255,245,240,0.5); white-space: nowrap;">{g.created_at.clone()}</span>
                            </div>
                            <div style="display: flex; gap: 6px; margin-top: 10px; padding-top: 10px; border-top: 1px solid rgba(255,60,20,0.06);">
                                {(g.status != "completed").then(|| {
                                    let gid = g.id.clone();
                                    view! {
                                        <button style="font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 2px; padding: 4px 10px; border-radius: 5px; cursor: pointer; background: rgba(34,197,94,0.12); border: 1px solid rgba(34,197,94,0.25); color: #22c55e;"
                                            on:click=move |_| {
                                                let gid = gid.clone();
                                                spawn_local(async move {
                                                    if let Err(e) = api::update_goal_status(&gid, "completed").await { web_sys::console::error_1(&format!("API error: {}", e).into()); }
                                                    reload_goals();
                                                });
                                            }
                                        >"COMPLETE"</button>
                                    }
                                })}
                                {(g.status == "active").then(|| {
                                    let gid = g.id.clone();
                                    view! {
                                        <button style="font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 2px; padding: 4px 10px; border-radius: 5px; cursor: pointer; background: rgba(234,179,8,0.12); border: 1px solid rgba(234,179,8,0.25); color: #eab308;"
                                            on:click=move |_| {
                                                let gid = gid.clone();
                                                spawn_local(async move {
                                                    if let Err(e) = api::update_goal_status(&gid, "in_progress").await { web_sys::console::error_1(&format!("API error: {}", e).into()); }
                                                    reload_goals();
                                                });
                                            }
                                        >"START"</button>
                                    }
                                })}
                            </div>
                        </Card>
                    }
                }).collect::<Vec<_>>()}
            </div>
        </div>
    }
}
