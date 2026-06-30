// ═══════════════════════════════════════════════════════════
// ZEUS — Pipeline Page — Phase 4: Autonomous Task Completions
// Mission queue, orchestration controls, approval mgmt, cron jobs
// ═══════════════════════════════════════════════════════════

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;
use crate::api;
use crate::components::design::*;
use crate::components::sentient_orb::SentientOrb;

#[component]
pub fn PipelinePage() -> impl IntoView {
    let pipeline = RwSignal::new(api::PipelineStatsResponse::default());
    let activity = RwSignal::new(Vec::<api::ActivityEvent>::new());
    let active_tasks = RwSignal::new(api::ObservatoryActiveTasks::default());
    let cron_jobs = RwSignal::new(api::CronJobsResponse::default());
    let cron_templates = RwSignal::new(Vec::<api::CronTemplate>::new());
    let loading = RwSignal::new(true);
    let tab = RwSignal::new("queue".to_string()); // queue | cron | activity

    // Approval action feedback
    let approval_msg = RwSignal::new(String::new());

    // Cron creation
    let show_cron = RwSignal::new(false);
    let cron_name = RwSignal::new(String::new());
    let cron_expr = RwSignal::new("0 * * * *".to_string());
    let cron_task = RwSignal::new(String::new());
    let cron_saving = RwSignal::new(false);

    // Fetch all data on mount
    {
        spawn_local(async move {
            let (p, a, t, c, ct) = (
                api::fetch_pipeline_stats().await,
                api::fetch_activity().await,
                api::fetch_observatory_active_tasks().await,
                api::fetch_cron_jobs().await,
                api::fetch_cron_templates().await,
            );
            if let Ok(p) = p { pipeline.set(p); }
            if let Ok(a) = a { activity.set(a.events); }
            if let Ok(t) = t { active_tasks.set(t); }
            if let Ok(c) = c { cron_jobs.set(c); }
            if let Ok(ct) = ct { cron_templates.set(ct.templates); }
            loading.set(false);
        });
    }

    // Auto-poll every 5s
    {
        spawn_local(async move {
            loop {
                gloo_timers::future::TimeoutFuture::new(5000).await;
                if let Ok(t) = api::fetch_observatory_active_tasks().await {
                    let has_active = t.summary.workflows_active > 0 || t.summary.approvals_pending > 0;
                    active_tasks.set(t);
                    if let Ok(a) = api::fetch_activity().await { activity.set(a.events); }
                    if !has_active { break; }
                } else {
                    break;
                }
            }
        });
    }

    let refresh_all = move |_| {
        spawn_local(async move {
            if let Ok(p) = api::fetch_pipeline_stats().await { pipeline.set(p); }
            if let Ok(a) = api::fetch_activity().await { activity.set(a.events); }
            if let Ok(t) = api::fetch_observatory_active_tasks().await { active_tasks.set(t); }
            if let Ok(c) = api::fetch_cron_jobs().await { cron_jobs.set(c); }
        });
    };

    let approve_action = move |id: String| {
        approval_msg.set(String::new());
        spawn_local(async move {
            match api::approve_execution(&id).await {
                Ok(_) => {
                    approval_msg.set(format!("Approved: {}", &id[..8.min(id.len())]));
                    if let Ok(t) = api::fetch_observatory_active_tasks().await { active_tasks.set(t); }
                }
                Err(e) => approval_msg.set(format!("Error: {}", e)),
            }
        });
    };

    let deny_action = move |id: String| {
        approval_msg.set(String::new());
        spawn_local(async move {
            match api::deny_execution(&id, Some("Denied via Pipeline UI")).await {
                Ok(_) => {
                    approval_msg.set(format!("Denied: {}", &id[..8.min(id.len())]));
                    if let Ok(t) = api::fetch_observatory_active_tasks().await { active_tasks.set(t); }
                }
                Err(e) => approval_msg.set(format!("Error: {}", e)),
            }
        });
    };

    let create_cron = move |_| {
        let name = cron_name.get_untracked();
        let expr = cron_expr.get_untracked();
        let task = cron_task.get_untracked();
        if name.trim().is_empty() || task.trim().is_empty() { return; }
        cron_saving.set(true);
        spawn_local(async move {
            let body = serde_json::json!({
                "name": name,
                "cron": expr,
                "task_type": { "chat": { "message": task } }
            });
            let _ = api::create_cron_job(&body).await;
            if let Ok(c) = api::fetch_cron_jobs().await { cron_jobs.set(c); }
            show_cron.set(false);
            cron_name.set(String::new());
            cron_expr.set("0 * * * *".to_string());
            cron_task.set(String::new());
            cron_saving.set(false);
        });
    };

    let delete_cron_job = move |id: String| {
        spawn_local(async move {
            let _ = api::delete_cron_job(&id).await;
            if let Ok(c) = api::fetch_cron_jobs().await { cron_jobs.set(c); }
        });
    };

    let input_style = "width: 100%; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 8px; padding: 10px 14px; color: rgba(255,245,240,0.9); font-family: 'Rajdhani', sans-serif; font-size: 14px; box-sizing: border-box; outline: none;";
    let label_style = "font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,245,240,0.7); margin-bottom: 6px;";

    view! {
        // ── Cron Create Modal ────────────────────────────────
        <Show when=move || show_cron.get()>
            <div style="position: fixed; inset: 0; background: rgba(0,0,0,0.78); z-index: 1000; display: flex; align-items: center; justify-content: center;">
                <div style="background: #0d0704; border: 1px solid rgba(255,60,20,0.25); border-radius: 16px; padding: 32px; width: 480px; max-width: 92vw; box-shadow: 0 0 60px rgba(255,60,20,0.15);">
                    <div style="font-family: 'Orbitron', monospace; font-size: 11px; letter-spacing: 5px; color: rgba(255,245,240,0.9); margin-bottom: 20px;">"CREATE SCHEDULED TASK"</div>
                    <div style="display: flex; flex-direction: column; gap: 14px;">
                        <div>
                            <div style=label_style>"NAME"</div>
                            <input type="text" placeholder="e.g. Daily health check" style=input_style
                                prop:value=move || cron_name.get()
                                on:input=move |ev| cron_name.set(event_target_value(&ev))
                            />
                        </div>
                        <div>
                            <div style=label_style>"CRON EXPRESSION"</div>
                            <input type="text" placeholder="0 * * * *" style=input_style
                                prop:value=move || cron_expr.get()
                                on:input=move |ev| cron_expr.set(event_target_value(&ev))
                            />
                            <div style="font-size: 10px; color: rgba(255,245,240,0.2); margin-top: 4px;">"min hour day month weekday (e.g. '0 9 * * 1-5' = 9am weekdays)"</div>
                        </div>
                        <div>
                            <div style=label_style>"TASK MESSAGE"</div>
                            <textarea rows=3 placeholder="What should Zeus do on this schedule?" style="width: 100%; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 8px; padding: 10px 14px; color: rgba(255,245,240,0.9); font-family: 'Rajdhani', sans-serif; font-size: 13px; box-sizing: border-box; outline: none; resize: vertical;"
                                prop:value=move || cron_task.get()
                                on:input=move |ev| cron_task.set(event_target_value(&ev))
                            />
                        </div>

                        // Quick templates
                        {move || {
                            let templates = cron_templates.get();
                            if templates.is_empty() { return view! { <div /> }.into_any(); }
                            view! {
                                <div>
                                    <div style=label_style>"TEMPLATES"</div>
                                    <div style="display: flex; flex-wrap: wrap; gap: 4px;">
                                        {templates.into_iter().map(|t| {
                                            let tname = t.name.clone();
                                            let tcron = t.cron.clone();
                                            view! {
                                                <button
                                                    on:click=move |_| {
                                                        cron_name.set(tname.clone());
                                                        cron_expr.set(tcron.clone());
                                                    }
                                                    style="font-family: 'Rajdhani', sans-serif; font-size: 10px; padding: 3px 10px; border-radius: 4px; cursor: pointer; background: rgba(255,60,20,0.06); border: 1px solid rgba(255,60,20,0.12); color: rgba(255,140,80,0.7);"
                                                >{t.name.clone()}</button>
                                            }
                                        }).collect::<Vec<_>>()}
                                    </div>
                                </div>
                            }.into_any()
                        }}
                    </div>
                    <div style="display: flex; gap: 10px; margin-top: 20px; justify-content: flex-end;">
                        <Button on_click=Some(Callback::new(move |_| show_cron.set(false)))>"Cancel"</Button>
                        <Button primary=true on_click=Some(Callback::new(create_cron))>
                            {move || if cron_saving.get() { "Creating..." } else { "Create" }}
                        </Button>
                    </div>
                </div>
            </div>
        </Show>

        <div style="padding: 32px;">
            // ── Header ──
            <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 24px;">
                <div style="display: flex; align-items: center; gap: 16px;">
                    <SentientOrb size=48 mode="active" />
                    <div>
                        <h1 style="font-family: 'Orbitron', monospace; font-size: 14px; letter-spacing: 6px; color: rgba(255,245,240,0.9); margin: 0;">"PIPELINE"</h1>
                        <p style="font-size: 12px; color: rgba(255,245,240,0.7); margin: 4px 0 0;">
                            {move || {
                                let p = pipeline.get();
                                let t = active_tasks.get();
                                format!("{} processed \u{2022} {} active workflows \u{2022} {} approvals pending",
                                    p.total_messages, t.summary.workflows_active, t.summary.approvals_pending)
                            }}
                        </p>
                    </div>
                </div>
                <div style="display: flex; gap: 8px;">
                    <Button on_click=Some(Callback::new(move |_| show_cron.set(true)))>
                        <Icon name="plus" size=12 /> " Schedule"
                    </Button>
                    <Button on_click=Some(Callback::new(refresh_all))>
                        <Icon name="activity" size=12 /> " Refresh"
                    </Button>
                </div>
            </div>

            // ── Pipeline stage metrics ──
            <div style="display: flex; gap: 12px; margin-bottom: 24px; flex-wrap: wrap;">
                {move || {
                    pipeline.get().stages.into_iter().map(|stage| {
                        let latency_color = if stage.avg_latency_ms < 100 { "#22c55e" }
                            else if stage.avg_latency_ms < 500 { "#eab308" }
                            else { "#ef4444" };
                        view! {
                            <div style="flex: 1; min-width: 160px; padding: 16px; background: rgba(255,255,255,0.02); border: 1px solid rgba(255,60,20,0.08); border-radius: 12px;">
                                <div style="font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 2px; color: rgba(255,245,240,0.3); margin-bottom: 8px; text-transform: uppercase;">
                                    {stage.name.clone()}
                                </div>
                                <div style="font-size: 22px; font-weight: 700; color: rgba(255,245,240,0.95); margin-bottom: 6px;">
                                    {stage.messages_processed.to_string()}
                                </div>
                                <div style="display: flex; gap: 12px; font-size: 10px;">
                                    <span style={format!("color: {};", latency_color)}>{format!("{}ms", stage.avg_latency_ms)}</span>
                                    {(stage.error_count > 0).then(|| view! {
                                        <span style="color: #ef4444;">{format!("{} err", stage.error_count)}</span>
                                    })}
                                </div>
                            </div>
                        }
                    }).collect::<Vec<_>>()
                }}
            </div>

            // ── Tab bar ──
            <div style="display: flex; gap: 4px; margin-bottom: 20px; border-bottom: 1px solid rgba(255,60,20,0.1); padding-bottom: 8px;">
                {["queue", "cron", "activity"].iter().map(|t| {
                    let t_str = t.to_string();
                    let t_c = t_str.clone();
                    let label = match *t {
                        "queue" => "Mission Queue",
                        "cron" => "Scheduled Tasks",
                        "activity" => "Event Stream",
                        _ => *t,
                    };
                    // Count badges
                    let badge = match *t {
                        "queue" => Some(move || {
                            let tasks = active_tasks.get();
                            tasks.summary.workflows_active + tasks.summary.approvals_pending
                        }),
                        _ => None,
                    };
                    view! {
                        <button
                            on:click={let t = t_str.clone(); move |_| tab.set(t.clone())}
                            style=move || format!(
                                "font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; text-transform: uppercase; padding: 8px 16px; border-radius: 6px 6px 0 0; cursor: pointer; border: 1px solid {}; border-bottom: none; background: {}; color: {}; transition: all 0.2s; display: flex; align-items: center; gap: 6px;",
                                if tab.get() == t_c { "rgba(255,60,20,0.3)" } else { "rgba(255,60,20,0.1)" },
                                if tab.get() == t_c { "rgba(255,60,20,0.08)" } else { "transparent" },
                                if tab.get() == t_c { "rgba(255,140,80,1)" } else { "rgba(255,245,240,0.7)" },
                            )
                        >
                            {label}
                            {badge.map(|count_fn| view! {
                                <span style="font-size: 8px; background: rgba(255,60,20,0.3); padding: 1px 6px; border-radius: 10px; color: rgba(255,245,240,0.9);">
                                    {move || count_fn().to_string()}
                                </span>
                            })}
                        </button>
                    }
                }).collect::<Vec<_>>()}
            </div>

            // ══════════════ QUEUE TAB ══════════════
            <Show when=move || tab.get() == "queue">
                // Approval feedback
                <Show when=move || !approval_msg.get().is_empty()>
                    <div style="margin-bottom: 12px; padding: 8px 14px; border-radius: 8px; background: rgba(34,197,94,0.08); border: 1px solid rgba(34,197,94,0.2); font-size: 12px; color: rgba(34,197,94,0.8);">
                        {move || approval_msg.get()}
                    </div>
                </Show>

                // Pending Approvals
                {move || {
                    let approvals = active_tasks.get().pending_approvals;
                    if approvals.is_empty() { return view! { <div /> }.into_any(); }
                    view! {
                        <div style="margin-bottom: 20px;">
                            <div style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 3px; color: rgba(234,179,8,0.7); margin-bottom: 10px;">
                                {format!("PENDING APPROVALS ({})", approvals.len())}
                            </div>
                            <div style="display: flex; flex-direction: column; gap: 8px;">
                                {approvals.into_iter().map(|ap| {
                                    let ap_id = ap.id.clone();
                                    let ap_id2 = ap.id.clone();
                                    let approve = approve_action;
                                    let deny = deny_action;
                                    view! {
                                        <div style="display: flex; align-items: center; gap: 12px; padding: 14px 18px; background: rgba(234,179,8,0.04); border: 1px solid rgba(234,179,8,0.15); border-radius: 12px;">
                                            <div style="width: 36px; height: 36px; border-radius: 8px; background: rgba(234,179,8,0.1); display: flex; align-items: center; justify-content: center; flex-shrink: 0;">
                                                <Icon name="security" size=18 color="rgba(234,179,8,0.7)".to_string() />
                                            </div>
                                            <div style="flex: 1; min-width: 0;">
                                                <div style="font-size: 14px; color: rgba(255,245,240,0.95); font-weight: 500;">{format!("Tool: {}", ap.tool)}</div>
                                                <div style="font-size: 10px; color: rgba(255,245,240,0.35); margin-top: 2px;">
                                                    {format!("Requested: {} \u{2022} ID: {}", ap.requested_at, &ap.id[..8.min(ap.id.len())])}
                                                </div>
                                            </div>
                                            <div style="display: flex; gap: 6px; flex-shrink: 0;">
                                                <button
                                                    on:click=move |_| approve(ap_id.clone())
                                                    style="font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 1px; padding: 6px 14px; border-radius: 6px; cursor: pointer; background: rgba(34,197,94,0.15); border: 1px solid rgba(34,197,94,0.4); color: rgba(34,197,94,0.9);"
                                                >"APPROVE"</button>
                                                <button
                                                    on:click=move |_| deny(ap_id2.clone())
                                                    style="font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 1px; padding: 6px 14px; border-radius: 6px; cursor: pointer; background: rgba(239,68,68,0.08); border: 1px solid rgba(239,68,68,0.2); color: rgba(239,68,68,0.7);"
                                                >"DENY"</button>
                                            </div>
                                        </div>
                                    }
                                }).collect::<Vec<_>>()}
                            </div>
                        </div>
                    }.into_any()
                }}

                // Active Workflows
                {move || {
                    let workflows = active_tasks.get().workflows;
                    view! {
                        <div style="margin-bottom: 20px;">
                            <div style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 3px; color: rgba(255,245,240,0.35); margin-bottom: 10px;">
                                {format!("ACTIVE WORKFLOWS ({})", workflows.len())}
                            </div>
                            {if workflows.is_empty() {
                                view! {
                                    <div style="display: flex; flex-direction: column; align-items: center; padding: 48px; gap: 16px;">
                                        <SentientOrb size=64 mode="dormant" />
                                        <div style="font-family: 'Orbitron', monospace; font-size: 11px; letter-spacing: 3px; color: rgba(255,245,240,0.5);">"NO ACTIVE MISSIONS"</div>
                                        <div style="font-size: 12px; color: rgba(255,245,240,0.4); text-align: center; max-width: 400px;">
                                            "Dispatch tasks from the Agents page or create workflows from Prometheus to see them here."
                                        </div>
                                    </div>
                                }.into_any()
                            } else {
                                view! {
                                    <div style="display: flex; flex-direction: column; gap: 10px;">
                                        {workflows.into_iter().map(|wf| {
                                            let status_color = match wf.status.as_str() {
                                                "running" => "#eab308",
                                                "completed" => "#22c55e",
                                                "failed" => "#ef4444",
                                                "pending" => "rgba(255,245,240,0.7)",
                                                _ => "rgba(255,245,240,0.5)",
                                            };
                                            let is_running = wf.status == "running";
                                            view! {
                                                <div style="background: rgba(255,255,255,0.02); border: 1px solid rgba(255,60,20,0.1); border-radius: 12px; padding: 16px 20px;">
                                                    <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 10px;">
                                                        <div style="display: flex; align-items: center; gap: 10px;">
                                                            <SentientOrb size=28 mode={if is_running { "thinking" } else { "dormant" }} />
                                                            <div>
                                                                <div style="font-size: 13px; color: rgba(255,245,240,0.95); font-weight: 500;">{wf.message.clone()}</div>
                                                                <div style="font-size: 10px; color: rgba(255,245,240,0.3); margin-top: 2px;">
                                                                    {format!("{} \u{2022} {}", &wf.workflow_id[..8.min(wf.workflow_id.len())], wf.created_at)}
                                                                </div>
                                                            </div>
                                                        </div>
                                                        <Badge text={wf.status.clone()} color=status_color.to_string() />
                                                    </div>
                                                    <ProgressBar value={wf.progress_pct} max=100.0 color=status_color.to_string() />
                                                    <div style="display: flex; justify-content: space-between; margin-top: 6px; font-size: 10px; color: rgba(255,245,240,0.5);">
                                                        <span>{format!("{}/{} nodes completed", wf.completed_nodes, wf.total_nodes)}</span>
                                                        {(wf.failed_nodes > 0).then(|| view! {
                                                            <span style="color: #ef4444;">{format!("{} failed", wf.failed_nodes)}</span>
                                                        })}
                                                        <span>{format!("{:.0}%", wf.progress_pct)}</span>
                                                    </div>
                                                </div>
                                            }
                                        }).collect::<Vec<_>>()}
                                    </div>
                                }.into_any()
                            }}
                        </div>
                    }
                }}

                // Cron Tasks summary
                {move || {
                    let tasks = active_tasks.get().cron_tasks;
                    if tasks.is_empty() { return view! { <div /> }.into_any(); }
                    view! {
                        <div>
                            <div style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 3px; color: rgba(255,245,240,0.35); margin-bottom: 10px;">
                                {format!("SCHEDULED RUNS ({})", tasks.len())}
                            </div>
                            <div style="display: flex; flex-direction: column; gap: 6px;">
                                {tasks.into_iter().map(|ct| {
                                    let enabled_color = if ct.enabled { "#22c55e" } else { "rgba(255,245,240,0.5)" };
                                    view! {
                                        <div style="display: flex; align-items: center; gap: 10px; padding: 10px 14px; background: rgba(255,255,255,0.015); border-radius: 8px; border: 1px solid rgba(255,60,20,0.06);">
                                            <div style={format!("width: 8px; height: 8px; border-radius: 50%; background: {};", enabled_color)} />
                                            <div style="flex: 1;">
                                                <div style="font-size: 12px; color: rgba(255,245,240,0.9);">{ct.name.clone()}</div>
                                                <div style="font-size: 10px; color: rgba(255,245,240,0.3);">
                                                    {format!("{} \u{2022} {}", ct.cron_expr, ct.task_type)}
                                                </div>
                                            </div>
                                            <div style="text-align: right; font-size: 10px; color: rgba(255,245,240,0.5);">
                                                {ct.next_run.clone().unwrap_or_else(|| "\u{2014}".to_string())}
                                            </div>
                                        </div>
                                    }
                                }).collect::<Vec<_>>()}
                            </div>
                        </div>
                    }.into_any()
                }}
            </Show>

            // ══════════════ CRON TAB ══════════════
            <Show when=move || tab.get() == "cron">
                <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 16px;">
                    <div style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 3px; color: rgba(255,245,240,0.35);">
                        {move || format!("SCHEDULED TASKS ({})", cron_jobs.get().count)}
                    </div>
                    <Button primary=true on_click=Some(Callback::new(move |_| show_cron.set(true)))>
                        <Icon name="plus" size=12 /> " New Schedule"
                    </Button>
                </div>

                <div style="display: flex; flex-direction: column; gap: 10px;">
                    {move || {
                        let jobs = cron_jobs.get().jobs;
                        if jobs.is_empty() {
                            vec![view! {
                                <div style="display: flex; flex-direction: column; align-items: center; padding: 48px; gap: 12px;">
                                    <Icon name="activity" size=32 color="rgba(255,245,240,0.15)".to_string() />
                                    <div style="font-family: 'Orbitron', monospace; font-size: 11px; letter-spacing: 3px; color: rgba(255,245,240,0.5);">"NO SCHEDULED TASKS"</div>
                                    <div style="font-size: 12px; color: rgba(255,245,240,0.4); text-align: center; max-width: 360px;">
                                        "Schedule recurring tasks like health checks, reports, or automated workflows."
                                    </div>
                                </div>
                            }.into_any()]
                        } else {
                            jobs.into_iter().map(|job| {
                                let id = job.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                let name = job.get("name").and_then(|v| v.as_str()).unwrap_or("Unnamed").to_string();
                                let cron = job.get("cron").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                let enabled = job.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false);
                                let last_run = job.get("last_run").and_then(|v| v.as_str()).unwrap_or("\u{2014}").to_string();
                                let next_run = job.get("next_run").and_then(|v| v.as_str()).unwrap_or("\u{2014}").to_string();
                                let del_id = id.clone();
                                let del_fn = delete_cron_job;
                                view! {
                                    <Card>
                                        <div style="display: flex; align-items: center; gap: 12px;">
                                            <div style={format!("width: 10px; height: 10px; border-radius: 50%; background: {}; flex-shrink: 0;",
                                                if enabled { "#22c55e" } else { "rgba(255,245,240,0.15)" })} />
                                            <div style="flex: 1;">
                                                <div style="display: flex; align-items: center; gap: 8px;">
                                                    <span style="font-size: 14px; color: rgba(255,245,240,0.95); font-weight: 500;">{name}</span>
                                                    <span style="font-family: monospace; font-size: 11px; padding: 2px 8px; border-radius: 4px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.08); color: rgba(255,245,240,0.6);">{cron}</span>
                                                </div>
                                                <div style="display: flex; gap: 16px; margin-top: 6px; font-size: 10px; color: rgba(255,245,240,0.3);">
                                                    <span>{format!("Last: {}", last_run)}</span>
                                                    <span>{format!("Next: {}", next_run)}</span>
                                                </div>
                                            </div>
                                            <button
                                                on:click=move |_| del_fn(del_id.clone())
                                                style="font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 1px; padding: 5px 10px; border-radius: 5px; cursor: pointer; background: rgba(239,68,68,0.06); border: 1px solid rgba(239,68,68,0.12); color: rgba(239,68,68,0.5);"
                                            >"DELETE"</button>
                                        </div>
                                    </Card>
                                }.into_any()
                            }).collect::<Vec<_>>()
                        }
                    }}
                </div>
            </Show>

            // ══════════════ ACTIVITY TAB ══════════════
            <Show when=move || tab.get() == "activity">
                <div style="display: flex; flex-direction: column; gap: 2px; max-height: 600px; overflow-y: auto;">
                    {move || {
                        let events = activity.get();
                        if events.is_empty() {
                            vec![view! {
                                <div style="text-align: center; padding: 48px; color: rgba(255,245,240,0.5); font-size: 12px;">"No recent activity"</div>
                            }.into_any()]
                        } else {
                            events.into_iter().map(|ev| {
                                let icon = match ev.event_type.as_str() {
                                    "error" => "security",
                                    "tool" => "tools",
                                    "chat" | "message" => "chat",
                                    "session" => "sessions",
                                    "workflow" => "activity",
                                    "approval" => "security",
                                    _ => "activity",
                                };
                                let color = match ev.event_type.as_str() {
                                    "error" => "rgba(239,68,68,0.7)",
                                    "tool" => "rgba(59,130,246,0.7)",
                                    "chat" | "message" => "rgba(34,197,94,0.7)",
                                    "approval" => "rgba(234,179,8,0.7)",
                                    _ => "rgba(255,60,20,0.5)",
                                };
                                view! {
                                    <div style="display: flex; align-items: flex-start; gap: 10px; padding: 10px 12px; border-radius: 6px;">
                                        <div style={format!("width: 28px; height: 28px; border-radius: 6px; background: {}; display: flex; align-items: center; justify-content: center; flex-shrink: 0;",
                                            color.replace("0.7", "0.12")
                                        )}>
                                            <Icon name=icon size=14 color={color.to_string()} />
                                        </div>
                                        <div style="flex: 1; min-width: 0;">
                                            <div style="font-size: 13px; color: rgba(255,245,240,0.9);">{ev.summary.clone()}</div>
                                            {(!ev.details.is_empty()).then(|| view! {
                                                <div style="font-size: 11px; color: rgba(255,245,240,0.7); margin-top: 2px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;">
                                                    {ev.details.clone()}
                                                </div>
                                            })}
                                        </div>
                                        <div style="display: flex; flex-direction: column; align-items: flex-end; gap: 2px; flex-shrink: 0;">
                                            <Badge text={ev.event_type.clone()} color={color.to_string()} />
                                            <span style="font-size: 10px; color: rgba(255,245,240,0.3);">{ev.timestamp.clone()}</span>
                                        </div>
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
