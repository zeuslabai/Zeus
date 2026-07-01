// ═══════════════════════════════════════════════════════════
// ZEUS — Sessions Page — Full API Wiring
// ═══════════════════════════════════════════════════════════

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;
use crate::api;
use crate::components::design::*;

#[component]
pub fn SessionsPage() -> impl IntoView {
    let search = RwSignal::new(String::new());
    let sessions = RwSignal::new(Vec::<api::Session>::new());
    let total = RwSignal::new(0u32);
    let total_cost = RwSignal::new(0.0f64);
    let search_mode = RwSignal::new(false);
    let detail_id = RwSignal::new(Option::<String>::None);
    let detail = RwSignal::new(Option::<api::SessionDetail>::None);
    let detail_stats = RwSignal::new(Option::<api::SessionStatsDetail>::None);
    let detail_tools = RwSignal::new(Vec::<api::ToolExecution>::new());
    let detail_replay = RwSignal::new(Vec::<api::ReplayTurn>::new());
    let detail_tab = RwSignal::new("messages".to_string());

    {
        spawn_local(async move {
            if let Ok(s) = api::fetch_sessions().await {
                total.set(s.total);
                let cost: f64 = s.sessions.iter().map(|sess| sess.cost).sum();
                total_cost.set(cost);
                sessions.set(s.sessions);
            }
        });
    }

    // 30s polling refresh — same cadence as the dashboard. Pauses while the
    // tab is hidden (doesn't hammer the API for an off-screen view), while
    // the user has a search query active (so we don't clobber their result
    // set mid-browse), and while a session-detail panel is open (so we don't
    // refetch the list the user is navigating away from).
    let tab_visible = crate::components::visibility::use_tab_visible();
    spawn_local(async move {
        loop {
            gloo_timers::future::TimeoutFuture::new(30_000).await;
            if !tab_visible.get_untracked() { continue; }
            if search_mode.get_untracked() { continue; }
            if detail_id.get_untracked().is_some() { continue; }
            if let Ok(s) = api::fetch_sessions().await {
                total.set(s.total);
                let cost: f64 = s.sessions.iter().map(|sess| sess.cost).sum();
                total_cost.set(cost);
                sessions.set(s.sessions);
            }
        }
    });

    let reload_sessions = move || {
        spawn_local(async move {
            if let Ok(s) = api::fetch_sessions().await {
                total.set(s.total);
                let cost: f64 = s.sessions.iter().map(|sess| sess.cost).sum();
                total_cost.set(cost);
                sessions.set(s.sessions);
            }
        });
    };

    let do_search = move |_| {
        let q = search.get_untracked();
        if q.trim().is_empty() {
            search_mode.set(false);
            reload_sessions();
            return;
        }
        search_mode.set(true);
        spawn_local(async move {
            match api::search_sessions(&q).await {
                Ok(s) => {
                    total.set(s.total);
                    sessions.set(s.sessions);
                }
                Err(e) => web_sys::console::warn_1(&format!("Search failed: {}", e).into()),
            }
        });
    };

    let open_detail = move |sid: String| {
        detail_id.set(Some(sid.clone()));
        detail_tab.set("messages".to_string());
        detail.set(None);
        detail_stats.set(None);
        detail_tools.set(vec![]);
        detail_replay.set(vec![]);
        let sid2 = sid.clone();
        let sid3 = sid.clone();
        let sid4 = sid.clone();
        spawn_local(async move {
            if let Ok(d) = api::fetch_session(&sid).await { detail.set(Some(d)); }
        });
        spawn_local(async move {
            if let Ok(s) = api::fetch_session_stats(&sid2).await { detail_stats.set(Some(s)); }
        });
        spawn_local(async move {
            if let Ok(t) = api::fetch_session_tools(&sid3).await {
                detail_tools.set(t.tools);
            }
        });
        spawn_local(async move {
            if let Ok(r) = api::fetch_session_replay(&sid4).await { detail_replay.set(r); }
        });
    };

    view! {
        // Session detail overlay
        <Show when=move || detail_id.get().is_some()>
            <div style="position: fixed; inset: 0; background: rgba(0,0,0,0.78); z-index: 1000; display: flex; align-items: center; justify-content: center;">
                <div style="background: #0d0704; border: 1px solid rgba(255,60,20,0.25); border-radius: 16px; padding: 32px; width: 720px; max-width: 94vw; max-height: 80vh; overflow-y: auto;">
                    <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 20px;">
                        <div style="font-family: 'Orbitron', monospace; font-size: 11px; letter-spacing: 5px; color: rgba(255,245,240,0.9);">"SESSION DETAIL"</div>
                        <button style="background: transparent; border: none; color: rgba(255,245,240,0.7); font-size: 18px; cursor: pointer;" on:click=move |_| detail_id.set(None)>"\u{00D7}"</button>
                    </div>

                    // Stats row
                    {move || detail_stats.get().map(|s| view! {
                        <div style="display: flex; gap: 12px; margin-bottom: 16px; flex-wrap: wrap;">
                            <MetricCard label="Messages" value={s.message_count.to_string()} icon="chat" />
                            <MetricCard label="User Msgs" value={s.user_messages.to_string()} icon="agents" />
                            <MetricCard label="Tool Calls" value={s.tool_calls.to_string()} icon="tools" />
                            <MetricCard label="Duration" value={format!("{}s", s.duration_seconds)} icon="activity" />
                        </div>
                    })}

                    // Tabs
                    <div style="display: flex; gap: 4px; margin-bottom: 16px;">
                        {["messages", "replay", "tools", "audit"].into_iter().map(|tab| {
                            let t = tab.to_string();
                            let t2 = t.clone();
                            view! {
                                <button
                                    on:click=move |_| detail_tab.set(t2.clone())
                                    style=move || format!("font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 2px; padding: 6px 14px; border-radius: 6px; cursor: pointer; border: 1px solid {}; background: {}; color: {};",
                                        if detail_tab.get() == t { "rgba(255,60,20,0.5)" } else { "rgba(255,60,20,0.1)" },
                                        if detail_tab.get() == t { "rgba(255,60,20,0.15)" } else { "transparent" },
                                        if detail_tab.get() == t { "rgba(255,140,80,1)" } else { "rgba(255,245,240,0.7)" },
                                    )
                                >{tab.to_uppercase()}</button>
                            }
                        }).collect::<Vec<_>>()}
                    </div>

                    // Messages tab
                    <Show when=move || detail_tab.get() == "messages">
                        {move || {
                            let d = detail.get();
                            match d {
                                None => view! { <div style="color: rgba(255,245,240,0.7); font-size: 13px;">"Loading..."</div> }.into_any(),
                                Some(sd) => view! {
                                    <div style="display: flex; flex-direction: column; gap: 8px; max-height: 400px; overflow-y: auto;">
                                        {sd.messages.into_iter().map(|m| {
                                            let is_user = m.role == "user";
                                            view! {
                                                <div style={format!("padding: 10px 14px; border-radius: 10px; max-width: 85%; {}",
                                                    if is_user { "align-self: flex-end; background: rgba(255,60,20,0.12); border: 1px solid rgba(255,60,20,0.2);" }
                                                    else { "align-self: flex-start; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.06);" }
                                                )}>
                                                    <div style="font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 1px; color: rgba(255,245,240,0.35); margin-bottom: 4px;">{m.role.to_uppercase()}</div>
                                                    <div style="font-size: 13px; color: rgba(255,245,240,0.9); line-height: 1.5; white-space: pre-wrap;">{m.content.clone()}</div>
                                                </div>
                                            }
                                        }).collect::<Vec<_>>()}
                                    </div>
                                }.into_any(),
                            }
                        }}
                    </Show>

                    // Replay tab
                    <Show when=move || detail_tab.get() == "replay">
                        {move || {
                            let r = detail_replay.get();
                            if r.is_empty() {
                                view! { <div style="color: rgba(255,245,240,0.7); font-size: 13px; padding: 16px;">"No replay data — this session wasn't recorded."</div> }.into_any()
                            } else {
                                view! {
                                    <div style="display: flex; flex-direction: column; gap: 6px; max-height: 400px; overflow-y: auto;">
                                        {r.into_iter().enumerate().map(|(i, turn)| {
                                            view! {
                                                <div style="display: flex; gap: 10px; padding: 8px 10px; background: rgba(255,255,255,0.02); border-radius: 6px;">
                                                    <span style="font-family: 'Orbitron', monospace; font-size: 10px; color: rgba(255,60,20,0.4); min-width: 24px;">{format!("#{}", i + 1)}</span>
                                                    <div style="flex: 1;">
                                                        <div style="font-size: 12px; color: rgba(255,245,240,0.9);">{turn.role.clone()}</div>
                                                        <div style="font-size: 11px; color: rgba(255,245,240,0.7); overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">{turn.content.clone()}</div>
                                                    </div>
                                                    {(turn.token_count > 0).then(|| view! {
                                                        <span style="font-size: 10px; color: rgba(255,245,240,0.5);">{turn.token_count.to_string()}" tok"</span>
                                                    })}
                                                </div>
                                            }
                                        }).collect::<Vec<_>>()}
                                    </div>
                                }.into_any()
                            }
                        }}
                    </Show>

                    // Tools tab
                    <Show when=move || detail_tab.get() == "tools">
                        {move || {
                            let t = detail_tools.get();
                            if t.is_empty() {
                                view! { <div style="color: rgba(255,245,240,0.7); font-size: 13px; padding: 16px;">"No tools were called in this session."</div> }.into_any()
                            } else {
                                view! {
                                    <div style="display: flex; flex-direction: column; gap: 6px;">
                                        {t.into_iter().map(|tool| {
                                            view! {
                                                <div style="padding: 8px 12px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.08); border-radius: 8px;">
                                                    <div style="font-family: 'Orbitron', monospace; font-size: 11px; color: rgba(255,245,240,0.9); margin-bottom: 4px;">{tool.name.clone()}</div>
                                                    <div style="font-size: 10px; color: rgba(255,245,240,0.7); overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">{tool.output.clone()}</div>
                                                </div>
                                            }
                                        }).collect::<Vec<_>>()}
                                    </div>
                                }.into_any()
                            }
                        }}
                    </Show>

                    // Audit tab
                    <Show when=move || detail_tab.get() == "audit">
                        {move || {
                            let sid = detail_id.get().unwrap_or_default();
                            let audit_data = RwSignal::new(Vec::<api::AuditEntry>::new());
                            let sid2 = sid.clone();
                            spawn_local(async move {
                                if let Ok(a) = api::fetch_session_audit(&sid2).await {
                                    audit_data.set(a.entries);
                                }
                            });
                            view! {
                                <div style="max-height: 400px; overflow-y: auto;">
                                    {move || {
                                        let entries = audit_data.get();
                                        if entries.is_empty() {
                                            view! { <div style="color: rgba(255,245,240,0.7); font-size: 13px; padding: 16px;">"No audit entries — this session ran clean."</div> }.into_any()
                                        } else {
                                            view! {
                                                <div style="display: flex; flex-direction: column; gap: 4px;">
                                                    {entries.into_iter().map(|e| {
                                                        view! {
                                                            <div style="display: flex; align-items: center; gap: 8px; padding: 6px 10px; background: rgba(255,255,255,0.02); border-radius: 4px;">
                                                                <div style="width: 4px; height: 4px; border-radius: 50%; background: rgba(255,60,20,0.4);" />
                                                                <span style="font-family: 'Orbitron', monospace; font-size: 10px; color: rgba(255,60,20,0.7);">{e.entry_type.clone()}</span>
                                                                <span style="flex: 1; font-size: 11px; color: rgba(255,245,240,0.7); overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">{e.detail.clone()}</span>
                                                                <span style="font-size: 9px; color: rgba(255,245,240,0.5);">{e.timestamp.clone()}</span>
                                                            </div>
                                                        }
                                                    }).collect::<Vec<_>>()}
                                                </div>
                                            }.into_any()
                                        }
                                    }}
                                </div>
                            }
                        }}
                    </Show>
                </div>
            </div>
        </Show>

        <div style="padding: 32px;">
            <div style="margin-bottom: 24px;">
                <h1 style="font-family: 'Orbitron', monospace; font-size: 14px; letter-spacing: 6px; color: rgba(255,245,240,0.9); margin: 0;">"SESSIONS"</h1>
                <p style="font-size: 12px; color: rgba(255,245,240,0.7); margin: 4px 0 0;">{move || {
                    let t = total.get();
                    let c = total_cost.get();
                    if t == 0 { "Loading sessions...".to_string() }
                    else { format!("{} total sessions \u{2022} ${:.2} total cost", t, c) }
                }}</p>
            </div>
            <div style="display: flex; gap: 8px; margin-bottom: 16px;">
                <div style="flex: 1;">
                    <SearchBar placeholder="Search sessions..." value=search />
                </div>
                <Button primary=true on_click=Some(Callback::new(do_search))>"Search"</Button>
                <Show when=move || search_mode.get()>
                    <Button on_click=Some(Callback::new(move |_| {
                        search_mode.set(false);
                        search.set(String::new());
                        reload_sessions();
                    }))>"Clear"</Button>
                </Show>
            </div>
            <div style="display: flex; flex-direction: column; gap: 8px;">
                {move || {
                    let s = search.get().to_lowercase();
                    sessions.get().into_iter()
                        .filter(|sess| !search_mode.get() || s.is_empty() || sess.agent_name.to_lowercase().contains(&s) || sess.channel.to_lowercase().contains(&s))
                        .map(|sess| {
                            let cost_str = format!("${:.2}", sess.cost);
                            let model_label = if sess.agent_name.is_empty() { "unknown".to_string() } else { sess.agent_name.clone() };
                            let sid_click = sess.id.clone();
                            view! {
                                <Card style="display: flex; align-items: center; gap: 16px; cursor: pointer; padding: 16px 20px;">
                                    <div style="width: 40px; height: 40px; border-radius: 10px; background: rgba(255,60,20,0.15); display: flex; align-items: center; justify-content: center;"
                                        on:click=move |_| open_detail(sid_click.clone())
                                    >
                                        <Icon name="chat" size=18 color="rgba(255,60,20,0.6)".to_string() />
                                    </div>
                                    <div style="flex: 1;" on:click={let sid2 = sess.id.clone(); move |_| open_detail(sid2.clone())}>
                                        <div style="font-size: 14px; color: rgba(255,245,240,0.9); font-weight: 600; margin-bottom: 3px;">{model_label}</div>
                                        <div style="font-size: 11px; color: rgba(255,245,240,0.7);">
                                            {sess.message_count}" messages \u{2022} "{cost_str}
                                            {if !sess.channel.is_empty() { format!(" \u{2022} {}", sess.channel) } else { String::new() }}
                                        </div>
                                    </div>
                                    <Badge text={if sess.model.is_empty() { sess.id[..8.min(sess.id.len())].to_string() } else { sess.model.clone() }} />
                                    <span style="font-size: 11px; color: rgba(255,245,240,0.5); min-width: 50px; text-align: right;">{sess.created.clone()}</span>
                                    <button style="font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 1px; padding: 4px 8px; border-radius: 5px; cursor: pointer; background: rgba(239,68,68,0.08); border: 1px solid rgba(239,68,68,0.15); color: rgba(239,68,68,0.6);"
                                        on:click={
                                            let sid = sess.id.clone();
                                            move |_| {
                                                let sid = sid.clone();
                                                spawn_local(async move {
                                                    let _ = api::delete_session(&sid).await;
                                                    reload_sessions();
                                                });
                                            }
                                        }
                                    >"DEL"</button>
                                </Card>
                            }
                        })
                        .collect::<Vec<_>>()
                }}
            </div>
        </div>
    }
}
