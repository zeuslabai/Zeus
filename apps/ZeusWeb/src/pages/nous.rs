// ═══════════════════════════════════════════════════════════
// ZEUS — Nous Page — Phase 7: Cognitive Engine Dashboard
// ═══════════════════════════════════════════════════════════

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;
use crate::api;
use crate::components::design::*;

#[component]
pub fn NousPage() -> impl IntoView {
    let reflection = RwSignal::new(Option::<api::NousReflection>::None);
    let capabilities = RwSignal::new(Vec::<api::NousCapability>::new());
    let learning_stats = RwSignal::new(Option::<api::NousLearningStats>::None);
    let lessons = RwSignal::new(Vec::<api::NousLesson>::new());
    let loading = RwSignal::new(true);

    // Graph overview
    let graph_nodes = RwSignal::new(Vec::<api::GraphNode>::new());
    let graph_edges = RwSignal::new(Vec::<api::GraphEdgeType>::new());
    let graph_stats = RwSignal::new(Option::<api::MemoryGraphStats>::None);
    let patterns = RwSignal::new(Vec::<api::MemoryPattern>::new());

    // REPL state
    let repl_input = RwSignal::new(String::new());
    let repl_mode = RwSignal::new("understand".to_string());
    let repl_result: RwSignal<Option<serde_json::Value>> = RwSignal::new(None);
    let repl_busy = RwSignal::new(false);
    let repl_error = RwSignal::new(String::new());

    // Predictive Spawner state (Phase 7 final — Zeus112 `c5df7610`)
    let spawner_status = RwSignal::new(Option::<api::SpawnerStatus>::None);
    let spawner_active = RwSignal::new(Vec::<api::ActiveSpawn>::new());
    let spawner_history = RwSignal::new(Vec::<api::SpawnHistoryEntry>::new());
    let spawn_task_input = RwSignal::new(String::new());
    let spawn_analyzing = RwSignal::new(false);
    let spawn_result: RwSignal<Option<api::SpawnAnalyzeResponse>> = RwSignal::new(None);
    let spawn_error = RwSignal::new(String::new());

    {
        spawn_local(async move {
            let (r, c, s, l, gn, ge, gs, p) = (
                api::fetch_nous_reflection().await,
                api::fetch_nous_capabilities().await,
                api::fetch_nous_learning_stats().await,
                api::fetch_nous_lessons(Some(20)).await,
                api::fetch_graph_nodes(Some(30)).await,
                api::fetch_graph_edges().await,
                api::fetch_graph_stats().await,
                api::fetch_memory_patterns(Some(20)).await,
            );
            if let Ok(v) = r { reflection.set(Some(v)); }
            if let Ok(v) = c { capabilities.set(v.capabilities); }
            if let Ok(v) = s { learning_stats.set(Some(v)); }
            if let Ok(v) = l { lessons.set(v.lessons); }
            if let Ok(v) = gn { graph_nodes.set(v.nodes); }
            if let Ok(v) = ge { graph_edges.set(v.edge_types); }
            if let Ok(v) = gs { graph_stats.set(Some(v)); }
            if let Ok(v) = p { patterns.set(v.patterns); }
            loading.set(false);
        });
        // Load spawner data
        spawn_local(async move {
            if let Ok(v) = api::fetch_spawner_status().await { spawner_status.set(Some(v)); }
            if let Ok(v) = api::fetch_spawner_active().await { spawner_active.set(v); }
            if let Ok(v) = api::fetch_spawner_history().await { spawner_history.set(v); }
        });
    }

    let submit_repl = move || {
        let inp = repl_input.get();
        if inp.trim().is_empty() || repl_busy.get() { return; }
        repl_busy.set(true);
        repl_result.set(None);
        repl_error.set(String::new());
        let mode = repl_mode.get();
        spawn_local(async move {
            let res = if mode == "understand" {
                api::nous_understand(&inp).await
            } else {
                api::nous_reason(&inp).await
            };
            match res {
                Ok(v) => repl_result.set(Some(v)),
                Err(e) => repl_error.set(e),
            }
            repl_busy.set(false);
        });
    };

    view! {
        <div style="padding: 32px;">
            // ── Header ──
            <div style="margin-bottom: 24px;">
                <h1 style="font-family: 'Orbitron', monospace; font-size: 14px; letter-spacing: 6px; color: rgba(255,245,240,0.9); margin: 0;">"NOUS"</h1>
                <p style="font-size: 12px; color: rgba(255,245,240,0.7); margin: 4px 0 0;">
                    {move || if loading.get() { "Loading cognitive engine..." } else { "Cognitive Engine — Reflect · Learn · Reason" }}
                </p>
            </div>

            // ── Row 1: Reflection + Learning Stats ──
            <div style="display: grid; grid-template-columns: 1fr 1fr; gap: 16px; margin-bottom: 24px;">

                // Reflection
                <Card>
                    <SectionTitle>"Self-Reflection"</SectionTitle>
                    {move || {
                        let Some(r) = reflection.get() else {
                            return view! { <div style="color: rgba(255,245,240,0.5); font-size: 12px; text-align: center; padding: 20px;">{"No reflection data — Nous not initialized"}</div> }.into_any();
                        };
                        let health_pct = r.health * 100.0;
                        let state_color = match r.state.as_str() {
                            "Engaged" => "rgba(34,197,94,0.8)",
                            "Idle" | "Resting" => "rgba(255,245,240,0.4)",
                            _ => "rgba(255,140,80,0.8)",
                        };
                        view! {
                            <div>
                                <div style="display: flex; align-items: center; gap: 12px; margin-bottom: 14px;">
                                    <div style="flex: 1;">
                                        <div style="display: flex; justify-content: space-between; margin-bottom: 4px;">
                                            <span style="font-size: 10px; color: rgba(255,245,240,0.4);">"HEALTH"</span>
                                            <span style="font-family: 'Orbitron', monospace; font-size: 10px; color: rgba(255,245,240,0.8);">{format!("{:.0}%", health_pct)}</span>
                                        </div>
                                        <ProgressBar value=health_pct max=100.0 color="rgba(34,197,94,0.7)".to_string() />
                                    </div>
                                    <div style=format!("padding: 4px 10px; border-radius: 6px; background: {}18; border: 1px solid {}40; font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 1px; color: {}; flex-shrink: 0;", state_color, state_color, state_color)>
                                        {r.state.to_uppercase()}
                                    </div>
                                </div>
                                {(!r.current_focus.is_empty()).then(|| view! {
                                    <div style="margin-bottom: 12px; padding: 8px 10px; background: rgba(168,85,247,0.06); border: 1px solid rgba(168,85,247,0.12); border-radius: 6px;">
                                        <div style="font-size: 9px; color: rgba(168,85,247,0.6); font-family: 'Orbitron', monospace; letter-spacing: 1px; margin-bottom: 3px;">"CURRENT FOCUS"</div>
                                        <div style="font-size: 12px; color: rgba(255,245,240,0.8);">{r.current_focus.clone()}</div>
                                    </div>
                                })}
                                {(!r.summary.is_empty()).then(|| view! {
                                    <div style="font-size: 12px; color: rgba(255,245,240,0.55); line-height: 1.5; margin-bottom: 12px;">{r.summary.clone()}</div>
                                })}
                                {(!r.recent_successes.is_empty()).then(|| view! {
                                    <div style="margin-bottom: 8px;">
                                        <div style="font-size: 9px; color: rgba(34,197,94,0.6); font-family: 'Orbitron', monospace; letter-spacing: 1px; margin-bottom: 5px;">"RECENT SUCCESSES"</div>
                                        {r.recent_successes.into_iter().take(3).map(|s| view! {
                                            <div style="font-size: 11px; color: rgba(255,245,240,0.6); padding: 2px 0 2px 10px; border-left: 2px solid rgba(34,197,94,0.3); margin-bottom: 3px;">{s}</div>
                                        }).collect::<Vec<_>>()}
                                    </div>
                                })}
                                {(!r.recent_challenges.is_empty()).then(|| view! {
                                    <div>
                                        <div style="font-size: 9px; color: rgba(239,68,68,0.6); font-family: 'Orbitron', monospace; letter-spacing: 1px; margin-bottom: 5px;">"CHALLENGES"</div>
                                        {r.recent_challenges.into_iter().take(2).map(|c| view! {
                                            <div style="font-size: 11px; color: rgba(255,245,240,0.6); padding: 2px 0 2px 10px; border-left: 2px solid rgba(239,68,68,0.3); margin-bottom: 3px;">{c}</div>
                                        }).collect::<Vec<_>>()}
                                    </div>
                                })}
                            </div>
                        }.into_any()
                    }}
                </Card>

                // Learning Stats
                <Card>
                    <SectionTitle>"Learning Stats"</SectionTitle>
                    {move || {
                        let Some(s) = learning_stats.get() else {
                            return view! { <div style="color: rgba(255,245,240,0.5); font-size: 12px; text-align: center; padding: 20px;">{"No learning data"}</div> }.into_any();
                        };
                        view! {
                            <div>
                                <div style="display: grid; grid-template-columns: 1fr 1fr; gap: 10px; margin-bottom: 16px;">
                                    {[
                                        ("LESSONS", s.total_lessons.to_string(), "rgba(59,130,246,0.7)"),
                                        ("OUTCOMES", s.total_outcomes.to_string(), "rgba(168,85,247,0.7)"),
                                        ("SUCCESS RATE", format!("{:.0}%", s.success_rate * 100.0), "rgba(34,197,94,0.7)"),
                                        ("CONFIDENCE", format!("{:.0}%", s.avg_lesson_confidence * 100.0), "rgba(255,140,80,0.7)"),
                                    ].iter().map(|(label, value, color)| {
                                        let l = *label; let v = value.clone(); let c = color.to_string();
                                        view! {
                                            <div style=format!("background: {}08; border: 1px solid {}25; border-radius: 8px; padding: 10px;", c, c)>
                                                <div style=format!("font-family: 'Orbitron', monospace; font-size: 7px; letter-spacing: 2px; color: {}; margin-bottom: 6px;", c)>{l}</div>
                                                <div style=format!("font-family: 'Orbitron', monospace; font-size: 20px; color: {};", c)>{v}</div>
                                            </div>
                                        }
                                    }).collect::<Vec<_>>()}
                                </div>
                                {(!s.lessons_by_category.is_empty()).then(move || {
                                    let total: u64 = s.lessons_by_category.values().sum();
                                    view! {
                                        <div>
                                            <div style="font-size: 9px; color: rgba(255,245,240,0.35); font-family: 'Orbitron', monospace; letter-spacing: 2px; margin-bottom: 8px;">"BY CATEGORY"</div>
                                            <div style="display: flex; flex-direction: column; gap: 5px;">
                                                {s.lessons_by_category.into_iter().map(|(cat, count)| {
                                                    let pct = if total > 0 { (count as f64 / total as f64) * 100.0 } else { 0.0 };
                                                    view! {
                                                        <div style="display: flex; align-items: center; gap: 8px;">
                                                            <div style="font-size: 10px; color: rgba(255,245,240,0.5); width: 130px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; flex-shrink: 0;">{cat}</div>
                                                            <div style="flex: 1; height: 5px; background: rgba(255,255,255,0.05); border-radius: 3px; overflow: hidden;">
                                                                <div style=format!("height: 100%; width: {:.0}%; background: rgba(59,130,246,0.5); border-radius: 3px;", pct)></div>
                                                            </div>
                                                            <div style="font-family: 'Orbitron', monospace; font-size: 9px; color: rgba(255,245,240,0.4); width: 20px; text-align: right; flex-shrink: 0;">{count.to_string()}</div>
                                                        </div>
                                                    }
                                                }).collect::<Vec<_>>()}
                                            </div>
                                        </div>
                                    }
                                })}
                            </div>
                        }.into_any()
                    }}
                </Card>
            </div>

            // ── Row 2: Capabilities ──
            <Card>
                <SectionTitle>"Capabilities"</SectionTitle>
                <div style="display: grid; grid-template-columns: repeat(auto-fill, minmax(260px, 1fr)); gap: 10px; margin-top: 8px;">
                    {move || {
                        let caps = capabilities.get();
                        if caps.is_empty() {
                            return view! { <div style="color: rgba(255,245,240,0.5); font-size: 12px; padding: 16px; grid-column: 1/-1;">{"No capability data"}</div> }.into_any();
                        }
                        view! {
                            <>{caps.into_iter().map(|cap| {
                                let prof_pct = cap.proficiency * 100.0;
                                let sr_pct = cap.success_rate * 100.0;
                                let prof_color = if prof_pct >= 70.0 { "rgba(34,197,94,0.7)" }
                                    else if prof_pct >= 40.0 { "rgba(255,140,80,0.7)" }
                                    else { "rgba(239,68,68,0.7)" };
                                view! {
                                    <div style="background: rgba(255,255,255,0.02); border: 1px solid rgba(255,60,20,0.08); border-radius: 10px; padding: 12px;">
                                        <div style="display: flex; justify-content: space-between; align-items: flex-start; margin-bottom: 8px;">
                                            <div style="font-size: 13px; font-weight: 600; color: rgba(255,245,240,0.85); flex: 1; margin-right: 8px;">{cap.name.clone()}</div>
                                            <div style=format!("font-family: 'Orbitron', monospace; font-size: 11px; color: {}; flex-shrink: 0;", prof_color)>{format!("{:.0}%", prof_pct)}</div>
                                        </div>
                                        <ProgressBar value=prof_pct max=100.0 color=prof_color.to_string() />
                                        <div style="display: flex; justify-content: space-between; margin-top: 8px;">
                                            <span style="font-size: 10px; color: rgba(255,245,240,0.3);">{format!("{} uses", cap.usage_count)}</span>
                                            <span style="font-size: 10px; color: rgba(255,245,240,0.3);">{format!("{:.0}% success", sr_pct)}</span>
                                        </div>
                                    </div>
                                }
                            }).collect::<Vec<_>>()}</>
                        }.into_any()
                    }}
                </div>
            </Card>

            // ── Row 3: Lessons + REPL ──
            <div style="display: grid; grid-template-columns: 1fr 1fr; gap: 16px; margin-top: 24px;">

                // Lessons feed
                <Card>
                    <SectionTitle>"Learned Lessons"</SectionTitle>
                    <div style="display: flex; flex-direction: column; gap: 8px; max-height: 380px; overflow-y: auto; margin-top: 8px;">
                        {move || {
                            let lsns = lessons.get();
                            if lsns.is_empty() {
                                return view! { <div style="color: rgba(255,245,240,0.5); font-size: 12px; text-align: center; padding: 24px;">{"No lessons recorded yet"}</div> }.into_any();
                            }
                            view! {
                                <>{lsns.into_iter().map(|l| {
                                    let conf_pct = l.confidence * 100.0;
                                    view! {
                                        <div style="padding: 10px 12px; background: rgba(255,255,255,0.02); border: 1px solid rgba(255,60,20,0.08); border-radius: 8px;">
                                            <div style="display: flex; align-items: center; justify-content: space-between; margin-bottom: 6px;">
                                                <Badge text=l.category.clone() color="rgba(168,85,247,0.7)".to_string() />
                                                <span style="font-family: 'Orbitron', monospace; font-size: 9px; color: rgba(255,245,240,0.3);">{format!("{:.0}%", conf_pct)}</span>
                                            </div>
                                            <div style="font-size: 12px; color: rgba(255,245,240,0.8); margin-bottom: 4px; line-height: 1.4;">{l.insight.clone()}</div>
                                            {(!l.recommendation.is_empty()).then(|| view! {
                                                <div style="font-size: 10px; color: rgba(255,245,240,0.4); font-style: italic; margin-top: 3px;">{l.recommendation.clone()}</div>
                                            })}
                                        </div>
                                    }
                                }).collect::<Vec<_>>()}</>
                            }.into_any()
                        }}
                    </div>
                </Card>

                // Cognitive REPL
                <Card>
                    <SectionTitle>"Cognitive REPL"</SectionTitle>
                    <div style="margin-top: 8px;">
                        // Mode toggle
                        <div style="display: flex; gap: 6px; margin-bottom: 12px;">
                            <button
                                on:click=move |_| repl_mode.set("understand".to_string())
                                style=move || format!(
                                    "flex: 1; padding: 7px; border-radius: 6px; font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 1px; cursor: pointer; border: 1px solid {}; background: {}; color: {};",
                                    if repl_mode.get() == "understand" { "rgba(59,130,246,0.4)" } else { "rgba(255,245,240,0.08)" },
                                    if repl_mode.get() == "understand" { "rgba(59,130,246,0.1)" } else { "transparent" },
                                    if repl_mode.get() == "understand" { "rgba(255,245,240,0.9)" } else { "rgba(255,245,240,0.4)" },
                                )
                            >"🔍 UNDERSTAND"</button>
                            <button
                                on:click=move |_| repl_mode.set("reason".to_string())
                                style=move || format!(
                                    "flex: 1; padding: 7px; border-radius: 6px; font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 1px; cursor: pointer; border: 1px solid {}; background: {}; color: {};",
                                    if repl_mode.get() == "reason" { "rgba(168,85,247,0.4)" } else { "rgba(255,245,240,0.08)" },
                                    if repl_mode.get() == "reason" { "rgba(168,85,247,0.1)" } else { "transparent" },
                                    if repl_mode.get() == "reason" { "rgba(255,245,240,0.9)" } else { "rgba(255,245,240,0.4)" },
                                )
                            >"🧠 REASON"</button>
                        </div>
                        // Input row
                        <div style="display: flex; gap: 8px; margin-bottom: 12px;">
                            <input
                                type="text"
                                prop:value=move || repl_input.get()
                                placeholder=move || if repl_mode.get() == "understand" {
                                    "e.g. schedule a meeting tomorrow"
                                } else {
                                    "e.g. how should I deploy to production?"
                                }
                                on:input=move |e| {
                                    use wasm_bindgen::JsCast;
                                    let val = e.target().and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok()).map(|i| i.value()).unwrap_or_default();
                                    repl_input.set(val);
                                }
                                on:keydown=move |e| {
                                    use wasm_bindgen::JsCast;
                                    if e.unchecked_ref::<web_sys::KeyboardEvent>().key() == "Enter" {
                                        submit_repl();
                                    }
                                }
                                style="flex: 1; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.12); border-radius: 8px; padding: 8px 12px; color: rgba(255,245,240,0.9); font-family: 'Rajdhani', sans-serif; font-size: 13px; outline: none;"
                            />
                            <button
                                on:click=move |_| submit_repl()
                                disabled=move || repl_busy.get()
                                style=move || format!(
                                    "padding: 8px 16px; background: {}; border: 1px solid {}; border-radius: 8px; color: rgba(255,245,240,0.9); font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 2px; cursor: {};",
                                    if repl_busy.get() { "rgba(255,60,20,0.05)" } else { "rgba(255,60,20,0.12)" },
                                    if repl_busy.get() { "rgba(255,60,20,0.1)" } else { "rgba(255,60,20,0.3)" },
                                    if repl_busy.get() { "wait" } else { "pointer" },
                                )
                            >{move || if repl_busy.get() { "..." } else { "RUN" }}</button>
                        </div>
                        // Error
                        {move || (!repl_error.get().is_empty()).then(|| view! {
                            <div style="padding: 8px 12px; background: rgba(239,68,68,0.08); border: 1px solid rgba(239,68,68,0.2); border-radius: 6px; font-size: 12px; color: rgba(239,68,68,0.8); margin-bottom: 10px;">
                                {repl_error.get()}
                            </div>
                        })}
                        // Result
                        {move || repl_result.get().map(|result| {
                            let mode = repl_mode.get();
                            if mode == "understand" {
                                let intent_type = result.get("intent_type").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                let confidence = result.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.0);
                                let urgency = result.get("urgency").and_then(|v| v.as_f64()).unwrap_or(0.0);
                                let entities = result.get("entities").and_then(|v| v.as_array()).cloned().unwrap_or_default();
                                view! {
                                    <div style="background: rgba(59,130,246,0.05); border: 1px solid rgba(59,130,246,0.15); border-radius: 8px; padding: 12px;">
                                        <div style="font-family: 'Orbitron', monospace; font-size: 10px; color: rgba(59,130,246,0.7); margin-bottom: 10px; letter-spacing: 2px;">"INTENT ANALYSIS"</div>
                                        <div style="display: grid; grid-template-columns: 1fr 1fr; gap: 8px; margin-bottom: 10px;">
                                            <div>
                                                <div style="font-size: 9px; color: rgba(255,245,240,0.35); margin-bottom: 3px;">"INTENT TYPE"</div>
                                                <div style="font-size: 11px; color: rgba(255,245,240,0.8);">{intent_type}</div>
                                            </div>
                                            <div>
                                                <div style="font-size: 9px; color: rgba(255,245,240,0.35); margin-bottom: 3px;">"CONFIDENCE"</div>
                                                <div style="font-family: 'Orbitron', monospace; font-size: 18px; color: rgba(34,197,94,0.9);">{format!("{:.0}%", confidence * 100.0)}</div>
                                            </div>
                                        </div>
                                        {(!entities.is_empty()).then(|| view! {
                                            <div style="margin-bottom: 10px;">
                                                <div style="font-size: 9px; color: rgba(255,245,240,0.35); margin-bottom: 5px;">"ENTITIES"</div>
                                                <div style="display: flex; flex-wrap: wrap; gap: 4px;">
                                                    {entities.iter().map(|e| {
                                                        let text = e.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                                        let etype = e.get("entity_type").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                                        view! {
                                                            <span style="padding: 2px 8px; background: rgba(59,130,246,0.1); border: 1px solid rgba(59,130,246,0.2); border-radius: 4px; font-size: 10px; color: rgba(255,245,240,0.7);">
                                                                {text}{if etype.is_empty() { String::new() } else { format!(" ({})", etype) }}
                                                            </span>
                                                        }
                                                    }).collect::<Vec<_>>()}
                                                </div>
                                            </div>
                                        })}
                                        <div style="display: flex; align-items: center; gap: 8px;">
                                            <span style="font-size: 9px; color: rgba(255,245,240,0.35); flex-shrink: 0;">"URGENCY"</span>
                                            <ProgressBar value={urgency * 100.0} max=100.0 color="rgba(255,140,80,0.6)".to_string() />
                                        </div>
                                    </div>
                                }.into_any()
                            } else {
                                let conclusion = result.get("conclusion").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                let confidence = result.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.0);
                                let steps = result.get("steps").and_then(|v| v.as_array()).cloned().unwrap_or_default();
                                let thinking_ms = result.get("thinking_time_ms").and_then(|v| v.as_u64()).unwrap_or(0);
                                view! {
                                    <div style="background: rgba(168,85,247,0.05); border: 1px solid rgba(168,85,247,0.15); border-radius: 8px; padding: 12px;">
                                        <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 10px;">
                                            <div style="font-family: 'Orbitron', monospace; font-size: 10px; color: rgba(168,85,247,0.7); letter-spacing: 2px;">"REASONING CHAIN"</div>
                                            <div style="font-family: 'Orbitron', monospace; font-size: 9px; color: rgba(255,245,240,0.3);">{format!("{}ms", thinking_ms)}</div>
                                        </div>
                                        <div style="display: flex; flex-direction: column; gap: 6px; margin-bottom: 10px; max-height: 180px; overflow-y: auto;">
                                            {steps.iter().enumerate().map(|(i, step)| {
                                                let thought = step.get("thought").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                                let step_type = step.get("step_type").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                                view! {
                                                    <div style="display: flex; gap: 8px; align-items: flex-start;">
                                                        <div style="width: 18px; height: 18px; border-radius: 50%; background: rgba(168,85,247,0.15); border: 1px solid rgba(168,85,247,0.3); display: flex; align-items: center; justify-content: center; font-family: 'Orbitron', monospace; font-size: 8px; color: rgba(168,85,247,0.7); flex-shrink: 0;">{(i + 1).to_string()}</div>
                                                        <div style="flex: 1;">
                                                            <div style="font-size: 9px; color: rgba(168,85,247,0.5); font-family: 'Orbitron', monospace; letter-spacing: 1px; margin-bottom: 2px;">{step_type}</div>
                                                            <div style="font-size: 11px; color: rgba(255,245,240,0.7); line-height: 1.4;">{thought}</div>
                                                        </div>
                                                    </div>
                                                }
                                            }).collect::<Vec<_>>()}
                                        </div>
                                        <div style="padding: 8px 10px; background: rgba(168,85,247,0.08); border-radius: 6px; border-left: 3px solid rgba(168,85,247,0.4);">
                                            <div style="font-size: 9px; color: rgba(168,85,247,0.5); font-family: 'Orbitron', monospace; margin-bottom: 4px;">{format!("CONCLUSION — {:.0}% CONFIDENCE", confidence * 100.0)}</div>
                                            <div style="font-size: 12px; color: rgba(255,245,240,0.85); line-height: 1.5;">{conclusion}</div>
                                        </div>
                                    </div>
                                }.into_any()
                            }
                        })}
                    </div>
                </Card>
            </div>

            // ── Row 4: Knowledge Graph Overview ──
            <div style="display: grid; grid-template-columns: 1fr 1fr; gap: 16px; margin-top: 24px;">

                // Top Nodes
                <Card>
                    <SectionTitle>"Knowledge Graph Nodes"</SectionTitle>
                    {move || {
                        let gs = graph_stats.get();
                        let node_count = gs.as_ref().and_then(|s| s.graph.get("entity_count").and_then(|v| v.as_u64())).unwrap_or(0);
                        let rel_count = gs.as_ref().and_then(|s| s.graph.get("relationship_count").and_then(|v| v.as_u64())).unwrap_or(0);
                        let msg_count = gs.as_ref().and_then(|s| s.memory.get("message_count").and_then(|v| v.as_u64())).unwrap_or(0);
                        view! {
                            <div>
                                <div style="display: flex; gap: 12px; margin-bottom: 14px;">
                                    {[
                                        ("NODES", node_count.to_string(), "rgba(59,130,246,0.7)"),
                                        ("EDGES", rel_count.to_string(), "rgba(168,85,247,0.7)"),
                                        ("MESSAGES", msg_count.to_string(), "rgba(255,140,80,0.7)"),
                                    ].iter().map(|(l, v, c)| {
                                        let label = *l; let val = v.clone(); let col = c.to_string();
                                        view! {
                                            <div style=format!("flex: 1; background: {}08; border: 1px solid {}20; border-radius: 8px; padding: 8px; text-align: center;", col, col)>
                                                <div style=format!("font-family: 'Orbitron', monospace; font-size: 16px; color: {};", col)>{val}</div>
                                                <div style=format!("font-size: 8px; color: {}; font-family: 'Orbitron', monospace; letter-spacing: 1px; margin-top: 3px;", col)>{label}</div>
                                            </div>
                                        }
                                    }).collect::<Vec<_>>()}
                                </div>
                                <div style="display: flex; flex-direction: column; gap: 4px; max-height: 220px; overflow-y: auto;">
                                    {move || {
                                        graph_nodes.get().into_iter().take(15).map(|node| {
                                            let type_color = match node.node_type.to_lowercase().as_str() {
                                                "person" | "agent" => "rgba(59,130,246,0.7)",
                                                "concept" | "topic" => "rgba(168,85,247,0.7)",
                                                "tool" => "rgba(255,140,80,0.7)",
                                                _ => "rgba(255,245,240,0.4)",
                                            };
                                            view! {
                                                <div style="display: flex; align-items: center; gap: 8px; padding: 6px 8px; border-radius: 6px; background: rgba(255,255,255,0.02);">
                                                    <div style=format!("width: 6px; height: 6px; border-radius: 50%; background: {}; flex-shrink: 0;", type_color)></div>
                                                    <div style="flex: 1; font-size: 12px; color: rgba(255,245,240,0.8); overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">{node.name.clone()}</div>
                                                    <div style=format!("font-size: 9px; color: {}; font-family: 'Orbitron', monospace; flex-shrink: 0;", type_color)>{node.node_type.to_uppercase()}</div>
                                                    <div style="font-family: 'Orbitron', monospace; font-size: 9px; color: rgba(255,245,240,0.3); flex-shrink: 0;">{node.mention_count.to_string()}</div>
                                                </div>
                                            }
                                        }).collect::<Vec<_>>()
                                    }}
                                </div>
                            </div>
                        }
                    }}
                </Card>

                // Patterns + Edge types
                <Card>
                    <SectionTitle>"Patterns & Relationships"</SectionTitle>
                    <div style="margin-bottom: 16px;">
                        <div style="font-size: 9px; color: rgba(255,245,240,0.35); font-family: 'Orbitron', monospace; letter-spacing: 2px; margin-bottom: 8px;">"EDGE TYPES"</div>
                        {move || {
                            let edges = graph_edges.get();
                            if edges.is_empty() {
                                return view! { <div style="font-size: 11px; color: rgba(255,245,240,0.5);">{"No edge data"}</div> }.into_any();
                            }
                            let max_count = edges.iter().map(|e| e.count).max().unwrap_or(1).max(1);
                            view! {
                                <div style="display: flex; flex-direction: column; gap: 5px;">
                                    {edges.into_iter().take(6).map(|e| {
                                        let pct = (e.count as f64 / max_count as f64) * 100.0;
                                        view! {
                                            <div style="display: flex; align-items: center; gap: 8px;">
                                                <div style="font-size: 10px; color: rgba(255,245,240,0.5); width: 120px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; flex-shrink: 0;">{e.relationship_type.clone()}</div>
                                                <div style="flex: 1; height: 5px; background: rgba(255,255,255,0.05); border-radius: 3px; overflow: hidden;">
                                                    <div style=format!("height: 100%; width: {:.0}%; background: rgba(168,85,247,0.5); border-radius: 3px;", pct)></div>
                                                </div>
                                                <div style="font-family: 'Orbitron', monospace; font-size: 9px; color: rgba(255,245,240,0.4); width: 24px; text-align: right; flex-shrink: 0;">{e.count.to_string()}</div>
                                            </div>
                                        }
                                    }).collect::<Vec<_>>()}
                                </div>
                            }.into_any()
                        }}
                    </div>
                    <div style="border-top: 1px solid rgba(255,60,20,0.08); padding-top: 14px;">
                        <div style="font-size: 9px; color: rgba(255,245,240,0.35); font-family: 'Orbitron', monospace; letter-spacing: 2px; margin-bottom: 8px;">"INTERACTION PATTERNS"</div>
                        {move || {
                            let pats = patterns.get();
                            if pats.is_empty() {
                                return view! { <div style="font-size: 11px; color: rgba(255,245,240,0.5);">{"No patterns detected yet"}</div> }.into_any();
                            }
                            view! {
                                <div style="display: flex; flex-direction: column; gap: 6px; max-height: 180px; overflow-y: auto;">
                                    {pats.into_iter().take(8).map(|p| {
                                        view! {
                                            <div style="display: flex; align-items: flex-start; gap: 8px; padding: 6px 8px; background: rgba(255,255,255,0.02); border-radius: 6px;">
                                                <Badge text=p.pattern_type.clone() color="rgba(255,140,80,0.6)".to_string() />
                                                <div style="flex: 1; font-size: 11px; color: rgba(255,245,240,0.65); line-height: 1.4; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">{p.content.clone()}</div>
                                                <div style="font-family: 'Orbitron', monospace; font-size: 9px; color: rgba(255,245,240,0.3); flex-shrink: 0;">{p.frequency.to_string()}{"×"}</div>
                                            </div>
                                        }
                                    }).collect::<Vec<_>>()}
                                </div>
                            }.into_any()
                        }}
                    </div>
                </Card>
            </div>

            // ── Predictive Spawning (Phase 7 final — Zeus112 `c5df7610`) ──
            <div style="grid-column: 1 / -1; margin-top: 8px;">
                <Card>
                    <SectionTitle>"Predictive Spawning"</SectionTitle>
                    <div style="display: grid; grid-template-columns: 1fr 1fr 1fr; gap: 16px; margin-top: 8px;">

                        // Status + Criteria
                        <div>
                            <div style="font-size: 9px; color: rgba(255,245,240,0.35); font-family: 'Orbitron', monospace; letter-spacing: 2px; margin-bottom: 10px;">"STATUS"</div>
                            {move || match spawner_status.get() {
                                None => view! { <div style="font-size: 11px; color: rgba(255,245,240,0.2);">"Loading..."</div> }.into_any(),
                                Some(s) => view! {
                                    <div style="display: flex; flex-direction: column; gap: 8px;">
                                        <div style="display: flex; align-items: center; gap: 8px;">
                                            <div style=format!("width: 8px; height: 8px; border-radius: 50%; background: {};", if s.health.is_healthy { "#22c55e" } else { "#ef4444" }) />
                                            <span style="font-size: 12px; color: rgba(255,245,240,0.8); font-weight: 600;">
                                                {if s.health.is_healthy { "Healthy" } else { "Degraded" }}
                                            </span>
                                        </div>
                                        <div style="display: grid; grid-template-columns: 1fr 1fr; gap: 6px;">
                                            {[
                                                ("Active", s.health.active_spawns.to_string(), "#3b82f6"),
                                                ("Completed", s.health.completed_spawns.to_string(), "#22c55e"),
                                                ("Failed", s.health.failed_spawns.to_string(), "#ef4444"),
                                                ("Success %", format!("{:.0}%", s.health.success_rate * 100.0), "#22c55e"),
                                            ].into_iter().map(|(label, val, color)| view! {
                                                <div style="background: rgba(255,255,255,0.02); border-radius: 6px; padding: 6px 8px;">
                                                    <div style="font-size: 9px; color: rgba(255,245,240,0.3);">{label}</div>
                                                    <div style=format!("font-family: 'Orbitron', monospace; font-size: 12px; font-weight: 700; color: {};", color)>{val}</div>
                                                </div>
                                            }).collect::<Vec<_>>()}
                                        </div>
                                        <div style="border-top: 1px solid rgba(255,255,255,0.04); padding-top: 8px; display: flex; flex-direction: column; gap: 4px;">
                                            <div style="font-size: 9px; color: rgba(255,245,240,0.3); font-family: 'Orbitron', monospace; letter-spacing: 1px; margin-bottom: 4px;">"CRITERIA"</div>
                                            <div style="display: flex; justify-content: space-between; font-size: 11px;">
                                                <span style="color: rgba(255,245,240,0.35);">"Min complexity"</span>
                                                <span style="color: rgba(255,245,240,0.7); font-weight: 600;">{s.criteria.min_complexity.clone()}</span>
                                            </div>
                                            <div style="display: flex; justify-content: space-between; font-size: 11px;">
                                                <span style="color: rgba(255,245,240,0.35);">"Max agents"</span>
                                                <span style="color: rgba(255,245,240,0.7); font-weight: 600;">{s.criteria.max_active_agents.to_string()}</span>
                                            </div>
                                            <div style="display: flex; justify-content: space-between; font-size: 11px;">
                                                <span style="color: rgba(255,245,240,0.35);">"Parallel"</span>
                                                <span style=format!("color: {}; font-weight: 600;", if s.criteria.enable_parallel { "#22c55e" } else { "#6b7280" })>
                                                    {if s.criteria.enable_parallel { "ON" } else { "OFF" }}
                                                </span>
                                            </div>
                                            <div style="display: flex; justify-content: space-between; font-size: 11px;">
                                                <span style="color: rgba(255,245,240,0.35);">"Specialization"</span>
                                                <span style=format!("color: {}; font-weight: 600;", if s.criteria.enable_specialization { "#22c55e" } else { "#6b7280" })>
                                                    {if s.criteria.enable_specialization { "ON" } else { "OFF" }}
                                                </span>
                                            </div>
                                        </div>
                                    </div>
                                }.into_any()
                            }}
                            // Active spawns
                            {move || {
                                let active = spawner_active.get();
                                if active.is_empty() { return None; }
                                Some(view! {
                                    <div style="margin-top: 12px; border-top: 1px solid rgba(255,255,255,0.04); padding-top: 8px;">
                                        <div style="font-size: 9px; color: rgba(255,245,240,0.3); font-family: 'Orbitron', monospace; letter-spacing: 1px; margin-bottom: 6px;">"ACTIVE SPAWNS"</div>
                                        {active.into_iter().map(|sp| view! {
                                            <div style="padding: 5px 0; border-bottom: 1px solid rgba(255,255,255,0.03);">
                                                <div style="font-size: 11px; color: rgba(255,245,240,0.7); font-weight: 600;">{sp.role.clone()}</div>
                                                <div style="font-size: 10px; color: rgba(255,245,240,0.35); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; margin-top: 1px;">{sp.task.clone()}</div>
                                            </div>
                                        }).collect::<Vec<_>>()}
                                    </div>
                                })
                            }}
                        </div>

                        // Spawn Analyzer REPL
                        <div>
                            <div style="font-size: 9px; color: rgba(255,245,240,0.35); font-family: 'Orbitron', monospace; letter-spacing: 2px; margin-bottom: 10px;">"SPAWN ANALYZER"</div>
                            <textarea
                                placeholder="Describe a task to analyze spawning strategy..."
                                prop:value=move || spawn_task_input.get()
                                on:input=move |ev| spawn_task_input.set(event_target_value(&ev))
                                rows=4
                                style="width: 100%; padding: 10px 12px; background: rgba(255,255,255,0.02); border: 1px solid rgba(34,197,94,0.12); border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 12px; outline: none; font-family: 'Rajdhani', sans-serif; resize: none; box-sizing: border-box;"
                            />
                            <button
                                disabled=move || spawn_analyzing.get() || spawn_task_input.get().trim().is_empty()
                                on:click=move |_| {
                                    let task = spawn_task_input.get_untracked();
                                    if task.trim().is_empty() || spawn_analyzing.get() { return; }
                                    spawn_analyzing.set(true);
                                    spawn_result.set(None);
                                    spawn_error.set(String::new());
                                    spawn_local(async move {
                                        match api::spawner_analyze(&task, vec![]).await {
                                            Ok(r) => { spawn_result.set(Some(r)); spawn_analyzing.set(false); }
                                            Err(e) => { spawn_error.set(e); spawn_analyzing.set(false); }
                                        }
                                    });
                                }
                                style=move || format!(
                                    "width: 100%; margin-top: 8px; font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; padding: 9px; border-radius: 7px; cursor: {}; background: {}; border: 1px solid rgba(34,197,94,0.25); color: rgba(255,245,240,0.8);",
                                    if spawn_analyzing.get() || spawn_task_input.get().trim().is_empty() { "not-allowed" } else { "pointer" },
                                    if spawn_analyzing.get() { "rgba(34,197,94,0.05)" } else { "rgba(34,197,94,0.1)" }
                                )
                            >{move || if spawn_analyzing.get() { "ANALYZING..." } else { "ANALYZE ▶" }}</button>
                            {move || { let e = spawn_error.get(); (!e.is_empty()).then(|| view! {
                                <div style="margin-top: 8px; padding: 8px 10px; background: rgba(239,68,68,0.06); border-radius: 6px; font-size: 11px; color: #ef4444;">{e}</div>
                            })}}
                            {move || spawn_result.get().map(|r| {
                                let decision_color = if r.should_spawn { "#22c55e" } else { "#eab308" };
                                let decision_label = if r.should_spawn { "SPAWN RECOMMENDED" } else { "NO SPAWN NEEDED" };
                                view! {
                                    <div style="margin-top: 12px; border: 1px solid rgba(34,197,94,0.15); border-radius: 8px; padding: 12px; background: rgba(34,197,94,0.03);">
                                        <div style="display: flex; align-items: center; gap: 8px; margin-bottom: 8px;">
                                            <span style=format!("font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 1px; padding: 2px 8px; border-radius: 4px; background: {}1a; color: {};", decision_color, decision_color)>
                                                {decision_label}
                                            </span>
                                            <span style="font-family: 'Orbitron', monospace; font-size: 10px; color: #22c55e; margin-left: auto;">
                                                {format!("{:.1}× speedup", r.estimated_speedup)}
                                            </span>
                                        </div>
                                        <div style="font-size: 12px; color: rgba(255,245,240,0.6); line-height: 1.5; margin-bottom: 8px;">{r.rationale.clone()}</div>
                                        <div style="display: flex; gap: 8px; margin-bottom: 8px; flex-wrap: wrap;">
                                            <span style="font-size: 10px; padding: 2px 8px; background: rgba(255,255,255,0.04); border-radius: 4px; color: rgba(255,245,240,0.7);">
                                                {format!("complexity: {}", r.analysis.detected_complexity)}
                                            </span>
                                            <span style="font-size: 10px; padding: 2px 8px; background: rgba(255,255,255,0.04); border-radius: 4px; color: rgba(255,245,240,0.7);">
                                                {format!("{} tools", r.analysis.tool_count)}
                                            </span>
                                        </div>
                                        {(!r.agents.is_empty()).then(|| view! {
                                            <div>
                                                <div style="font-size: 9px; color: rgba(255,245,240,0.3); font-family: 'Orbitron', monospace; letter-spacing: 1px; margin-bottom: 6px;">"RECOMMENDED AGENTS"</div>
                                                <div style="display: flex; flex-direction: column; gap: 4px;">
                                                    {r.agents.into_iter().map(|a| view! {
                                                        <div style="padding: 6px 8px; background: rgba(255,255,255,0.02); border-radius: 5px; border-left: 2px solid rgba(34,197,94,0.3);">
                                                            <div style="font-size: 11px; color: rgba(255,245,240,0.8); font-weight: 600;">{a.role.clone()}</div>
                                                            <div style="font-size: 10px; color: rgba(255,245,240,0.4); margin-top: 1px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">{a.task.clone()}</div>
                                                            {(!a.tools.is_empty()).then(|| view! {
                                                                <div style="display: flex; gap: 3px; flex-wrap: wrap; margin-top: 3px;">
                                                                    {a.tools.into_iter().take(4).map(|t| view! {
                                                                        <span style="font-size: 9px; padding: 1px 5px; background: rgba(34,197,94,0.08); border-radius: 3px; color: rgba(34,197,94,0.6);">{t}</span>
                                                                    }).collect::<Vec<_>>()}
                                                                </div>
                                                            })}
                                                        </div>
                                                    }).collect::<Vec<_>>()}
                                                </div>
                                            </div>
                                        })}
                                    </div>
                                }
                            })}
                        </div>

                        // Spawn History
                        <div>
                            <div style="font-size: 9px; color: rgba(255,245,240,0.35); font-family: 'Orbitron', monospace; letter-spacing: 2px; margin-bottom: 10px;">"SPAWN HISTORY"</div>
                            {move || {
                                let hist = spawner_history.get();
                                if hist.is_empty() {
                                    return view! { <div style="font-size: 11px; color: rgba(255,245,240,0.2);">"No spawn history yet"</div> }.into_any();
                                }
                                view! {
                                    <div style="display: flex; flex-direction: column; gap: 4px; max-height: 320px; overflow-y: auto;">
                                        {hist.into_iter().take(15).map(|h| {
                                            let status_color = if h.success { "#22c55e" } else { "#ef4444" };
                                            let status_label = if h.success { "OK" } else { "FAIL" };
                                            view! {
                                                <div style="padding: 6px 8px; background: rgba(255,255,255,0.02); border-radius: 5px; display: flex; align-items: center; gap: 8px;">
                                                    <span style=format!("font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 1px; padding: 1px 6px; border-radius: 3px; background: {}1a; color: {}; flex-shrink: 0;", status_color, status_color)>
                                                        {status_label}
                                                    </span>
                                                    <span style="font-family: 'Orbitron', monospace; font-size: 9px; color: rgba(255,245,240,0.4); flex-shrink: 0;">
                                                        {format!("{}ms", h.duration_ms)}
                                                    </span>
                                                    {h.output.as_ref().map(|out| view! {
                                                        <span style="font-size: 10px; color: rgba(255,245,240,0.35); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; flex: 1;">
                                                            {out.chars().take(40).collect::<String>()}
                                                        </span>
                                                    })}
                                                </div>
                                            }
                                        }).collect::<Vec<_>>()}
                                    </div>
                                }.into_any()
                            }}
                        </div>

                    </div>
                </Card>
            </div>
        </div>
    }
}
