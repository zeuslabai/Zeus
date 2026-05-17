// ═══════════════════════════════════════════════════════════
// ZEUS — Dashboard Page — Phase 2: Wired to API
// ═══════════════════════════════════════════════════════════

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;
use crate::api;
use crate::components::design::*;
use crate::components::sentient_orb::SentientOrb;
use crate::components::visibility::use_tab_visible;

#[component]
pub fn DashboardPage() -> impl IntoView {
    let orb_mode = RwSignal::new("active".to_string());

    // API data signals
    let status = RwSignal::new(Option::<api::StatusResponse>::None);
    let stats = RwSignal::new(Option::<api::StatsResponse>::None);
    let channels = RwSignal::new(Option::<Vec<api::Channel>>::None);
    let agents = RwSignal::new(Option::<Vec<api::NetworkAgent>>::None);
    let sessions = RwSignal::new(Option::<Vec<api::Session>>::None);

    // Loading/error state
    let loading = RwSignal::new(true);
    let gateway_error = RwSignal::new(false);

    // Fire parallel fetches
    {
        let status = status;
        let gateway_error = gateway_error;
        spawn_local(async move {
            match api::fetch_status().await {
                Ok(s) => status.set(Some(s)),
                Err(_) => { gateway_error.set(true); }
            }
        });
    }
    {
        let stats = stats;
        spawn_local(async move {
            if let Ok(s) = api::fetch_stats().await { stats.set(Some(s)); }
        });
    }
    {
        let channels = channels;
        spawn_local(async move {
            if let Ok(c) = api::fetch_channels().await { channels.set(Some(c.channels)); }
        });
    }
    {
        let agents = agents;
        spawn_local(async move {
            if let Ok(a) = api::fetch_agents().await { agents.set(Some(a.agents)); }
        });
    }
    {
        let sessions = sessions;
        let loading = loading;
        spawn_local(async move {
            if let Ok(s) = api::fetch_sessions().await { sessions.set(Some(s.sessions)); }
            loading.set(false);
        });
    }

    // 30s polling refresh — keeps dashboard data live, pauses when tab is hidden
    let tab_visible = use_tab_visible();
    spawn_local(async move {
        loop {
            gloo_timers::future::TimeoutFuture::new(30_000).await;
            if !tab_visible.get_untracked() { continue; }
            if let Ok(s) = api::fetch_status().await { status.set(Some(s)); gateway_error.set(false); }
            else { gateway_error.set(true); }
            if let Ok(s) = api::fetch_stats().await { stats.set(Some(s)); }
            if let Ok(c) = api::fetch_channels().await { channels.set(Some(c.channels)); }
            if let Ok(a) = api::fetch_agents().await { agents.set(Some(a.agents)); }
            if let Ok(s) = api::fetch_sessions().await { sessions.set(Some(s.sessions)); }
        }
    });

    view! {
        <div style="padding: 32px;">
            // Gateway offline banner
            {move || gateway_error.get().then(|| view! {
                <div style="background: rgba(255,60,20,0.15); border: 1px solid rgba(255,60,20,0.4); border-radius: 8px; padding: 12px 16px; margin-bottom: 20px; display: flex; align-items: center; gap: 10px;">
                    <Icon name="warning" size=14 color="rgba(255,80,40,0.9)".to_string() />
                    <span style="font-family: 'Rajdhani', sans-serif; font-size: 13px; color: rgba(255,180,160,0.9); font-weight: 500;">
                        "Gateway offline — unable to reach Zeus API. Showing last known state."
                    </span>
                </div>
            })}

            // Header
            <div style="display: flex; align-items: center; justify-content: space-between; margin-bottom: 32px;">
                <div>
                    <h1 style="font-family: 'Orbitron', monospace; font-size: 14px; letter-spacing: 6px; color: rgba(255,245,240,0.9); margin: 0; text-transform: uppercase;">
                        "Command Center"
                    </h1>
                    <p style="font-family: 'Rajdhani', sans-serif; font-size: 12px; color: rgba(255,245,240,0.7); margin: 4px 0 0;">
                        {move || {
                            if loading.get() {
                                "Connecting to gateway...".to_string()
                            } else if gateway_error.get() {
                                "Gateway offline".to_string()
                            } else {
                                match status.get() {
                                    Some(s) if !s.model.is_empty() => format!("All systems operational — {}", s.model),
                                    _ => "Connecting to gateway...".to_string(),
                                }
                            }
                        }}
                    </p>
                </div>
                <div style="display: flex; gap: 8px;">
                    <Button primary=true on_click=Some(Callback::new(move |_| {
                        let _ = web_sys::window().unwrap().location().assign("/studio");
                    }))>
                        <Icon name="chat" size=12 /> " New Session"
                    </Button>
                </div>
            </div>

            // Hero Orb Card
            <Card glow=true style="display: flex; align-items: center; gap: 32px; margin-bottom: 24px; padding: 28px;">
                {move || view! { <SentientOrb size=140 mode={orb_mode.get()} /> }}
                <div style="flex: 1;">
                    <div style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 4px; color: rgba(255,60,20,0.6); margin-bottom: 8px;">
                        {move || {
                            match status.get() {
                                Some(s) if !s.status.is_empty() => format!("ZEUS PRIME — {}", s.status.to_uppercase()),
                                _ if gateway_error.get() => "ZEUS PRIME — OFFLINE".to_string(),
                                _ => "ZEUS PRIME — CONNECTING".to_string(),
                            }
                        }}
                    </div>
                    <div style="font-family: 'Rajdhani', sans-serif; font-size: 20px; font-weight: 600; color: rgba(255,245,240,0.9); margin-bottom: 8px;">
                        "Autonomous Cognitive Entity"
                    </div>
                    <div style="font-family: 'Rajdhani', sans-serif; font-size: 13px; color: rgba(255,245,240,0.7); line-height: 1.6; margin-bottom: 16px;">
                        {move || {
                            if loading.get() {
                                "Loading...".to_string()
                            } else {
                                let st = stats.get().unwrap_or_default();
                                let v = status.get().map(|s| s.version).unwrap_or_default();
                                format!("{} tools • {} sessions • v{}", st.tools.total, st.sessions.total, v)
                            }
                        }}
                    </div>
                    <div style="display: flex; gap: 8px;">
                        {["dormant", "active", "speaking", "thinking", "rage"].iter().map(|m| {
                            let mode = m.to_string();
                            let mode_c = mode.clone();
                            view! {
                                <button
                                    class=move || if orb_mode.get() == mode_c { "zbtn zbtn-small zbtn-primary" } else { "zbtn zbtn-small" }
                                    on:click={
                                        let mode = mode.clone();
                                        move |_| orb_mode.set(mode.clone())
                                    }
                                >
                                    {*m}
                                </button>
                            }
                        }).collect::<Vec<_>>()}
                    </div>
                </div>
            </Card>

            // Metric Cards
            <div style="display: flex; gap: 12px; margin-bottom: 24px; flex-wrap: wrap;">
                {move || {
                    if loading.get() {
                        view! {
                            <div style="font-family: 'Rajdhani', sans-serif; font-size: 13px; color: rgba(255,245,240,0.4); padding: 16px;">
                                "Loading metrics..."
                            </div>
                        }.into_any()
                    } else {
                        let st = stats.get().unwrap_or_default();
                        let s = status.get().unwrap_or_default();
                        let ch = channels.get().unwrap_or_default();
                        let active_ch = ch.iter().filter(|c| c.status == "connected").count();
                        view! {
                            <MetricCard label="Tools" value={st.tools.total.to_string()} icon="tools" trend=Some(format!("{} categories", st.tools.categories)) />
                            <MetricCard label="LLM Providers" value={if s.provider.is_empty() { "—".to_string() } else { s.provider.clone() }} icon="cpu" />
                            <MetricCard label="Channels" value={ch.len().to_string()} icon="channels" trend=Some(format!("{} active", active_ch)) />
                            <MetricCard label="Memory" value={format!("{} files", st.memory.workspace_files)} icon="memory" trend=Some(format!("{} entries", st.memory.total_entries)) />
                            <MetricCard label="Sessions" value={st.sessions.total.to_string()} icon="sessions" trend=Some(format!("{} active", st.sessions.active)) />
                        }.into_any()
                    }
                }}
            </div>

            // Two-column grid
            <div style="display: grid; grid-template-columns: 1fr 1fr; gap: 16px;">
                // Recent Sessions
                <Card>
                    <SectionTitle action=Box::new(|| view! { <a href="/missions" style="text-decoration: none;"><Button small=true>"View All"</Button></a> }.into_any())>
                        "Recent Activity"
                    </SectionTitle>
                    {move || {
                        let sess = sessions.get();
                        match sess {
                            None if loading.get() => view! {
                                <div style="font-size: 12px; color: rgba(255,245,240,0.35); padding: 16px 0; text-align: center;">
                                    "Loading sessions..."
                                </div>
                            }.into_any(),
                            None | Some(_) if gateway_error.get() => view! {
                                <div style="font-size: 12px; color: rgba(255,100,60,0.6); padding: 16px 0; text-align: center;">
                                    "Gateway offline"
                                </div>
                            }.into_any(),
                            Some(list) if list.is_empty() => view! {
                                <div style="font-size: 12px; color: rgba(255,245,240,0.35); padding: 16px 0; text-align: center;">
                                    "No recent sessions"
                                </div>
                            }.into_any(),
                            Some(list) => list.into_iter().take(4).map(|s| {
                                let cost_str = format!("${:.2}", s.cost);
                                view! {
                                    <div style="display: flex; align-items: center; gap: 12px; padding: 10px 0; border-bottom: 1px solid rgba(255,60,20,0.1);">
                                        <div style="width: 32px; height: 32px; border-radius: 8px; background: rgba(255,60,20,0.15); display: flex; align-items: center; justify-content: center;">
                                            <Icon name="chat" size=14 color="rgba(255,60,20,0.6)".to_string() />
                                        </div>
                                        <div style="flex: 1;">
                                            <div style="font-size: 13px; color: rgba(255,245,240,0.9); font-weight: 500;">{s.agent_name.clone()}</div>
                                            <div style="font-size: 11px; color: rgba(255,245,240,0.7);">{s.message_count}" messages • "{cost_str}" • "{s.model.clone()}</div>
                                        </div>
                                        <span style="font-size: 11px; color: rgba(255,245,240,0.5);">{s.created.clone()}</span>
                                    </div>
                                }
                            }).collect::<Vec<_>>().into_any(),
                            _ => view! { <div></div> }.into_any(),
                        }
                    }}
                </Card>

                // Active Agents
                <Card>
                    <SectionTitle action=Box::new(|| view! { <a href="/agents" style="text-decoration: none;"><Button small=true>"Manage"</Button></a> }.into_any())>
                        "Active Agents"
                    </SectionTitle>
                    {move || {
                        let ag = agents.get();
                        match ag {
                            None if loading.get() => view! {
                                <div style="font-size: 12px; color: rgba(255,245,240,0.35); padding: 16px 0; text-align: center;">
                                    "Loading agents..."
                                </div>
                            }.into_any(),
                            None | Some(_) if gateway_error.get() => view! {
                                <div style="font-size: 12px; color: rgba(255,100,60,0.6); padding: 16px 0; text-align: center;">
                                    "Gateway offline"
                                </div>
                            }.into_any(),
                            Some(list) if list.is_empty() => view! {
                                <div style="padding: 20px 0; text-align: center;">
                                    <div style="font-size: 12px; color: rgba(255,245,240,0.35); margin-bottom: 12px;">
                                        "Your fleet is ready. Spawn your first agent to begin."
                                    </div>
                                    <a href="/agents" style="display: inline-block; padding: 8px 20px; background: rgba(255,60,20,0.15); border: 1px solid rgba(255,60,20,0.4); border-radius: 6px; font-family: 'Rajdhani', sans-serif; font-size: 12px; font-weight: 600; letter-spacing: 2px; color: rgba(255,245,240,0.85); text-decoration: none; text-transform: uppercase; cursor: pointer;">
                                        "Create Agent →"
                                    </a>
                                </div>
                            }.into_any(),
                            Some(list) => list.into_iter().take(4).map(|a| {
                                let agent_mode = if a.status == "active" { "active".to_string() } else { "dormant".to_string() };
                                let status_str = a.status.clone();
                                let model_badge = a.model.split('-').take(2).collect::<Vec<_>>().join("-");
                                view! {
                                    <div style="display: flex; align-items: center; gap: 12px; padding: 10px 0; border-bottom: 1px solid rgba(255,60,20,0.1);">
                                        <SentientOrb size=36 mode=agent_mode />
                                        <div style="flex: 1;">
                                            <div style="display: flex; align-items: center; gap: 8px;">
                                                <span style="font-size: 13px; color: rgba(255,245,240,0.9); font-weight: 600;">{a.name.clone()}</span>
                                                <StatusDot status=status_str />
                                            </div>
                                            <div style="font-size: 11px; color: rgba(255,245,240,0.7);">{a.role.clone()}" • "{a.tasks}" tasks"</div>
                                        </div>
                                        <Badge text=model_badge />
                                    </div>
                                }
                            }).collect::<Vec<_>>().into_any(),
                            _ => view! { <div></div> }.into_any(),
                        }
                    }}
                </Card>
            </div>

            // Channel Status
            <Card style="margin-top: 16px;">
                <SectionTitle action=Box::new(|| view! { <a href="/channels" style="text-decoration: none;"><Button small=true>"Configure"</Button></a> }.into_any())>
                    "Channel Status"
                </SectionTitle>
                <div style="display: grid; grid-template-columns: repeat(4, 1fr); gap: 12px;">
                    {move || {
                        let ch = channels.get();
                        match ch {
                            None if loading.get() => view! {
                                <div style="font-size: 12px; color: rgba(255,245,240,0.35); padding: 8px; grid-column: span 4;">
                                    "Loading channels..."
                                </div>
                            }.into_any(),
                            None | Some(_) if gateway_error.get() => view! {
                                <div style="font-size: 12px; color: rgba(255,100,60,0.6); padding: 8px; grid-column: span 4;">
                                    "Gateway offline"
                                </div>
                            }.into_any(),
                            Some(list) => list.into_iter().map(|c| {
                                let status_str = c.status.clone();
                                view! {
                                    <div style="padding: 12px; background: rgba(255,255,255,0.02); border-radius: 8px; border: 1px solid rgba(255,60,20,0.1);">
                                        <div style="display: flex; align-items: center; gap: 8px; margin-bottom: 6px;">
                                            <StatusDot status=status_str />
                                            <span style="font-size: 13px; font-weight: 500; color: rgba(255,245,240,0.9);">{c.name.clone()}</span>
                                        </div>
                                        <div style="font-size: 11px; color: rgba(255,245,240,0.7);">{c.message_count}" messages"</div>
                                    </div>
                                }
                            }).collect::<Vec<_>>().into_any(),
                            _ => view! { <div></div> }.into_any(),
                        }
                    }}
                </div>
            </Card>
        </div>
    }
}
