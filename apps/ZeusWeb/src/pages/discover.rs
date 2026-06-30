// ═══════════════════════════════════════════════════════════
// ZEUS — Agent Discovery — searchable fleet + capability browser
// ═══════════════════════════════════════════════════════════

use leptos::prelude::*;
use crate::components::design::*;
use wasm_bindgen_futures::spawn_local;
use crate::api;

fn avatar_color(name: &str) -> &'static str {
    const COLORS: &[&str] = &[
        "#3b82f6", "#a855f7", "#ef4444", "#22c55e",
        "#f97316", "#eab308", "#06b6d4", "#8b5cf6",
    ];
    let idx = name.bytes().fold(0usize, |acc, b| acc.wrapping_add(b as usize)) % COLORS.len();
    COLORS[idx]
}

fn initials(name: &str) -> String {
    let parts: Vec<&str> = name.split_whitespace().collect();
    match parts.as_slice() {
        [] => "?".to_string(),
        [w] => w.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_else(|| "?".to_string()),
        [a, b, ..] => format!(
            "{}{}",
            a.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default(),
            b.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default()
        ),
    }
}

fn status_color(s: &str) -> &'static str {
    if s.contains("busy") || s == "executing" || s == "active" { "#22c55e" }
    else if s == "idle" || s == "available" || s == "online" { "#3b82f6" }
    else { "rgba(255,245,240,0.2)" }
}

#[component]
pub fn DiscoverPage() -> impl IntoView {
    let result   = RwSignal::new(None::<api::AgentDiscoverResponse>);
    let loading  = RwSignal::new(true);
    let q        = RwSignal::new(String::new());
    let cap_filter = RwSignal::new(Option::<String>::None);
    let stat_filter = RwSignal::new(Option::<String>::None);

    let run_discover = move || {
        loading.set(true);
        let q_val  = q.get_untracked();
        let cap    = cap_filter.get_untracked();
        let status = stat_filter.get_untracked();
        spawn_local(async move {
            let q_opt  = if q_val.trim().is_empty() { None } else { Some(q_val.as_str()) };
            let cap_opt = cap.as_deref();
            let sta_opt = status.as_deref();
            if let Ok(r) = api::discover_agents(cap_opt, sta_opt, q_opt).await {
                result.set(Some(r));
            }
            loading.set(false);
        });
    };

    // Initial load
    {
        let run = run_discover;
        spawn_local(async move { run(); });
    }

    let clear_filters = move || {
        q.set(String::new());
        cap_filter.set(None);
        stat_filter.set(None);
        run_discover();
    };

    view! {
        <div style="padding: 32px; font-family: 'Rajdhani', sans-serif; max-width: 1100px;">

            // ── Header ──────────────────────────────────────────
            <div style="margin-bottom: 28px;">
                <h1 style="font-family: 'Orbitron', monospace; font-size: 14px; letter-spacing: 6px; color: rgba(255,245,240,0.9); margin: 0 0 6px;">"AGENT DISCOVERY"</h1>
                <p style="font-size: 13px; color: rgba(255,245,240,0.3); margin: 0;">
                    {move || match result.get() {
                        None => "Scanning fleet...".to_string(),
                        Some(ref r) => format!("{} agents registered · {} in fleet · {} capability types", r.total, r.fleet_size, r.capabilities.len()),
                    }}
                </p>
            </div>

            // ── Search + Filters ─────────────────────────────────
            <div style="display: flex; gap: 10px; margin-bottom: 24px; flex-wrap: wrap; align-items: center;">
                // Search box
                <input
                    type="text"
                    placeholder="Search agents by name, capability..."
                    prop:value=move || q.get()
                    on:input=move |ev| q.set(event_target_value(&ev))
                    on:keydown=move |ev| { if ev.key() == "Enter" { run_discover(); } }
                    style="flex: 1; min-width: 220px; padding: 10px 14px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.15); border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 13px; outline: none; font-family: 'Rajdhani', sans-serif;"
                />
                // Status filter
                <select
                    on:change=move |ev| {
                        let v = event_target_value(&ev);
                        stat_filter.set(if v.is_empty() { None } else { Some(v) });
                        run_discover();
                    }
                    style="padding: 10px 12px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.12); border-radius: 8px; color: rgba(255,245,240,0.7); font-size: 12px; outline: none; cursor: pointer; font-family: 'Rajdhani', sans-serif;"
                >
                    <option value="">"All Status"</option>
                    <option value="idle">"Idle"</option>
                    <option value="busy">"Busy"</option>
                    <option value="online">"Online"</option>
                </select>
                // Search button
                <Button primary=true on_click=Some(Callback::new(move |_| run_discover()))>
                    "SEARCH"
                </Button>
                // Clear
                {move || {
                    let has_filter = !q.get().is_empty() || cap_filter.get().is_some() || stat_filter.get().is_some();
                    has_filter.then(|| view! {
                        <Button on_click=Some(Callback::new(move |_| clear_filters()))>
                            "CLEAR"
                        </Button>
                    })
                }}
            </div>

            // ── Capability tags ──────────────────────────────────
            {move || result.get().map(|r| {
                if r.capabilities.is_empty() { return None; }
                Some(view! {
                    <div style="display: flex; flex-wrap: wrap; gap: 6px; margin-bottom: 24px;">
                        {r.capabilities.into_iter().map(|cap| {
                            let cap2 = cap.clone();
                            let is_active = cap_filter.get().as_deref() == Some(&cap);
                            view! {
                                <button
                                    on:click=move |_| {
                                        if cap_filter.get().as_deref() == Some(&cap2) {
                                            cap_filter.set(None);
                                        } else {
                                            cap_filter.set(Some(cap2.clone()));
                                        }
                                        run_discover();
                                    }
                                    style=move || format!(
                                        "font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 1px; padding: 4px 10px; border-radius: 20px; cursor: pointer; transition: all 0.15s; {}",
                                        if is_active {
                                            "background: rgba(255,60,20,0.2); border: 1px solid rgba(255,60,20,0.4); color: rgba(255,245,240,0.9);"
                                        } else {
                                            "background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); color: rgba(255,245,240,0.4);"
                                        }
                                    )
                                >{cap}</button>
                            }
                        }).collect::<Vec<_>>()}
                    </div>
                })
            })}

            // ── Agent grid ──────────────────────────────────────
            {move || {
                if loading.get() {
                    return view! {
                        <div style="padding: 60px; text-align: center; color: rgba(255,245,240,0.2);">
                            <div style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 3px;">"SCANNING FLEET..."</div>
                        </div>
                    }.into_any();
                }

                let agents = result.get().map(|r| r.agents).unwrap_or_default();

                if agents.is_empty() {
                    return view! {
                        <div style="padding: 60px; text-align: center; color: rgba(255,245,240,0.2);">
                            <div style="font-size: 32px; margin-bottom: 12px; opacity: 0.3;">"🔍"</div>
                            <div style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 3px; margin-bottom: 8px;">"NO AGENTS FOUND"</div>
                            <div style="font-size: 12px; opacity: 0.8;">"Try different filters or check that agents are registered with the gateway"</div>
                        </div>
                    }.into_any();
                }

                view! {
                    <div style="display: grid; grid-template-columns: repeat(auto-fill, minmax(300px, 1fr)); gap: 14px;">
                        {agents.into_iter().map(|a| {
                            let av = avatar_color(&a.name);
                            let init = initials(&a.name);
                            let sc = status_color(&a.status);
                            let load_pct = (a.load_pct * 100.0) as u32;
                            let hb = if a.last_heartbeat.len() >= 16 {
                                a.last_heartbeat[11..16].to_string()
                            } else {
                                a.last_heartbeat.clone()
                            };
                            view! {
                                <div style="padding: 20px; background: rgba(255,255,255,0.02); border: 1px solid rgba(255,60,20,0.1); border-radius: 12px; display: flex; flex-direction: column; gap: 14px; transition: border-color 0.15s;">

                                    // Agent header
                                    <div style="display: flex; align-items: flex-start; gap: 12px;">
                                        // Avatar
                                        <div style="position: relative; flex-shrink: 0;">
                                            <div style=format!(
                                                "width: 42px; height: 42px; border-radius: 50%; background: {}; display: flex; align-items: center; justify-content: center; font-size: 14px; font-weight: 700; color: white; letter-spacing: 0;",
                                                av
                                            )>{init}</div>
                                            <div style=format!(
                                                "position: absolute; bottom: 0; right: 0; width: 11px; height: 11px; border-radius: 50%; background: {}; border: 2px solid #0a0508;",
                                                sc
                                            ) />
                                        </div>
                                        // Name + status
                                        <div style="flex: 1; min-width: 0;">
                                            <div style="font-size: 15px; font-weight: 700; color: rgba(255,245,240,0.9); overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">{a.name.clone()}</div>
                                            <div style="display: flex; align-items: center; gap: 8px; margin-top: 3px; flex-wrap: wrap;">
                                                <span style=format!(
                                                    "font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 1px; padding: 2px 7px; border-radius: 4px; background: {}20; color: {};",
                                                    sc, sc
                                                )>{a.status.to_uppercase()}</span>
                                                {(!hb.is_empty()).then(|| view! {
                                                    <span style="font-size: 10px; color: rgba(255,245,240,0.2);">{"♥ "}{hb}</span>
                                                })}
                                            </div>
                                        </div>
                                    </div>

                                    // Capabilities
                                    {(!a.capabilities.is_empty()).then(|| view! {
                                        <div style="display: flex; flex-wrap: wrap; gap: 5px;">
                                            {a.capabilities.iter().map(|cap| {
                                                let is_highlighted = cap_filter.get().as_deref()
                                                    .map(|f| cap.to_lowercase().contains(f))
                                                    .unwrap_or(false);
                                                view! {
                                                    <span style=move || format!(
                                                        "font-size: 11px; padding: 3px 8px; border-radius: 4px; {}",
                                                        if is_highlighted {
                                                            "background: rgba(255,60,20,0.15); color: rgba(255,245,240,0.85); border: 1px solid rgba(255,60,20,0.3);"
                                                        } else {
                                                            "background: rgba(255,255,255,0.04); color: rgba(255,245,240,0.7); border: 1px solid rgba(255,255,255,0.05);"
                                                        }
                                                    )>{cap.clone()}</span>
                                                }
                                            }).collect::<Vec<_>>()}
                                        </div>
                                    })}

                                    // Load bar + metadata
                                    <div style="display: flex; flex-direction: column; gap: 6px;">
                                        {(load_pct > 0).then(|| view! {
                                            <div>
                                                <div style="display: flex; justify-content: space-between; font-size: 10px; color: rgba(255,245,240,0.5); margin-bottom: 4px;">
                                                    <span>"Load"</span>
                                                    <span>{format!("{}%", load_pct)}</span>
                                                </div>
                                                <div style="height: 3px; background: rgba(255,255,255,0.06); border-radius: 2px; overflow: hidden;">
                                                    <div style=format!(
                                                        "height: 100%; width: {}%; background: {}; border-radius: 2px;",
                                                        load_pct.min(100), sc
                                                    ) />
                                                </div>
                                            </div>
                                        })}
                                        // Metadata badges (ip, model, etc.)
                                        {let meta_items: Vec<_> = a.metadata.iter()
                                            .filter(|(k, _)| ["ip", "model", "host", "machine", "role"].contains(&k.as_str()))
                                            .map(|(k, v)| (k.clone(), v.clone()))
                                            .collect();
                                         (!meta_items.is_empty()).then(move || view! {
                                            <div style="display: flex; flex-wrap: wrap; gap: 5px;">
                                                {meta_items.into_iter().map(|(k, v)| view! {
                                                    <span style="font-size: 10px; padding: 2px 7px; border-radius: 4px; background: rgba(255,255,255,0.03); color: rgba(255,245,240,0.3); border: 1px solid rgba(255,255,255,0.05);">
                                                        {format!("{}: {}", k, v)}
                                                    </span>
                                                }).collect::<Vec<_>>()}
                                            </div>
                                        })}
                                    </div>
                                </div>
                            }
                        }).collect::<Vec<_>>()}
                    </div>
                }.into_any()
            }}
        </div>
    }
}
