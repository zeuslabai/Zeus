// ═══════════════════════════════════════════════════════════
// ZEUS — Workflows Page — Prometheus Orchestration & Plans
// Phase 2: DAG visualization, live workflow tracking, node progress
// ═══════════════════════════════════════════════════════════

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;
use crate::api;
use crate::components::design::*;
use crate::components::sentient_orb::SentientOrb;

#[component]
pub fn WorkflowsPage() -> impl IntoView {
    let state = RwSignal::new(api::PrometheusStateResponse::default());
    let loading = RwSignal::new(true);
    let goal_input = RwSignal::new(String::new());
    let plan_result = RwSignal::new(Option::<api::PrometheusPlanResponse>::None);
    let exec_result = RwSignal::new(Option::<api::PrometheusExecuteResponse>::None);
    let message = RwSignal::new(String::new());
    let planning = RwSignal::new(false);
    let executing = RwSignal::new(false);
    let workflows = RwSignal::new(Vec::<api::WorkflowSummary>::new());
    let selected_workflow = RwSignal::new(Option::<api::WorkflowDetail>::None);
    let tab = RwSignal::new("create".to_string()); // "create" | "active" | "history"

    // Fetch state + workflows on mount
    {
        spawn_local(async move {
            if let Ok(s) = api::fetch_prometheus_state().await { state.set(s); }
            if let Ok(w) = api::fetch_workflows().await { workflows.set(w.workflows); }
            loading.set(false);
        });
    }

    // Auto-poll active workflows every 3s
    {
        let workflows = workflows;
        let selected_workflow = selected_workflow;
        spawn_local(async move {
            loop {
                gloo_timers::future::TimeoutFuture::new(3000).await;
                if let Ok(w) = api::fetch_workflows().await {
                    let has_active = w.workflows.iter().any(|wf| wf.status == "running" || wf.status == "pending");
                    workflows.set(w.workflows);
                    // Refresh selected workflow detail if it's running
                    if let Some(sel) = selected_workflow.get_untracked()
                        && (sel.status == "running" || sel.status == "pending")
                            && let Ok(detail) = api::fetch_workflow(&sel.workflow_id).await {
                                selected_workflow.set(Some(detail));
                            }
                    if !has_active { break; }
                }
            }
        });
    }

    let create_plan = move |_| {
        let goal = goal_input.get();
        if goal.trim().is_empty() { return; }
        planning.set(true);
        plan_result.set(None);
        exec_result.set(None);
        message.set(String::new());
        spawn_local(async move {
            match api::prometheus_plan(&goal, None).await {
                Ok(p) => {
                    message.set(format!("Plan created: {} nodes, ~{}ms est.", p.nodes, p.estimated_total_ms));
                    plan_result.set(Some(p));
                }
                Err(e) => message.set(format!("Error: {}", e)),
            }
            planning.set(false);
        });
    };

    let execute_plan = move |_| {
        let goal = goal_input.get();
        if goal.trim().is_empty() { return; }
        executing.set(true);
        exec_result.set(None);
        message.set(String::new());
        spawn_local(async move {
            match api::prometheus_execute(&goal, None).await {
                Ok(e) => {
                    message.set(format!("Execution {}: {} steps", e.status, e.total_steps));
                    exec_result.set(Some(e));
                    // Refresh workflow list
                    if let Ok(w) = api::fetch_workflows().await { workflows.set(w.workflows); }
                    tab.set("active".to_string());
                }
                Err(e) => message.set(format!("Error: {}", e)),
            }
            executing.set(false);
        });
    };

    let refresh_state = move |_| {
        spawn_local(async move {
            if let Ok(s) = api::fetch_prometheus_state().await { state.set(s); }
            if let Ok(w) = api::fetch_workflows().await { workflows.set(w.workflows); }
        });
    };

    let view_workflow = move |id: String| {
        spawn_local(async move {
            if let Ok(detail) = api::fetch_workflow(&id).await {
                selected_workflow.set(Some(detail));
            }
        });
    };

    let close_detail = move |_| {
        selected_workflow.set(None);
    };

    view! {
        <div style="padding: 32px;">
            // Header
            <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 24px;">
                <div style="display: flex; align-items: center; gap: 16px;">
                    <SentientOrb size=48 mode="active" />
                    <div>
                        <h1 style="font-family: 'Orbitron', monospace; font-size: 14px; letter-spacing: 6px; color: rgba(255,245,240,0.9); margin: 0;">"PROMETHEUS ENGINE"</h1>
                        <p style="color: rgba(255,245,240,0.7); font-size: 12px; margin: 4px 0 0;">
                            {move || {
                                if loading.get() { "Initializing orchestrator...".to_string() }
                                else {
                                    let s = state.get();
                                    let wf = workflows.get();
                                    let active = wf.iter().filter(|w| w.status == "running").count();
                                    format!("{} agents ({} working) \u{2022} {} workflows ({} active)",
                                        s.total_agents, s.total_agents - s.idle_agents, wf.len(), active)
                                }
                            }}
                        </p>
                    </div>
                </div>
                <Button on_click=Some(Callback::new(refresh_state))>
                    <Icon name="activity" size=14 /> " Refresh"
                </Button>
            </div>

            // Prometheus agent cards
            <div style="display: flex; gap: 12px; margin-bottom: 24px; flex-wrap: wrap;">
                {move || {
                    let agents = state.get().agents;
                    agents.into_iter().map(|a| {
                        let is_working = a.status == "working" || a.status == "active";
                        let status_color = match a.status.as_str() {
                            "idle" => "#eab308",
                            "working" | "active" => "#22c55e",
                            "error" => "#ef4444",
                            _ => "rgba(255,245,240,0.5)",
                        };
                        let orb_mode = if is_working { "thinking" } else { "dormant" };
                        view! {
                            <div style="background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 12px; padding: 14px 16px; min-width: 200px; flex: 1;">
                                <div style="display: flex; align-items: center; gap: 10px; margin-bottom: 8px;">
                                    <SentientOrb size=28 mode=orb_mode />
                                    <span style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 2px; color: rgba(255,245,240,0.9);">
                                        {if a.agent_id.is_empty() { a.id.clone() } else { a.agent_id[..12.min(a.agent_id.len())].to_string() }}
                                    </span>
                                    <div style={format!("width: 6px; height: 6px; border-radius: 50%; background: {}; box-shadow: 0 0 6px {};", status_color, status_color)} />
                                </div>
                                {(!a.current_task.is_empty()).then(|| view! {
                                    <div style="font-size: 11px; color: rgba(255,245,240,0.6); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; margin-bottom: 4px;">
                                        {a.current_task.clone()}
                                    </div>
                                })}
                                <div style="font-size: 10px; color: rgba(255,245,240,0.5);">{format!("{} iterations", a.iterations)}</div>
                            </div>
                        }
                    }).collect::<Vec<_>>()
                }}
            </div>

            // Tab bar
            <div style="display: flex; gap: 4px; margin-bottom: 20px; border-bottom: 1px solid rgba(255,60,20,0.1); padding-bottom: 8px;">
                {["create", "active", "history"].iter().map(|t| {
                    let t_str = t.to_string();
                    let t_c = t_str.clone();
                    let label = match *t {
                        "create" => "Create Workflow",
                        "active" => "Active Workflows",
                        "history" => "History",
                        _ => *t,
                    };
                    view! {
                        <button
                            on:click={let t = t_str.clone(); move |_| tab.set(t.clone())}
                            style=move || format!(
                                "font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; text-transform: uppercase; padding: 8px 16px; border-radius: 6px 6px 0 0; cursor: pointer; border: 1px solid {}; border-bottom: none; background: {}; color: {}; transition: all 0.2s;",
                                if tab.get() == t_c { "rgba(255,60,20,0.3)" } else { "rgba(255,60,20,0.1)" },
                                if tab.get() == t_c { "rgba(255,60,20,0.08)" } else { "transparent" },
                                if tab.get() == t_c { "rgba(255,140,80,1)" } else { "rgba(255,245,240,0.7)" },
                            )
                        >{label}</button>
                    }
                }).collect::<Vec<_>>()}
            </div>

            // ── CREATE TAB ──
            <Show when=move || tab.get() == "create">
                <Card style="margin-bottom: 24px;">
                    <div style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 3px; color: rgba(255,245,240,0.7); margin-bottom: 12px;">"MISSION OBJECTIVE"</div>
                    <textarea
                        prop:value=move || goal_input.get()
                        on:input=move |ev| goal_input.set(event_target_value(&ev))
                        rows=4
                        placeholder="Describe your goal in natural language...&#10;&#10;Examples:&#10;  \u{2022} Analyze all Rust files and generate a dependency graph&#10;  \u{2022} Research competitors and write a comparison report&#10;  \u{2022} Refactor the auth module to use JWT tokens"
                        style="width: 100%; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 8px; padding: 12px 16px; color: rgba(255,245,240,0.9); font-size: 14px; font-family: 'Rajdhani', sans-serif; resize: vertical; box-sizing: border-box; line-height: 1.6;"
                    />
                    <div style="display: flex; align-items: center; gap: 10px; margin-top: 12px;">
                        <Button on_click=Some(Callback::new(create_plan))>
                            <Icon name="search" size=12 />
                            {move || if planning.get() { " Analyzing..." } else { " Plan First" }}
                        </Button>
                        <Button primary=true on_click=Some(Callback::new(execute_plan))>
                            <Icon name="zap" size=12 />
                            {move || if executing.get() { " Launching..." } else { " Execute Now" }}
                        </Button>
                        {move || {
                            let msg = message.get();
                            (!msg.is_empty()).then(|| {
                                let is_err = msg.starts_with("Error:");
                                let color = if is_err { "rgba(239,68,68,0.8)" } else { "rgba(34,197,94,0.8)" };
                                view! {
                                    <span style={format!("font-size: 12px; color: {};", color)}>{msg}</span>
                                }
                            })
                        }}
                    </div>
                </Card>

                // Plan visualization
                {move || {
                    plan_result.get().map(|p| view! {
                        <Card style="margin-bottom: 16px;" glow=true>
                            <div style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 3px; color: rgba(255,60,20,0.6); margin-bottom: 16px;">"EXECUTION PLAN"</div>
                            <div style="display: flex; gap: 12px; margin-bottom: 16px; flex-wrap: wrap;">
                                <MetricCard label="NODES" value={p.nodes.to_string()} icon="cpu" />
                                <MetricCard label="PARALLEL GROUPS" value={p.parallel_groups.len().to_string()} icon="agents" />
                                <MetricCard label="CRITICAL PATH" value={p.critical_path.len().to_string()} icon="zap" />
                                <MetricCard label="EST. TIME" value={format!("{}ms", p.estimated_total_ms)} icon="activity" />
                            </div>
                            // DAG flow visualization
                            <div style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,245,240,0.5); margin-bottom: 10px;">"EXECUTION FLOW"</div>
                            <div style="display: flex; align-items: center; gap: 0; overflow-x: auto; padding: 8px 0;">
                                {p.parallel_groups.iter().enumerate().map(|(gi, group)| {
                                    let is_last = gi == p.parallel_groups.len() - 1;
                                    view! {
                                        <div style="display: flex; flex-direction: column; gap: 6px; align-items: center; min-width: 80px;">
                                            <div style="font-family: 'Orbitron', monospace; font-size: 8px; color: rgba(255,245,240,0.5); letter-spacing: 1px;">
                                                {format!("G{}", gi + 1)}
                                            </div>
                                            {group.iter().map(|node_id| {
                                                let is_critical = p.critical_path.contains(node_id);
                                                let border = if is_critical { "rgba(255,60,20,0.5)" } else { "rgba(255,60,20,0.15)" };
                                                let bg = if is_critical { "rgba(255,60,20,0.08)" } else { "rgba(255,255,255,0.03)" };
                                                view! {
                                                    <div style={format!("padding: 6px 12px; border: 1px solid {}; border-radius: 6px; background: {}; font-family: 'Orbitron', monospace; font-size: 10px; color: rgba(255,245,240,0.9);", border, bg)}>
                                                        {format!("Step {}", node_id)}
                                                    </div>
                                                }
                                            }).collect::<Vec<_>>()}
                                        </div>
                                        {(!is_last).then(|| view! {
                                            <div style="color: rgba(255,60,20,0.3); font-size: 18px; padding: 0 8px;">"→"</div>
                                        })}
                                    }
                                }).collect::<Vec<_>>()}
                            </div>
                            <div style="margin-top: 12px; font-size: 12px; color: rgba(255,245,240,0.6);">
                                {p.goal.clone()}
                            </div>
                        </Card>
                    })
                }}
            </Show>

            // ── ACTIVE WORKFLOWS TAB ──
            <Show when=move || tab.get() == "active">
                // Workflow detail overlay
                {move || {
                    selected_workflow.get().map(|detail| {
                        let total = detail.total_nodes as f64;
                        let completed = detail.completed_nodes as f64;
                        let failed = detail.failed_nodes as f64;
                        let progress = detail.progress_percentage;
                        let status_color = match detail.status.as_str() {
                            "running" => "#eab308",
                            "completed" => "#22c55e",
                            "failed" => "#ef4444",
                            _ => "rgba(255,245,240,0.7)",
                        };
                        view! {
                            <Card glow={detail.status == "running"} style="margin-bottom: 24px;">
                                <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 16px;">
                                    <div style="display: flex; align-items: center; gap: 12px;">
                                        <div style="font-family: 'Orbitron', monospace; font-size: 11px; letter-spacing: 3px; color: rgba(255,245,240,0.9);">"WORKFLOW DETAIL"</div>
                                        <Badge text={detail.status.clone()} color=status_color.to_string() />
                                    </div>
                                    <button
                                        on:click=close_detail
                                        style="background: none; border: 1px solid rgba(255,60,20,0.2); color: rgba(255,245,240,0.5); padding: 4px 10px; border-radius: 4px; cursor: pointer; font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 2px;"
                                    >"CLOSE"</button>
                                </div>

                                // Progress bar
                                <div style="margin-bottom: 16px;">
                                    <div style="display: flex; justify-content: space-between; margin-bottom: 6px;">
                                        <span style="font-size: 11px; color: rgba(255,245,240,0.7);">{detail.message.clone()}</span>
                                        <span style="font-family: 'Orbitron', monospace; font-size: 11px; color: rgba(255,245,240,0.5);">{format!("{:.0}%", progress)}</span>
                                    </div>
                                    <ProgressBar value=progress max=100.0 color=status_color.to_string() />
                                    <div style="display: flex; gap: 16px; margin-top: 6px; font-size: 10px; color: rgba(255,245,240,0.5);">
                                        <span>{format!("{}/{} completed", completed as u32, total as u32)}</span>
                                        {(failed > 0.0).then(|| view! {
                                            <span style="color: #ef4444;">{format!("{} failed", failed as u32)}</span>
                                        })}
                                    </div>
                                </div>

                                // Node list with status
                                <div style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,245,240,0.5); margin-bottom: 8px;">"EXECUTION NODES"</div>
                                <div style="display: flex; flex-direction: column; gap: 6px; max-height: 400px; overflow-y: auto;">
                                    {detail.nodes.iter().map(|node| {
                                        let (icon, node_color, node_class) = match node.status.as_str() {
                                            "completed" => ("\u{2713}", "#22c55e", ""),
                                            "running" => ("\u{25B6}", "#eab308", "zeus-node-running"),
                                            "failed" => ("\u{2717}", "#ef4444", ""),
                                            "pending" => ("\u{25CB}", "rgba(255,245,240,0.5)", ""),
                                            _ => ("\u{25CB}", "rgba(255,245,240,0.5)", ""),
                                        };
                                        let has_deps = !node.dependencies.is_empty();
                                        let has_error = node.error.is_some();
                                        view! {
                                            <div
                                                class=node_class
                                                style={format!("display: flex; align-items: flex-start; gap: 10px; padding: 10px 14px; background: rgba(255,255,255,0.02); border-radius: 6px; border-left: 3px solid {};", node_color)}
                                            >
                                                <span style={format!("color: {}; font-size: 14px; line-height: 1; flex-shrink: 0; margin-top: 2px;", node_color)}>{icon}</span>
                                                <div style="flex: 1; min-width: 0;">
                                                    <div style="display: flex; align-items: center; gap: 8px;">
                                                        <span style="font-family: 'Orbitron', monospace; font-size: 11px; color: rgba(255,245,240,0.9);">{node.node_id.clone()}</span>
                                                        <Badge text={node.status.clone()} color=node_color.to_string() />
                                                    </div>
                                                    {has_deps.then(|| view! {
                                                        <div style="font-size: 10px; color: rgba(255,245,240,0.5); margin-top: 4px;">
                                                            {"depends: "}{node.dependencies.join(", ")}
                                                        </div>
                                                    })}
                                                    {has_error.then(|| {
                                                        let err = node.error.clone().unwrap_or_default();
                                                        view! {
                                                            <div style="font-size: 10px; color: #ef4444; margin-top: 4px; font-family: monospace;">
                                                                {err}
                                                            </div>
                                                        }
                                                    })}
                                                </div>
                                                {node.completed_at.as_ref().map(|t| view! {
                                                    <span style="font-size: 9px; color: rgba(255,245,240,0.5); white-space: nowrap;">{t.clone()}</span>
                                                })}
                                            </div>
                                        }
                                    }).collect::<Vec<_>>()}
                                </div>
                            </Card>
                        }
                    })
                }}

                // Workflow list
                <Show when=move || selected_workflow.get().is_none()>
                    <div style="display: flex; flex-direction: column; gap: 10px;">
                        {move || {
                            let wfs = workflows.get();
                            let active: Vec<_> = wfs.into_iter().filter(|w| w.status == "running" || w.status == "pending").collect();
                            if active.is_empty() {
                                vec![view! {
                                    <div style="display: flex; flex-direction: column; align-items: center; justify-content: center; min-height: 200px; gap: 16px;">
                                        <SentientOrb size=80 mode="dormant" />
                                        <div style="font-family: 'Orbitron', monospace; font-size: 11px; letter-spacing: 3px; color: rgba(255,245,240,0.5);">
                                            "NO ACTIVE WORKFLOWS"
                                        </div>
                                        <div style="font-size: 12px; color: rgba(255,245,240,0.7); text-align: center; max-width: 400px;">
                                            "Create a workflow in the Create tab to start autonomous task execution."
                                        </div>
                                    </div>
                                }.into_any()]
                            } else {
                                active.into_iter().map(|w| {
                                    let wid = w.workflow_id.clone();
                                    let view_wf = view_workflow;
                                    let status_color = match w.status.as_str() {
                                        "running" => "#eab308",
                                        "completed" => "#22c55e",
                                        "failed" => "#ef4444",
                                        _ => "rgba(255,245,240,0.7)",
                                    };
                                    let is_running = w.status == "running";
                                    view! {
                                        <div
                                            on:click=move |_| view_wf(wid.clone())
                                            style="cursor: pointer; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 12px; padding: 16px 20px; transition: all 0.2s;"
                                        >
                                            <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 10px;">
                                                <div style="display: flex; align-items: center; gap: 10px;">
                                                    <SentientOrb size=28 mode={if is_running { "thinking" } else { "dormant" }} />
                                                    <span style="font-size: 13px; color: rgba(255,245,240,0.9); font-weight: 500;">
                                                        {w.message.clone()}
                                                    </span>
                                                </div>
                                                <Badge text={w.status.clone()} color=status_color.to_string() />
                                            </div>
                                            <ProgressBar value={w.progress_percentage} max=100.0 color=status_color.to_string() />
                                            <div style="display: flex; justify-content: space-between; margin-top: 6px; font-size: 10px; color: rgba(255,245,240,0.5);">
                                                <span>{format!("{}/{} nodes", w.completed_nodes, w.total_nodes)}</span>
                                                <span>{w.workflow_id[..8.min(w.workflow_id.len())].to_string()}</span>
                                            </div>
                                        </div>
                                    }.into_any()
                                }).collect::<Vec<_>>()
                            }
                        }}
                    </div>
                </Show>
            </Show>

            // ── HISTORY TAB ──
            <Show when=move || tab.get() == "history">
                <div style="display: flex; flex-direction: column; gap: 8px;">
                    {move || {
                        let wfs = workflows.get();
                        let completed: Vec<_> = wfs.into_iter().filter(|w| w.status == "completed" || w.status == "failed").collect();
                        if completed.is_empty() {
                            vec![view! {
                                <div style="text-align: center; padding: 48px; color: rgba(255,245,240,0.5); font-size: 12px;">
                                    "No completed workflows yet"
                                </div>
                            }.into_any()]
                        } else {
                            completed.into_iter().map(|w| {
                                let wid = w.workflow_id.clone();
                                let view_wf = view_workflow;
                                let status_color = if w.status == "completed" { "#22c55e" } else { "#ef4444" };
                                let icon = if w.status == "completed" { "\u{2713}" } else { "\u{2717}" };
                                view! {
                                    <div
                                        on:click=move |_| { view_wf(wid.clone()); tab.set("active".to_string()); }
                                        style="cursor: pointer; display: flex; align-items: center; gap: 12px; padding: 12px 16px; background: rgba(255,255,255,0.02); border-radius: 8px; border: 1px solid rgba(255,60,20,0.08); transition: all 0.2s;"
                                    >
                                        <span style={format!("color: {}; font-size: 16px;", status_color)}>{icon}</span>
                                        <div style="flex: 1;">
                                            <div style="font-size: 13px; color: rgba(255,245,240,0.9);">{w.message.clone()}</div>
                                            <div style="font-size: 10px; color: rgba(255,245,240,0.5);">
                                                {format!("{} nodes \u{2022} {}", w.total_nodes, w.created_at)}
                                            </div>
                                        </div>
                                        <Badge text={w.status.clone()} color=status_color.to_string() />
                                    </div>
                                }.into_any()
                            }).collect::<Vec<_>>()
                        }
                    }}
                </div>
            </Show>
        </div>
    }
}
