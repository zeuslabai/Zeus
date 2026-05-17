// ═══════════════════════════════════════════════════════════
// ZEUS — Observatory Page — Phase 7: + Intelligence Insights
// ═══════════════════════════════════════════════════════════

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;
use crate::api;
use crate::components::design::*;

const OBSERVATORY_POLL_MS: i32 = 10_000;

#[component]
pub fn ObservatoryPage() -> impl IntoView {
    let activity = RwSignal::new(Vec::<api::ActivityEvent>::new());
    let active_tasks = RwSignal::new(api::ObservatoryActiveTasks::default());
    let agent_stats = RwSignal::new(api::ObservatoryAgentStats::default());
    let channel_health = RwSignal::new(api::ObservatoryChannelHealth::default());
    let cost_live = RwSignal::new(api::ObservatoryCostLive::default());
    let loading = RwSignal::new(true);
    // Phase 7: intelligence derived from activity
    let tool_freq: RwSignal<Vec<(String, usize)>> = RwSignal::new(Vec::new());
    let session_insights: RwSignal<Vec<String>> = RwSignal::new(Vec::new());

    {
        spawn_local(async move {
            let (a, t, ag, ch, co) = (
                api::fetch_activity().await,
                api::fetch_observatory_active_tasks().await,
                api::fetch_observatory_agent_stats().await,
                api::fetch_observatory_channel_health().await,
                api::fetch_observatory_cost_live().await,
            );
            if let Ok(ref a) = a {
                // Phase 7: derive tool frequency from activity for intelligence insights
                let mut freq: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
                for ev in &a.events {
                    if ev.event_type == "tool" && !ev.summary.is_empty() {
                        let tool = ev.summary.split_whitespace().next().unwrap_or("unknown").to_string();
                        *freq.entry(tool).or_insert(0) += 1;
                    }
                }
                let mut freq_vec: Vec<(String, usize)> = freq.into_iter().collect();
                freq_vec.sort_by(|a, b| b.1.cmp(&a.1));
                freq_vec.truncate(8);
                tool_freq.set(freq_vec);

                // Generate plain-language insights
                let events = &a.events;
                let mut insights = Vec::new();
                let error_count = events.iter().filter(|e| e.event_type == "error").count();
                let tool_count = events.iter().filter(|e| e.event_type == "tool").count();
                let chat_count = events.iter().filter(|e| e.event_type == "chat" || e.event_type == "message").count();
                if tool_count > 0 { insights.push(format!("{} tool executions in recent history", tool_count)); }
                if chat_count > 0 { insights.push(format!("{} conversation turns logged", chat_count)); }
                if error_count > 0 { insights.push(format!("{} errors detected — review security log", error_count)); }
                else { insights.push("No errors in recent activity — system healthy".to_string()); }
                if tool_count > chat_count * 2 { insights.push("High tool-to-chat ratio — autonomous mode active".to_string()); }
                session_insights.set(insights);
            }
            if let Ok(a) = a { activity.set(a.events); }
            if let Ok(t) = t { active_tasks.set(t); }
            if let Ok(ag) = ag { agent_stats.set(ag); }
            if let Ok(ch) = ch { channel_health.set(ch); }
            if let Ok(co) = co { cost_live.set(co); }
            loading.set(false);
        });
    }

    // ── 10s polling loop ─────────────────────────────────────
    {
        let cb = Closure::wrap(Box::new(move || {
            spawn_local(async move {
                let (a, t, ag, ch, co) = (
                    api::fetch_activity().await,
                    api::fetch_observatory_active_tasks().await,
                    api::fetch_observatory_agent_stats().await,
                    api::fetch_observatory_channel_health().await,
                    api::fetch_observatory_cost_live().await,
                );
                if let Ok(ref a) = a {
                    let mut freq: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
                    for ev in &a.events {
                        if ev.event_type == "tool" && !ev.summary.is_empty() {
                            let tool = ev.summary.split_whitespace().next().unwrap_or("unknown").to_string();
                            *freq.entry(tool).or_insert(0) += 1;
                        }
                    }
                    let mut freq_vec: Vec<(String, usize)> = freq.into_iter().collect();
                    freq_vec.sort_by(|a, b| b.1.cmp(&a.1));
                    freq_vec.truncate(8);
                    tool_freq.set(freq_vec);

                    let events = &a.events;
                    let mut insights = Vec::new();
                    let error_count = events.iter().filter(|e| e.event_type == "error").count();
                    let tool_count = events.iter().filter(|e| e.event_type == "tool").count();
                    let chat_count = events.iter().filter(|e| e.event_type == "chat" || e.event_type == "message").count();
                    if tool_count > 0 { insights.push(format!("{} tool executions in recent history", tool_count)); }
                    if chat_count > 0 { insights.push(format!("{} conversation turns logged", chat_count)); }
                    if error_count > 0 { insights.push(format!("{} errors detected — review security log", error_count)); }
                    else { insights.push("No errors in recent activity — system healthy".to_string()); }
                    if tool_count > chat_count * 2 { insights.push("High tool-to-chat ratio — autonomous mode active".to_string()); }
                    session_insights.set(insights);
                }
                if let Ok(a) = a { activity.set(a.events); }
                if let Ok(t) = t { active_tasks.set(t); }
                if let Ok(ag) = ag { agent_stats.set(ag); }
                if let Ok(ch) = ch { channel_health.set(ch); }
                if let Ok(co) = co { cost_live.set(co); }
            });
        }) as Box<dyn Fn()>);

        if let Some(win) = web_sys::window() {
            let handle = win.set_interval_with_callback_and_timeout_and_arguments_0(
                cb.as_ref().unchecked_ref(),
                OBSERVATORY_POLL_MS,
            ).unwrap_or(-1);
            cb.forget();
            on_cleanup(move || {
                if let Some(w) = web_sys::window() { w.clear_interval_with_handle(handle); }
            });
        }
    }

    view! {
        <div style="padding: 32px;">
            <div style="margin-bottom: 24px;">
                <h1 style="font-family: 'Orbitron', monospace; font-size: 14px; letter-spacing: 6px; color: rgba(255,245,240,0.9); margin: 0;">"OBSERVATORY"</h1>
                <p style="font-size: 12px; color: rgba(255,245,240,0.7); margin: 4px 0 0;">
                    {move || if loading.get() { "Loading live data...".to_string() } else { "Real-time system overview".to_string() }}
                </p>
            </div>

            // ── Summary Metrics ──
            <div style="display: grid; grid-template-columns: repeat(5, 1fr); gap: 12px; margin-bottom: 24px;">
                {move || {
                    let t = active_tasks.get();
                    let ag = agent_stats.get();
                    let ch = channel_health.get();
                    let co = cost_live.get();
                    view! {
                        <MetricCard label="Active Agents" value={ag.summary.total_agents.to_string()} icon="agents" />
                        <MetricCard label="Channels" value={format!("{}/{}", ch.summary.connected, ch.summary.total)} icon="channels" />
                        <MetricCard label="Workflows" value={format!("{}/{}", t.summary.workflows_active, t.summary.workflows_total)} icon="activity" />
                        <MetricCard label="Pending Approvals" value={t.summary.approvals_pending.to_string()} icon="security" />
                        <MetricCard label="Budget Left" value={format!("${:.2}", co.cost_summary.budget_remaining)} icon="analytics" />
                    }
                }}
            </div>

            <div style="display: grid; grid-template-columns: 1fr 1fr; gap: 16px; margin-bottom: 24px;">
                // ── Channel Health ──
                <Card>
                    <SectionTitle>"Channel Health"</SectionTitle>
                    <div style="display: flex; flex-direction: column; gap: 6px;">
                        {move || {
                            channel_health.get().channels.into_iter().map(|ch| {
                                let status = if ch.connected { "connected" } else { "disconnected" };
                                view! {
                                    <div style="display: flex; align-items: center; gap: 10px; padding: 10px 12px; background: rgba(255,255,255,0.03); border-radius: 8px; border: 1px solid rgba(255,60,20,0.1);">
                                        <StatusDot status={status.to_string()} />
                                        <div style="flex: 1;">
                                            <div style="font-size: 13px; color: rgba(255,245,240,0.9);">{ch.name.clone()}</div>
                                            <div style="font-size: 10px; color: rgba(255,245,240,0.5); font-family: 'Orbitron', monospace; letter-spacing: 1px;">{ch.channel_type.to_uppercase()}</div>
                                        </div>
                                        <div style="text-align: right;">
                                            <div style="font-family: 'Orbitron', monospace; font-size: 11px; color: rgba(255,245,240,0.7);">{format!("{:.0}%", ch.uptime_pct)}</div>
                                        </div>
                                    </div>
                                }
                            }).collect::<Vec<_>>()
                        }}
                    </div>
                </Card>

                // ── Agent Stats ──
                <Card>
                    <SectionTitle>"Active Agents"</SectionTitle>
                    <div style="display: flex; flex-direction: column; gap: 6px;">
                        {move || {
                            agent_stats.get().agents.into_iter().map(|ag| {
                                view! {
                                    <div style="display: flex; align-items: center; gap: 10px; padding: 10px 12px; background: rgba(255,255,255,0.03); border-radius: 8px; border: 1px solid rgba(255,60,20,0.1);">
                                        <StatusDot status="active".to_string() />
                                        <div style="flex: 1;">
                                            <div style="font-size: 13px; color: rgba(255,245,240,0.9);">{ag.name.clone()}</div>
                                            <div style="font-size: 10px; color: rgba(255,245,240,0.5);">{format!("{} messages", ag.message_count)}</div>
                                        </div>
                                        <Badge text={ag.agent_id.chars().take(8).collect::<String>()} color="#ff3c14".to_string() />
                                    </div>
                                }
                            }).collect::<Vec<_>>()
                        }}
                    </div>
                </Card>
            </div>

            <div style="display: grid; grid-template-columns: 1fr 1fr; gap: 16px; margin-bottom: 24px;">
                // ── Active Tasks ──
                <Card>
                    <SectionTitle>"Active Tasks"</SectionTitle>
                    <div style="display: flex; flex-direction: column; gap: 6px;">
                        {move || {
                            let t = active_tasks.get();
                            view! {
                                <div>
                                    {t.workflows.into_iter().map(|wf| {
                                        view! {
                                            <div style="display: flex; align-items: center; gap: 10px; padding: 10px 12px; margin-bottom: 6px; background: rgba(255,255,255,0.03); border-radius: 8px; border: 1px solid rgba(255,60,20,0.1);">
                                                <div style="width: 28px; height: 28px; border-radius: 6px; background: rgba(59,130,246,0.12); display: flex; align-items: center; justify-content: center;">
                                                    <Icon name="activity" size=14 color="rgba(59,130,246,0.7)".to_string() />
                                                </div>
                                                <div style="flex: 1; min-width: 0;">
                                                    <div style="font-size: 12px; color: rgba(255,245,240,0.9); white-space: nowrap; overflow: hidden; text-overflow: ellipsis;">{wf.message.clone()}</div>
                                                    <div style="font-size: 10px; color: rgba(255,245,240,0.5);">{format!("{}/{} nodes", wf.completed_nodes, wf.total_nodes)}</div>
                                                </div>
                                                <ProgressBar value={wf.progress_pct} max=100.0 color="rgba(59,130,246,0.7)".to_string() />
                                            </div>
                                        }
                                    }).collect::<Vec<_>>()}
                                    {t.pending_approvals.into_iter().map(|ap| {
                                        view! {
                                            <div style="display: flex; align-items: center; gap: 10px; padding: 10px 12px; margin-bottom: 6px; background: rgba(255,255,255,0.03); border-radius: 8px; border: 1px solid rgba(255,60,20,0.1);">
                                                <div style="width: 28px; height: 28px; border-radius: 6px; background: rgba(234,179,8,0.12); display: flex; align-items: center; justify-content: center;">
                                                    <Icon name="security" size=14 color="rgba(234,179,8,0.7)".to_string() />
                                                </div>
                                                <div style="flex: 1;">
                                                    <div style="font-size: 12px; color: rgba(255,245,240,0.9);">{format!("Approve: {}", ap.tool)}</div>
                                                </div>
                                                <Badge text="PENDING".to_string() color="#eab308".to_string() />
                                            </div>
                                        }
                                    }).collect::<Vec<_>>()}
                                </div>
                            }
                        }}
                    </div>
                </Card>

                // ── Cost Overview ──
                <Card>
                    <SectionTitle>"Cost Overview"</SectionTitle>
                    <div style="display: flex; flex-direction: column; gap: 10px;">
                        {move || {
                            let co = cost_live.get();
                            let budget = co.cost_summary.budget_remaining + co.cost_summary.total_cost;
                            let pct = if budget > 0.0 { (co.cost_summary.total_cost / budget) * 100.0 } else { 0.0 };
                            view! {
                                <div style="display: flex; justify-content: space-between; align-items: center; padding: 8px 0;">
                                    <span style="font-size: 12px; color: rgba(255,245,240,0.7);">"Total Spent"</span>
                                    <span style="font-family: 'Orbitron', monospace; font-size: 16px; color: rgba(255,245,240,0.9);">{format!("${:.4}", co.cost_summary.total_cost)}</span>
                                </div>
                                <ProgressBar value=pct max=100.0 color="#ff3c14".to_string() />
                                <div style="display: flex; flex-direction: column; gap: 4px; margin-top: 8px;">
                                    {co.cost_summary.top_models.into_iter().map(|m| {
                                        view! {
                                            <div style="display: flex; justify-content: space-between; padding: 4px 0;">
                                                <span style="font-family: 'Orbitron', monospace; font-size: 10px; color: rgba(255,245,240,0.7); letter-spacing: 1px;">{m.model}</span>
                                                <span style="font-family: 'Orbitron', monospace; font-size: 10px; color: rgba(255,245,240,0.9);">{format!("${:.4}", m.cost)}</span>
                                            </div>
                                        }
                                    }).collect::<Vec<_>>()}
                                </div>
                            }
                        }}
                    </div>
                </Card>
            </div>

            // ── Phase 7: Intelligence Layer ──
            <div style="display: grid; grid-template-columns: 1fr 1fr; gap: 16px; margin-bottom: 24px;">
                // ── Tool Frequency ──
                <Card>
                    <SectionTitle>"Top Tools"</SectionTitle>
                    <div style="display: flex; flex-direction: column; gap: 6px;">
                        {move || {
                            let freq = tool_freq.get();
                            if freq.is_empty() {
                                return view! { <div style="font-size: 12px; color: rgba(255,245,240,0.5); text-align: center; padding: 20px;">{"No tool data yet"}</div> }.into_any();
                            }
                            let max_count = freq.first().map(|(_, c)| *c).unwrap_or(1).max(1);
                            view! {
                                <div>
                                    {freq.into_iter().map(|(tool, count)| {
                                        let pct = (count as f64 / max_count as f64) * 100.0;
                                        view! {
                                            <div style="display: flex; align-items: center; gap: 8px; padding: 5px 0;">
                                                <div style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 1px; color: rgba(255,245,240,0.55); width: 120px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; flex-shrink: 0;">{tool.to_uppercase()}</div>
                                                <div style="flex: 1; height: 6px; background: rgba(255,255,255,0.05); border-radius: 3px; overflow: hidden;">
                                                    <div style={format!("height: 100%; width: {:.0}%; background: linear-gradient(90deg, rgba(59,130,246,0.7), rgba(59,130,246,0.4)); border-radius: 3px; transition: width 0.3s;", pct)}></div>
                                                </div>
                                                <div style="font-family: 'Orbitron', monospace; font-size: 9px; color: rgba(59,130,246,0.7); width: 24px; text-align: right; flex-shrink: 0;">{count.to_string()}</div>
                                            </div>
                                        }
                                    }).collect::<Vec<_>>()}
                                </div>
                            }.into_any()
                        }}
                    </div>
                </Card>

                // ── Session Insights ──
                <Card>
                    <SectionTitle>"Intelligence Insights"</SectionTitle>
                    <div style="display: flex; flex-direction: column; gap: 8px;">
                        {move || {
                            let insights = session_insights.get();
                            if insights.is_empty() {
                                return view! { <div style="font-size: 12px; color: rgba(255,245,240,0.5); text-align: center; padding: 20px;">{"Loading insights..."}</div> }.into_any();
                            }
                            view! {
                                <div>
                                    {insights.into_iter().map(|insight| {
                                        let (icon, color) = if insight.contains("error") || insight.contains("Error") {
                                            ("security", "rgba(239,68,68,0.7)")
                                        } else if insight.contains("autonomous") {
                                            ("activity", "rgba(168,85,247,0.7)")
                                        } else if insight.contains("healthy") {
                                            ("security", "rgba(34,197,94,0.7)")
                                        } else {
                                            ("analytics", "rgba(255,140,80,0.7)")
                                        };
                                        view! {
                                            <div style="display: flex; align-items: flex-start; gap: 10px; padding: 10px 12px; background: rgba(255,255,255,0.03); border-radius: 8px; border: 1px solid rgba(255,60,20,0.08);">
                                                <div style={format!("width: 26px; height: 26px; border-radius: 6px; background: {}18; display: flex; align-items: center; justify-content: center; flex-shrink: 0;", color)}>
                                                    <Icon name=icon size=13 color={color.to_string()} />
                                                </div>
                                                <div style="font-size: 12px; color: rgba(255,245,240,0.75); line-height: 1.5;">{insight}</div>
                                            </div>
                                        }
                                    }).collect::<Vec<_>>()}
                                </div>
                            }.into_any()
                        }}
                    </div>
                </Card>
            </div>

            // ── Event Stream ──
            <Card>
                <SectionTitle>"Event Stream"</SectionTitle>
                <div style="display: flex; flex-direction: column; gap: 2px; max-height: 400px; overflow-y: auto;">
                    {move || {
                        activity.get().into_iter().map(|ev| {
                            let icon = match ev.event_type.as_str() {
                                "error" => "security",
                                "tool" => "tools",
                                "chat" | "message" => "chat",
                                "session" => "sessions",
                                _ => "activity",
                            };
                            let color = match ev.event_type.as_str() {
                                "error" => "rgba(239,68,68,0.7)",
                                "tool" => "rgba(59,130,246,0.7)",
                                "chat" | "message" => "rgba(34,197,94,0.7)",
                                _ => "rgba(255,60,20,0.5)",
                            };
                            view! {
                                <div style="display: flex; align-items: flex-start; gap: 10px; padding: 10px 12px; border-radius: 6px; transition: background 0.2s;"
                                    class="zcard-hover"
                                >
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
                                        <span style="font-size: 10px; color: rgba(255,245,240,0.7);">{ev.timestamp.clone()}</span>
                                    </div>
                                </div>
                            }
                        }).collect::<Vec<_>>()
                    }}
                </div>
            </Card>
        </div>
    }
}
