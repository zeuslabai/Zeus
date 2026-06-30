// ═══════════════════════════════════════════════════════════
// ZEUS — Outcome Templates Page — S12-7
// ═══════════════════════════════════════════════════════════

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;
use crate::api;
use crate::components::design::{Card, Button};

#[component]
pub fn TemplatesPage() -> impl IntoView {
    let templates   = RwSignal::new(Vec::<api::OutcomeTemplate>::new());
    let categories  = RwSignal::new(Vec::<String>::new());
    let sel_cat     = RwSignal::new(String::new());
    let search_q    = RwSignal::new(String::new());
    let loading     = RwSignal::new(true);

    // Apply modal state
    let applying       = RwSignal::new(Option::<api::OutcomeTemplate>::None);
    let goal_input     = RwSignal::new(String::new());
    let apply_result   = RwSignal::new(Option::<api::AppliedTemplate>::None);
    let apply_error    = RwSignal::new(String::new());
    let apply_loading  = RwSignal::new(false);

    // Load categories + templates on mount
    {
        let templates = templates; let categories = categories; let loading = loading;
        spawn_local(async move {
            if let Ok(cats) = api::fetch_template_categories().await {
                categories.set(cats);
            }
            if let Ok(r) = api::fetch_templates(None, None).await {
                templates.set(r.templates);
            }
            loading.set(false);
        });
    }

    // Re-fetch when filter changes
    let do_search = move || {
        let q = search_q.get_untracked();
        let cat = sel_cat.get_untracked();
        loading.set(true);
        spawn_local(async move {
            let result = if !q.is_empty() {
                api::search_templates(&q).await
            } else {
                api::fetch_templates(if cat.is_empty() { None } else { Some(&cat) }, None).await
                    .map(|r| r.templates)
            };
            if let Ok(list) = result {
                templates.set(list);
            }
            loading.set(false);
        });
    };

    let category_color = |cat: &str| match cat {
        "writing"      => "#d4a574",
        "coding"       => "#74d4a5",
        "research"     => "#74a5d4",
        "deployment"   => "#d474a5",
        "testing"      => "#a5d474",
        "image"        => "#d4d474",
        _              => "#a574d4",
    };

    view! {
        // Apply modal
        <Show when=move || applying.get().is_some()>
            <div style="position: fixed; inset: 0; background: rgba(0,0,0,0.82); z-index: 1000; display: flex; align-items: center; justify-content: center; padding: 16px;">
                <div style="background: #0d0704; border: 1px solid rgba(255,60,20,0.25); border-radius: 16px; padding: 32px; width: 560px; max-width: 96vw; max-height: 90vh; overflow-y: auto;">
                    {move || {
                        let tpl = applying.get();
                        let result = apply_result.get();
                        if let Some(t) = tpl {
                            view! {
                                <div>
                                    <div style="display: flex; justify-content: space-between; align-items: flex-start; margin-bottom: 20px;">
                                        <div>
                                            <div style="font-family: 'Orbitron', monospace; font-size: 11px; letter-spacing: 5px; color: rgba(255,245,240,0.9);">"APPLY TEMPLATE"</div>
                                            <div style="font-size: 14px; color: rgba(255,245,240,0.7); margin-top: 4px;">{t.name.clone()}</div>
                                        </div>
                                        <button style="background: transparent; border: none; color: rgba(255,245,240,0.7); font-size: 20px; cursor: pointer; padding: 0 4px;"
                                            on:click=move |_| { applying.set(None); apply_result.set(None); apply_error.set(String::new()); goal_input.set(String::new()); }
                                        >"×"</button>
                                    </div>

                                    {if result.is_none() {
                                        view! {
                                            <div>
                                                <div style="font-size: 12px; color: rgba(255,245,240,0.6); margin-bottom: 12px; line-height: 1.6;">{t.description.clone()}</div>
                                                <label style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 3px; color: rgba(255,245,240,0.7); display: block; margin-bottom: 8px;">"YOUR GOAL"</label>
                                                <textarea
                                                    placeholder="Describe what you want to achieve..."
                                                    prop:value=move || goal_input.get()
                                                    on:input=move |e| goal_input.set(event_target_value(&e))
                                                    style="width: 100%; box-sizing: border-box; padding: 12px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.15); border-radius: 8px; color: rgba(255,245,240,0.9); font-family: 'Rajdhani', sans-serif; font-size: 14px; min-height: 90px; resize: vertical; outline: none;"
                                                />
                                                {move || {
                                                    let err = apply_error.get();
                                                    if !err.is_empty() {
                                                        view! { <div style="font-size: 12px; color: #ef4444; margin-top: 8px;">{err}</div> }.into_any()
                                                    } else { view! { <div /> }.into_any() }
                                                }}
                                                <div style="display: flex; gap: 10px; margin-top: 18px; justify-content: flex-end;">
                                                    <button
                                                        style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; padding: 10px 22px; border-radius: 8px; cursor: pointer; background: transparent; border: 1px solid rgba(255,60,20,0.2); color: rgba(255,245,240,0.7);"
                                                        on:click=move |_| { applying.set(None); apply_result.set(None); goal_input.set(String::new()); }
                                                    >"CANCEL"</button>
                                                    <button
                                                        style=move || format!("font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; padding: 10px 22px; border-radius: 8px; cursor: {}; background: rgba(255,60,20,0.15); border: 1px solid rgba(255,60,20,0.35); color: rgba(255,60,20,1); opacity: {};",
                                                            if apply_loading.get() { "not-allowed" } else { "pointer" },
                                                            if apply_loading.get() { "0.5" } else { "1" })
                                                        prop:disabled=move || apply_loading.get()
                                                        on:click={
                                                            let tid = t.id.clone();
                                                            move |_| {
                                                                let goal = goal_input.get_untracked();
                                                                if goal.trim().is_empty() { apply_error.set("Enter a goal description.".into()); return; }
                                                                let tid = tid.clone();
                                                                apply_loading.set(true);
                                                                apply_error.set(String::new());
                                                                spawn_local(async move {
                                                                    match api::apply_template(&tid, &goal).await {
                                                                        Ok(r)  => { apply_result.set(Some(r)); }
                                                                        Err(e) => { apply_error.set(e); }
                                                                    }
                                                                    apply_loading.set(false);
                                                                });
                                                            }
                                                        }
                                                    >{move || if apply_loading.get() { "APPLYING..." } else { "APPLY →" }}</button>
                                                </div>
                                            </div>
                                        }.into_any()
                                    } else {
                                        let r = result.unwrap();
                                        view! {
                                            <div>
                                                <div style="margin-bottom: 16px;">
                                                    <div style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 3px; color: rgba(34,197,94,0.9); margin-bottom: 8px;">"TEMPLATE APPLIED"</div>
                                                    {(!r.warnings.is_empty()).then(|| {
                                                        view! {
                                                            <div style="padding: 10px 14px; background: rgba(234,179,8,0.08); border: 1px solid rgba(234,179,8,0.2); border-radius: 8px; margin-bottom: 12px;">
                                                                {r.warnings.iter().map(|w| view! {
                                                                    <div style="font-size: 12px; color: #eab308;">{"⚠ "}{w.clone()}</div>
                                                                }).collect::<Vec<_>>()}
                                                            </div>
                                                        }
                                                    })}
                                                    {(!r.missing_providers.is_empty()).then(|| {
                                                        view! {
                                                            <div style="padding: 10px 14px; background: rgba(239,68,68,0.08); border: 1px solid rgba(239,68,68,0.2); border-radius: 8px; margin-bottom: 12px;">
                                                                <div style="font-size: 11px; color: rgba(239,68,68,0.9); margin-bottom: 4px; font-weight: 600;">"MISSING PROVIDERS"</div>
                                                                {r.missing_providers.iter().map(|p| view! {
                                                                    <div style="font-size: 12px; color: rgba(239,68,68,0.8);">{"• "}{p.clone()}</div>
                                                                }).collect::<Vec<_>>()}
                                                            </div>
                                                        }
                                                    })}
                                                    <div style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 2px; color: rgba(255,245,240,0.5); margin-bottom: 6px;">"ENRICHED PROMPT"</div>
                                                    <div style="padding: 12px 14px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 8px; font-size: 13px; color: rgba(255,245,240,0.85); line-height: 1.6; white-space: pre-wrap; max-height: 200px; overflow-y: auto;">{r.enriched_prompt.clone()}</div>
                                                </div>
                                                <div style="display: flex; justify-content: flex-end; gap: 10px;">
                                                    <button
                                                        style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; padding: 10px 22px; border-radius: 8px; cursor: pointer; background: rgba(34,197,94,0.12); border: 1px solid rgba(34,197,94,0.3); color: #22c55e;"
                                                        on:click=move |_| {
                                                            let prompt = r.enriched_prompt.clone();
                                                            if let Some(win) = web_sys::window() {
                                                                let nav = win.navigator();
                                                                let _ = nav.clipboard().write_text(&prompt);
                                                            }
                                                        }
                                                    >"COPY PROMPT"</button>
                                                    <button
                                                        style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; padding: 10px 22px; border-radius: 8px; cursor: pointer; background: transparent; border: 1px solid rgba(255,60,20,0.2); color: rgba(255,245,240,0.7);"
                                                        on:click=move |_| { applying.set(None); apply_result.set(None); goal_input.set(String::new()); }
                                                    >"CLOSE"</button>
                                                </div>
                                            </div>
                                        }.into_any()
                                    }}
                                </div>
                            }.into_any()
                        } else { view! { <div /> }.into_any() }
                    }}
                </div>
            </div>
        </Show>

        <div style="padding: 32px;">
            // Header
            <div style="display: flex; justify-content: space-between; align-items: flex-start; margin-bottom: 24px;">
                <div>
                    <h1 style="font-family: 'Orbitron', monospace; font-size: 14px; letter-spacing: 6px; color: rgba(255,245,240,0.9); margin: 0 0 6px;">"OUTCOME TEMPLATES"</h1>
                    <p style="font-size: 13px; color: rgba(255,245,240,0.5); margin: 0;">"Describe a goal — Zeus selects the right workflow automatically."</p>
                </div>
            </div>

            // Search + category filter bar
            <div style="display: flex; gap: 10px; margin-bottom: 20px; flex-wrap: wrap; align-items: center;">
                <input
                    type="text"
                    placeholder="Search templates..."
                    prop:value=move || search_q.get()
                    on:input=move |e| {
                        search_q.set(event_target_value(&e));
                        do_search();
                    }
                    style="flex: 1; min-width: 200px; padding: 10px 16px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.15); border-radius: 8px; color: rgba(255,245,240,0.9); font-family: 'Rajdhani', sans-serif; font-size: 14px; outline: none;"
                />
                <div style="display: flex; gap: 6px; flex-wrap: wrap;">
                    <button
                        style=move || format!("font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; padding: 8px 14px; border-radius: 6px; cursor: pointer; border: 1px solid {}; background: {}; color: {};",
                            if sel_cat.get().is_empty() { "rgba(255,60,20,0.4)" } else { "rgba(255,60,20,0.15)" },
                            if sel_cat.get().is_empty() { "rgba(255,60,20,0.12)" } else { "transparent" },
                            if sel_cat.get().is_empty() { "rgba(255,60,20,1)" } else { "rgba(255,245,240,0.6)" })
                        on:click=move |_| { sel_cat.set(String::new()); do_search(); }
                    >"ALL"</button>
                    {move || categories.get().into_iter().map(|cat| {
                        let cat2 = cat.clone();
                        let cat3 = cat.clone();
                        let col = category_color(&cat).to_string();
                        view! {
                            <button
                                style=move || {
                                    let active = sel_cat.get() == cat2;
                                    format!("font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; padding: 8px 14px; border-radius: 6px; cursor: pointer; border: 1px solid {}; background: {}; color: {};",
                                        if active { format!("{}66", col) } else { "rgba(255,255,255,0.08)".to_string() },
                                        if active { format!("{}22", col) } else { "transparent".to_string() },
                                        if active { col.clone() } else { "rgba(255,245,240,0.6)".to_string() })
                                }
                                on:click=move |_| { sel_cat.set(cat3.clone()); do_search(); }
                            >{cat.to_uppercase()}</button>
                        }
                    }).collect::<Vec<_>>()}
                </div>
            </div>

            // Template grid
            {move || {
                if loading.get() {
                    view! {
                        <div style="display: grid; grid-template-columns: repeat(auto-fill, minmax(300px, 1fr)); gap: 14px;">
                            {(0..6usize).map(|_| view! {
                                <div style="height: 140px; border-radius: 12px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.08); animation: pulse 1.5s ease infinite;" />
                            }).collect::<Vec<_>>()}
                        </div>
                    }.into_any()
                } else {
                    let list = templates.get();
                    if list.is_empty() {
                        view! {
                            <div style="text-align: center; padding: 64px 32px; color: rgba(255,245,240,0.4);">
                                <div style="font-size: 36px; margin-bottom: 12px;">"📋"</div>
                                <div style="font-family: 'Orbitron', monospace; font-size: 11px; letter-spacing: 4px;">"NO TEMPLATES FOUND"</div>
                            </div>
                        }.into_any()
                    } else {
                        view! {
                            <div style="display: grid; grid-template-columns: repeat(auto-fill, minmax(300px, 1fr)); gap: 14px;">
                                {list.into_iter().map(|t| {
                                    let t2 = t.clone();
                                    let col = category_color(t.categories.first().map(|s| s.as_str()).unwrap_or("")).to_string();
                                    view! {
                                        <Card style=format!("display: flex; flex-direction: column; gap: 10px; cursor: pointer; border-left: 3px solid {};", col)>
                                            <div style="display: flex; align-items: flex-start; justify-content: space-between; gap: 8px;">
                                                <div style="font-size: 15px; font-weight: 700; color: rgba(255,245,240,0.9); flex: 1;">{t.name.clone()}</div>
                                                <span style=format!("font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 1.5px; padding: 3px 8px; border-radius: 4px; background: {}22; color: {}; flex-shrink: 0;", col, col)>
                                                    {t.categories.first().cloned().unwrap_or_default().to_uppercase()}
                                                </span>
                                            </div>
                                            <div style="font-size: 13px; color: rgba(255,245,240,0.6); line-height: 1.5; flex: 1;">{t.description.clone()}</div>
                                            {(!t.required_skills.is_empty()).then(|| {
                                                view! {
                                                    <div style="display: flex; flex-wrap: wrap; gap: 4px;">
                                                        {t.required_skills.iter().map(|tool| view! {
                                                            <span style="font-size: 10px; padding: 2px 7px; border-radius: 4px; background: rgba(255,255,255,0.05); color: rgba(255,245,240,0.5);">{tool.clone()}</span>
                                                        }).collect::<Vec<_>>()}
                                                    </div>
                                                }
                                            })}
                                            <Button
                                                primary=true
                                                style="width: 100%; justify-content: center;".to_string()
                                                on_click=Some(Callback::new(move |_| {
                                                    applying.set(Some(t2.clone()));
                                                    apply_result.set(None);
                                                    apply_error.set(String::new());
                                                    goal_input.set(String::new());
                                                }))
                                            >"USE TEMPLATE →"</Button>
                                        </Card>
                                    }
                                }).collect::<Vec<_>>()}
                            </div>
                        }.into_any()
                    }
                }
            }}
        </div>
    }
}
