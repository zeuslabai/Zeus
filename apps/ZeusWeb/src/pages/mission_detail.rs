// ═══════════════════════════════════════════════════════════
// ZEUS — Mission Detail Page — Session Replay + Stats + Tools
// ═══════════════════════════════════════════════════════════

use leptos::prelude::*;
use leptos_router::hooks::use_params_map;
use wasm_bindgen_futures::spawn_local;
use crate::api;
use crate::components::design::*;

#[component]
pub fn MissionDetailPage() -> impl IntoView {
    let params = use_params_map();
    let stats = RwSignal::new(api::SessionStatsDetail::default());
    let replay = RwSignal::new(Vec::<api::ReplayTurn>::new());
    let tools = RwSignal::new(Vec::<api::ToolExecution>::new());
    let audit = RwSignal::new(Vec::<api::AuditEntry>::new());
    let loading = RwSignal::new(true);
    let error = RwSignal::new(String::new());
    let active_tab = RwSignal::new("replay");
    let session_id = RwSignal::new(String::new());

    {
        let id = params.get_untracked().get("id").unwrap_or_default().to_string();
        session_id.set(id.clone());
        spawn_local(async move {
            if id.is_empty() {
                error.set("No session ID provided".to_string());
                loading.set(false);
                return;
            }
            let (st, rp, tl, au) = (
                api::fetch_session_stats(&id).await,
                api::fetch_session_replay(&id).await,
                api::fetch_session_tools(&id).await,
                api::fetch_session_audit(&id).await,
            );
            if let Ok(s) = st { stats.set(s); }
            if let Ok(r) = rp { replay.set(r); }
            if let Ok(t) = tl { tools.set(t.tools); }
            if let Ok(a) = au { audit.set(a.entries); }
            loading.set(false);
        });
    }

    view! {
        <div style="padding: 32px;">
            // Header
            <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 24px;">
                <div>
                    <h1 style="font-family: 'Orbitron', monospace; font-size: 14px; letter-spacing: 6px; font-weight: 600; color: rgba(255,245,240,0.9); margin: 0;">"MISSION DETAIL"</h1>
                    <p style="color: rgba(255,245,240,0.7); font-size: 12px;">
                        {move || {
                            let id = session_id.get();
                            if id.is_empty() { "No session".to_string() }
                            else { id[..16.min(id.len())].to_string() }
                        }}
                    </p>
                </div>
                <a href="/missions" style="text-decoration: none;">
                    <Button>"BACK"</Button>
                </a>
            </div>

            // Error state
            {move || {
                let e = error.get();
                (!e.is_empty()).then(|| view! {
                    <Card>
                        <div style="text-align: center; padding: 20px; color: #ef4444; font-size: 12px;">{e}</div>
                    </Card>
                })
            }}

            <Show when=move || !loading.get() && error.get().is_empty()>
                // Stats bar
                <div style="display: flex; gap: 12px; margin-bottom: 24px; flex-wrap: wrap;">
                    {move || {
                        let s = stats.get();
                        view! {
                            <MetricCard label="MESSAGES" value={s.message_count.to_string()} icon="message-square" />
                            <MetricCard label="USER" value={s.user_messages.to_string()} icon="user" />
                            <MetricCard label="ASSISTANT" value={s.assistant_messages.to_string()} icon="cpu" />
                            <MetricCard label="TOOL CALLS" value={s.tool_calls.to_string()} icon="wrench" />
                            <MetricCard label="DURATION" value={format!("{}s", s.duration_seconds)} icon="clock" />
                        }
                    }}
                </div>

                // Tab switcher
                <div style="display: flex; gap: 4px; margin-bottom: 16px;">
                    {["replay", "tools", "audit"].into_iter().map(|tab| {
                        let t = tab;
                        view! {
                            <button
                                on:click=move |_| active_tab.set(t)
                                style=move || format!(
                                    "padding: 6px 16px; border-radius: 4px; border: 1px solid {}; background: {}; color: {}; cursor: pointer; font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px;",
                                    if active_tab.get() == t { "rgba(255,60,20,0.3)" } else { "rgba(255,60,20,0.1)" },
                                    if active_tab.get() == t { "rgba(255,60,20,0.15)" } else { "transparent" },
                                    if active_tab.get() == t { "#ff3c14" } else { "rgba(255,245,240,0.7)" }
                                )
                            >{t.to_uppercase()}</button>
                        }
                    }).collect::<Vec<_>>()}
                </div>

                // Replay tab
                <Show when=move || active_tab.get() == "replay">
                    <div style="display: flex; flex-direction: column; gap: 8px;">
                        {move || {
                            let turns = replay.get();
                            if turns.is_empty() {
                                vec![view! {
                                    <Card>
                                        <div style="text-align: center; padding: 20px; color: rgba(255,245,240,0.5); font-size: 12px;">"No replay data recorded for this session."</div>
                                    </Card>
                                }.into_any()]
                            } else {
                                turns.into_iter().map(|turn| {
                                    let role_color = match turn.role.as_str() {
                                        "user" => "rgba(255,60,20,0.8)",
                                        "assistant" => "rgba(34,197,94,0.8)",
                                        "tool" => "rgba(234,179,8,0.8)",
                                        _ => "rgba(255,245,240,0.4)",
                                    };
                                    let role_bg = match turn.role.as_str() {
                                        "user" => "rgba(255,60,20,0.15)",
                                        "assistant" => "rgba(34,197,94,0.06)",
                                        "tool" => "rgba(234,179,8,0.06)",
                                        _ => "rgba(255,255,255,0.03)",
                                    };
                                    let has_tools = !turn.tool_name.is_empty();
                                    view! {
                                        <div style={format!("padding: 10px 14px; background: {}; border-radius: 6px; border-left: 3px solid {};", role_bg, role_color)}>
                                            <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 4px;">
                                                <div style="display: flex; align-items: center; gap: 8px;">
                                                    <span style={format!("font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: {};", role_color)}>{turn.role.to_uppercase()}</span>
                                                    {has_tools.then(|| view! {
                                                        <Badge text={turn.tool_name.clone()} color="#eab308".to_string() />
                                                    })}
                                                </div>
                                                <div style="display: flex; align-items: center; gap: 8px;">
                                                    {(turn.token_count > 0).then(|| view! {
                                                        <span style="font-size: 9px; color: rgba(255,245,240,0.2);">{format!("{}tok", turn.token_count)}</span>
                                                    })}
                                                    <span style="font-size: 9px; color: rgba(255,245,240,0.15);">{"#"}{turn.index.to_string()}</span>
                                                </div>
                                            </div>
                                            <div style="font-size: 12px; color: rgba(255,245,240,0.5); white-space: pre-wrap; word-break: break-word; max-height: 200px; overflow-y: auto;">
                                                {if turn.content.len() > 800 { format!("{}...", api::truncate_str(&turn.content, 800)) } else { turn.content.clone() }}
                                            </div>
                                            {(!turn.tool_results.is_empty()).then(|| view! {
                                                <div style="margin-top: 6px; padding: 6px 10px; background: rgba(255,255,255,0.03); border-radius: 4px; font-size: 10px; color: rgba(255,245,240,0.4); font-family: 'Orbitron', monospace; max-height: 100px; overflow-y: auto;">
                                                    {if turn.tool_results.len() > 500 { format!("{}...", api::truncate_str(&turn.tool_results, 500)) } else { turn.tool_results.clone() }}
                                                </div>
                                            })}
                                        </div>
                                    }.into_any()
                                }).collect::<Vec<_>>()
                            }
                        }}
                    </div>
                </Show>

                // Tools tab
                <Show when=move || active_tab.get() == "tools">
                    <div style="display: flex; flex-direction: column; gap: 4px;">
                        {move || {
                            let tool_list = tools.get();
                            if tool_list.is_empty() {
                                vec![view! {
                                    <Card>
                                        <div style="text-align: center; padding: 20px; color: rgba(255,245,240,0.5); font-size: 12px;">"No tool calls were made in this session."</div>
                                    </Card>
                                }.into_any()]
                            } else {
                                tool_list.into_iter().map(|t| {
                                    let status_color = if t.success { "#22c55e" } else { "#ef4444" };
                                    view! {
                                        <div style="display: flex; align-items: center; gap: 10px; padding: 8px 12px; background: rgba(255,255,255,0.03); border-radius: 4px;">
                                            <div style={format!("width: 6px; height: 6px; border-radius: 50%; background: {}; flex-shrink: 0;", status_color)} />
                                            <span style="font-family: 'Orbitron', monospace; font-size: 11px; color: rgba(255,245,240,0.9); min-width: 120px;">{t.name.clone()}</span>
                                            <span style="flex: 1; font-size: 11px; color: rgba(255,245,240,0.4); overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">
                                                {if t.output.len() > 120 { format!("{}...", api::truncate_str(&t.output, 120)) } else { t.output.clone() }}
                                            </span>
                                            {(t.duration_ms > 0).then(|| view! {
                                                <span style="font-size: 9px; color: rgba(255,245,240,0.2); white-space: nowrap;">{format!("{}ms", t.duration_ms)}</span>
                                            })}
                                            <span style="font-size: 9px; color: rgba(255,245,240,0.15); white-space: nowrap;">{t.timestamp.clone()}</span>
                                        </div>
                                    }.into_any()
                                }).collect::<Vec<_>>()
                            }
                        }}
                    </div>
                </Show>

                // Audit tab
                <Show when=move || active_tab.get() == "audit">
                    <div style="display: flex; flex-direction: column; gap: 4px;">
                        {move || {
                            let entries = audit.get();
                            if entries.is_empty() {
                                vec![view! {
                                    <Card>
                                        <div style="text-align: center; padding: 20px; color: rgba(255,245,240,0.5); font-size: 12px;">"No audit entries — this session ran without flags."</div>
                                    </Card>
                                }.into_any()]
                            } else {
                                entries.into_iter().map(|e| {
                                    let type_color = match e.entry_type.as_str() {
                                        "tool_call" => "#eab308",
                                        "memory_write" => "#3b82f6",
                                        "error" => "#ef4444",
                                        _ => "rgba(255,245,240,0.35)",
                                    };
                                    view! {
                                        <div style="display: flex; align-items: center; gap: 10px; padding: 8px 12px; background: rgba(255,255,255,0.03); border-radius: 4px;">
                                            <div style={format!("width: 6px; height: 6px; border-radius: 50%; background: {}; flex-shrink: 0;", type_color)} />
                                            <Badge text={e.entry_type.clone()} color=type_color.to_string() />
                                            {(!e.tool.is_empty()).then(|| view! {
                                                <span style="font-family: 'Orbitron', monospace; font-size: 10px; color: rgba(255,245,240,0.7);">{e.tool.clone()}</span>
                                            })}
                                            <span style="flex: 1; font-size: 11px; color: rgba(255,245,240,0.5); overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">
                                                {if e.detail.len() > 150 { format!("{}...", &e.detail[..150]) } else { e.detail.clone() }}
                                            </span>
                                            <span style="font-size: 9px; color: rgba(255,245,240,0.15); white-space: nowrap;">{e.timestamp.clone()}</span>
                                        </div>
                                    }.into_any()
                                }).collect::<Vec<_>>()
                            }
                        }}
                    </div>
                </Show>
            </Show>

            // Loading state
            <Show when=move || loading.get()>
                <Card>
                    <div style="text-align: center; padding: 32px; color: rgba(255,245,240,0.35); font-size: 12px;">"Loading mission data..."</div>
                </Card>
            </Show>
        </div>
    }
}
