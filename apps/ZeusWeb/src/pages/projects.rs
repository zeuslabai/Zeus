// ═══════════════════════════════════════════════════════════
// ZEUS — Projects Page — Phase 2: Wired to API
// ═══════════════════════════════════════════════════════════

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;
use crate::api;
use crate::components::design::*;

#[component]
pub fn ProjectsPage() -> impl IntoView {
    let projects = RwSignal::new(Vec::<api::Project>::new());
    let show_create = RwSignal::new(false);
    let new_name = RwSignal::new(String::new());
    let new_desc = RwSignal::new(String::new());
    let new_budget = RwSignal::new(String::new());

    {
        let projects = projects;
        spawn_local(async move { if let Ok(p) = api::fetch_projects().await { projects.set(p.projects); } });
    }

    let reload_projects = move || {
        spawn_local(async move {
            if let Ok(p) = api::fetch_projects().await { projects.set(p.projects); }
        });
    };

    view! {
        // Create Project modal
        <Show when=move || show_create.get()>
            <div style="position: fixed; inset: 0; background: rgba(0,0,0,0.78); z-index: 1000; display: flex; align-items: center; justify-content: center;">
                <div style="background: #0d0704; border: 1px solid rgba(255,60,20,0.25); border-radius: 16px; padding: 32px; width: 480px; max-width: 92vw;">
                    <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 20px;">
                        <div style="font-family: 'Orbitron', monospace; font-size: 11px; letter-spacing: 5px; color: rgba(255,245,240,0.9);">"NEW PROJECT"</div>
                        <button style="background: transparent; border: none; color: rgba(255,245,240,0.7); font-size: 18px; cursor: pointer;" on:click=move |_| show_create.set(false)>"\u{00D7}"</button>
                    </div>
                    <div style="margin-bottom: 12px;">
                        <label style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 3px; color: rgba(255,245,240,0.7); display: block; margin-bottom: 6px;">"NAME"</label>
                        <input style="width: 100%; padding: 10px 14px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 14px; outline: none; box-sizing: border-box;"
                            prop:value=move || new_name.get()
                            on:input=move |ev| new_name.set(event_target_value(&ev))
                            placeholder="Project name"
                        />
                    </div>
                    <div style="margin-bottom: 12px;">
                        <label style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 3px; color: rgba(255,245,240,0.7); display: block; margin-bottom: 6px;">"DESCRIPTION"</label>
                        <input style="width: 100%; padding: 10px 14px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 14px; outline: none; box-sizing: border-box;"
                            prop:value=move || new_desc.get()
                            on:input=move |ev| new_desc.set(event_target_value(&ev))
                            placeholder="Description"
                        />
                    </div>
                    <div style="margin-bottom: 16px;">
                        <label style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 3px; color: rgba(255,245,240,0.7); display: block; margin-bottom: 6px;">"BUDGET (USD)"</label>
                        <input style="width: 100%; padding: 10px 14px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 14px; outline: none; box-sizing: border-box;"
                            type="number"
                            prop:value=move || new_budget.get()
                            on:input=move |ev| new_budget.set(event_target_value(&ev))
                            placeholder="0.00"
                        />
                    </div>
                    <Button primary=true on_click=Some(Callback::new(move |_| {
                        let name = new_name.get_untracked();
                        if name.trim().is_empty() { return; }
                        let desc = new_desc.get_untracked();
                        let budget: f64 = new_budget.get_untracked().parse().unwrap_or(0.0);
                        show_create.set(false);
                        spawn_local(async move {
                            let req = api::CreateProjectReq { name, description: Some(desc), budget };
                            match api::create_project(&req).await {
                                Ok(_) => { reload_projects(); new_name.set(String::new()); new_desc.set(String::new()); new_budget.set(String::new()); }
                                Err(e) => web_sys::console::warn_1(&format!("Create project failed: {}", e).into()),
                            }
                        });
                    }))>"Create Project"</Button>
                </div>
            </div>
        </Show>

        <div style="padding: 32px;">
            <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 24px;">
                <div style="margin-bottom: 0;">
                    <h1 style="font-family: 'Orbitron', monospace; font-size: 14px; letter-spacing: 6px; color: rgba(255,245,240,0.9); margin: 0;">"PROJECTS"</h1>
                    <p style="font-size: 12px; color: rgba(255,245,240,0.7); margin: 4px 0 0;">{move || {
                        let p = projects.get();
                        if p.is_empty() { "Loading projects...".to_string() }
                        else { format!("{} projects", p.len()) }
                    }}</p>
                </div>
                <Button primary=true on_click=Some(Callback::new(move |_| show_create.set(true)))>
                    <Icon name="plus" size=12 /> " New Project"
                </Button>
            </div>
            <div style="display: grid; grid-template-columns: repeat(auto-fill, minmax(320px, 1fr)); gap: 16px;">
                {move || projects.get().into_iter().map(|p| {
                    let is_active = p.status == "active";
                    let status_str = p.status.clone();
                    let budget_pct = if p.budget > 0.0 { (p.spent / p.budget) * 100.0 } else { 0.0 };
                    view! {
                        <Card glow=is_active>
                            <div style="display: flex; justify-content: space-between; align-items: flex-start; margin-bottom: 10px;">
                                <div>
                                    <div style="font-family: 'Orbitron', monospace; font-size: 14px; color: rgba(255,245,240,0.9); font-weight: 700;">{p.name.clone()}</div>
                                    <div style="font-size: 12px; color: rgba(255,245,240,0.7); margin-top: 2px;">{p.description.clone()}</div>
                                </div>
                                <StatusDot status=status_str />
                            </div>
                            <div style="display: grid; grid-template-columns: 1fr 1fr; gap: 8px; margin-bottom: 12px;">
                                <div style="padding: 6px 8px; background: rgba(255,255,255,0.03); border-radius: 6px;">
                                    <div style="font-family: 'Orbitron', monospace; font-size: 8px; color: rgba(255,245,240,0.5); letter-spacing: 1px;">"AGENTS"</div>
                                    <div style="font-size: 12px; color: rgba(255,245,240,0.9);">{p.agents.len()}</div>
                                </div>
                                <div style="padding: 6px 8px; background: rgba(255,255,255,0.03); border-radius: 6px;">
                                    <div style="font-family: 'Orbitron', monospace; font-size: 8px; color: rgba(255,245,240,0.5); letter-spacing: 1px;">"MISSIONS"</div>
                                    <div style="font-size: 12px; color: rgba(255,245,240,0.9);">{p.mission_count}</div>
                                </div>
                            </div>
                            {(p.budget > 0.0).then(|| view! {
                                <div style="margin-bottom: 8px;">
                                    <div style="display: flex; justify-content: space-between; font-size: 10px; color: rgba(255,245,240,0.7); margin-bottom: 4px;">
                                        <span>{format!("${:.2} spent", p.spent)}</span>
                                        <span>{format!("${:.2} budget", p.budget)}</span>
                                    </div>
                                    <ProgressBar value=budget_pct color={if budget_pct > 80.0 { "#ef4444".to_string() } else { "#ff3c14".to_string() }} />
                                </div>
                            })}
                            <div style="display: flex; justify-content: flex-end; gap: 8px; padding-top: 8px; border-top: 1px solid rgba(255,60,20,0.1);">
                                <button style="font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 1px; padding: 4px 10px; border-radius: 5px; cursor: pointer; background: rgba(239,68,68,0.08); border: 1px solid rgba(239,68,68,0.15); color: rgba(239,68,68,0.6);"
                                    on:click={
                                        let pid = p.id.clone();
                                        move |_| {
                                            let pid = pid.clone();
                                            spawn_local(async move {
                                                let _ = api::delete_project(&pid).await;
                                                reload_projects();
                                            });
                                        }
                                    }
                                >"DELETE"</button>
                                <a href={format!("/projects/{}", p.id)} style="text-decoration: none;">
                                    <Button small=true primary=is_active>"View"</Button>
                                </a>
                            </div>
                        </Card>
                    }
                }).collect::<Vec<_>>()}
            </div>
        </div>
    }
}
