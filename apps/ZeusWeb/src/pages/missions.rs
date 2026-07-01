// ═══════════════════════════════════════════════════════════
// ZEUS — Missions Page — Session List + Replay Viewer
// ═══════════════════════════════════════════════════════════

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;
use crate::api;
use crate::components::design::*;

#[component]
pub fn MissionsPage() -> impl IntoView {
    let sessions = RwSignal::new(Vec::<api::Session>::new());
    let total = RwSignal::new(0u32);
    let selected_id = RwSignal::new(Option::<String>::None);
    let replay = RwSignal::new(Vec::<api::ReplayTurn>::new());
    let stats = RwSignal::new(Option::<api::SessionStatsDetail>::None);
    let loading = RwSignal::new(true);
    let replay_loading = RwSignal::new(false);
    let search = RwSignal::new(String::new());

    // Fetch sessions on mount
    {
        let sessions = sessions;
        let total = total;
        let loading = loading;
        spawn_local(async move {
            if let Ok(s) = api::fetch_sessions().await {
                total.set(s.total);
                sessions.set(s.sessions);
            }
            loading.set(false);
        });
    }

    let load_replay = move |id: String| {
        selected_id.set(Some(id.clone()));
        replay.set(Vec::new());
        stats.set(None);
        replay_loading.set(true);
        let id2 = id.clone();
        spawn_local(async move {
            if let Ok(turns) = api::fetch_session_replay(&id).await {
                replay.set(turns);
            }
            replay_loading.set(false);
        });
        spawn_local(async move {
            if let Ok(s) = api::fetch_session_stats(&id2).await {
                stats.set(Some(s));
            }
        });
    };

    let close_replay = move |_| {
        selected_id.set(None);
        replay.set(Vec::new());
    };

    view! {
        <div style="padding: 32px;">
            <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 24px;">
                <div>
                    <h1 style="font-family: 'Orbitron', monospace; font-size: 14px; letter-spacing: 6px; color: rgba(255,245,240,0.9); margin: 0;">"MISSIONS"</h1>
                    <p style="color: rgba(255,245,240,0.7); font-size: 12px;">
                        {move || {
                            if loading.get() { "Loading missions...".to_string() }
                            else { format!("{} sessions recorded", total.get()) }
                        }}
                    </p>
                </div>
                <SearchBar placeholder="Search sessions..." value=search />
            </div>

            // Replay viewer overlay
            {move || {
                let sel = selected_id.get();
                sel.as_ref()?;
                let sid = sel.unwrap();
                Some(view! {
                    <Card>
                        <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 16px;">
                            <div>
                                <div style="font-family: 'Orbitron', monospace; font-size: 11px; letter-spacing: 2px; color: rgba(255,245,240,0.7);">"SESSION REPLAY"</div>
                                <div style="font-family: 'Orbitron', monospace; font-size: 10px; color: rgba(255,245,240,0.5); margin-top: 2px;">{sid.clone()}</div>
                            </div>
                            <button
                                on:click=close_replay
                                style="background: none; border: 1px solid rgba(255,60,20,0.2); color: rgba(255,245,240,0.5); cursor: pointer; padding: 4px 12px; border-radius: 4px; font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px;"
                            >"CLOSE"</button>
                        </div>

                        // Session stats bar
                        {move || {
                            let s = stats.get()?;
                            Some(view! {
                                <div style="display: flex; gap: 16px; margin-bottom: 12px; padding: 8px 12px; background: rgba(255,255,255,0.02); border-radius: 6px; flex-wrap: wrap;">
                                    <span style="font-size: 10px; color: rgba(255,245,240,0.6);">{format!("{} messages", s.message_count)}</span>
                                    <span style="font-size: 10px; color: rgba(255,245,240,0.6);">{format!("{} user / {} assistant", s.user_messages, s.assistant_messages)}</span>
                                    <span style="font-size: 10px; color: rgba(255,245,240,0.6);">{format!("{} tool calls", s.tool_calls)}</span>
                                    {(s.duration_seconds > 0).then(|| view! {
                                        <span style="font-size: 10px; color: rgba(255,245,240,0.6);">{format!("{}s duration", s.duration_seconds)}</span>
                                    })}
                                </div>
                            })
                        }}

                        {move || {
                            let is_loading = replay_loading.get();
                            let turns = replay.get();
                            if is_loading {
                                view! {
                                    <p style="color: rgba(255,245,240,0.5); font-size: 12px; padding: 20px 0;">"Loading replay..."</p>
                                }.into_any()
                            } else if turns.is_empty() {
                                view! {
                                    <p style="color: rgba(255,245,240,0.5); font-size: 12px; text-align: center; padding: 20px;">"No replay data — this mission wasn't recorded."</p>
                                }.into_any()
                            } else {
                                view! {
                            <div style="display: flex; flex-direction: column; gap: 8px; max-height: 500px; overflow-y: auto;">
                                {turns.into_iter().map(|turn| {
                                    let role_color = match turn.role.as_str() {
                                        "user" => "rgba(255,60,20,0.8)",
                                        "assistant" => "rgba(34,197,94,0.8)",
                                        "tool" => "rgba(234,179,8,0.8)",
                                        _ => "rgba(255,245,240,0.6)",
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
                                                        <span style="font-size: 9px; color: rgba(255,245,240,0.5);">{format!("{}tok", turn.token_count)}</span>
                                                    })}
                                                    <span style="font-size: 9px; color: rgba(255,245,240,0.5);">{"#"}{turn.index.to_string()}</span>
                                                </div>
                                            </div>
                                            <div style="font-size: 12px; color: rgba(255,245,240,0.5); white-space: pre-wrap; word-break: break-word; max-height: 150px; overflow-y: auto;">
                                                {if turn.content.len() > 500 { format!("{}...", &turn.content[..500]) } else { turn.content.clone() }}
                                            </div>
                                            {(!turn.tool_results.is_empty()).then(|| view! {
                                                <div style="margin-top: 6px; padding: 6px 10px; background: rgba(255,255,255,0.03); border-radius: 4px; font-size: 10px; color: rgba(255,245,240,0.6); font-family: 'Orbitron', monospace; max-height: 80px; overflow-y: auto;">
                                                    {if turn.tool_results.len() > 300 { format!("{}...", &turn.tool_results[..300]) } else { turn.tool_results.clone() }}
                                                </div>
                                            })}
                                        </div>
                                    }
                                }).collect::<Vec<_>>()}
                            </div>
                                }.into_any()
                            }
                        }}
                    </Card>
                })
            }}

            // Session list
            <Show when=move || selected_id.get().is_none()>
                <div style="display: flex; flex-direction: column; gap: 8px;">
                    {move || {
                        let filter = search.get().to_lowercase();
                        sessions.get().into_iter()
                            .filter(|s| {
                                if filter.is_empty() { return true; }
                                s.id.to_lowercase().contains(&filter)
                                    || s.agent_name.to_lowercase().contains(&filter)
                                    || s.model.to_lowercase().contains(&filter)
                                    || s.channel.to_lowercase().contains(&filter)
                            })
                            .map(|s| {
                                let sid = s.id.clone();
                                let load = load_replay;
                                view! {
                                    <Card style="cursor: pointer;">
                                        <div
                                            on:click=move |_| load(sid.clone())
                                            style="display: flex; align-items: center; gap: 12px;"
                                        >
                                            <div style="width: 4px; height: 36px; border-radius: 2px; background: #ff3c14; opacity: 0.4; flex-shrink: 0;" />
                                            <div style="flex: 1; min-width: 0;">
                                                <div style="display: flex; align-items: center; gap: 8px; margin-bottom: 4px;">
                                                    <span style="font-size: 13px; color: rgba(255,245,240,0.9); font-weight: 500;">
                                                        {if s.agent_name.is_empty() { "Session".to_string() } else { s.agent_name.clone() }}
                                                    </span>
                                                    {(!s.model.is_empty()).then(|| view! {
                                                        <Badge text={s.model.clone()} color="#3b82f6".to_string() />
                                                    })}
                                                    {(!s.channel.is_empty()).then(|| view! {
                                                        <Badge text={s.channel.clone()} color="rgba(255,60,20,0.6)".to_string() />
                                                    })}
                                                </div>
                                                <div style="display: flex; gap: 16px; align-items: center;">
                                                    <span style="font-size: 10px; color: rgba(255,245,240,0.5);">
                                                        {format!("{} messages", s.message_count)}
                                                    </span>
                                                    {(s.total_tokens > 0).then(|| view! {
                                                        <span style="font-size: 10px; color: rgba(255,245,240,0.5);">
                                                            {format!("{}tok", s.total_tokens)}
                                                        </span>
                                                    })}
                                                    {(s.cost > 0.0).then(|| view! {
                                                        <span style="font-size: 10px; color: rgba(255,245,240,0.5);">
                                                            {format!("${:.4}", s.cost)}
                                                        </span>
                                                    })}
                                                    {(s.duration_seconds > 0).then(|| view! {
                                                        <span style="font-size: 10px; color: rgba(255,245,240,0.5);">
                                                            {format!("{}s", s.duration_seconds)}
                                                        </span>
                                                    })}
                                                </div>
                                            </div>
                                            <div style="text-align: right; flex-shrink: 0;">
                                                <div style="font-size: 10px; color: rgba(255,245,240,0.5); font-family: 'Orbitron', monospace;">{s.id[..8.min(s.id.len())].to_string()}</div>
                                                <div style="font-size: 9px; color: rgba(255,245,240,0.5); margin-top: 2px;">{s.created.clone()}</div>
                                            </div>
                                            <Icon name="chevron-right" size=14 />
                                        </div>
                                    </Card>
                                }
                            }).collect::<Vec<_>>()
                    }}
                </div>
            </Show>
        </div>
    }
}
