// ═══════════════════════════════════════════════════════════
// ZEUS — Pantheon Page — War Rooms + Missions
// ═══════════════════════════════════════════════════════════

use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;
use crate::api;
use crate::components::design::*;

#[component]
pub fn PantheonPage() -> impl IntoView {
    let missions = RwSignal::new(Vec::<api::PantheonMission>::new());
    let rooms = RwSignal::new(Vec::<api::PantheonRoom>::new());
    let loading = RwSignal::new(true);
    let error = RwSignal::new(Option::<String>::None);

    // Selected mission / room
    let selected_mission = RwSignal::new(Option::<String>::None);
    let mission_detail = RwSignal::new(Option::<api::PantheonMission>::None);
    let room_messages = RwSignal::new(Vec::<api::PantheonRoomMessage>::new());
    let selected_room = RwSignal::new(Option::<String>::None);

    // New mission form
    let new_mission_goal = RwSignal::new(String::new());
    let creating_mission = RwSignal::new(false);

    // Tab: "missions" or "rooms"
    let active_tab = RwSignal::new("missions".to_string());

    // Fetch missions + rooms
    {
        let missions = missions;
        let rooms = rooms;
        let loading = loading;
        let error = error;
        spawn_local(async move {
            let mut err_msg = None;
            match api::fetch_pantheon_missions().await {
                Ok(m) => missions.set(m),
                Err(e) => { err_msg = Some(e); }
            }
            match api::fetch_pantheon_rooms().await {
                Ok(r) => rooms.set(r),
                Err(e) => if err_msg.is_none() { err_msg = Some(e); }
            }
            error.set(err_msg);
            loading.set(false);
        });
    }

    // Fetch mission detail when selected
    {
        let selected_mission = selected_mission;
        let mission_detail = mission_detail;
        Effect::new(move |_| {
            if let Some(id) = selected_mission.get() {
                let mission_detail = mission_detail;
                spawn_local(async move {
                    if let Ok(m) = api::fetch_pantheon_mission(&id).await {
                        mission_detail.set(Some(m));
                    }
                });
            } else {
                mission_detail.set(None);
            }
        });
    }

    // Fetch room messages when selected
    {
        let selected_room = selected_room;
        let room_messages = room_messages;
        Effect::new(move |_| {
            if let Some(id) = selected_room.get() {
                let room_messages = room_messages;
                spawn_local(async move {
                    if let Ok(msgs) = api::fetch_room_messages(&id, 50).await {
                        room_messages.set(msgs);
                    }
                });
            } else {
                room_messages.set(vec![]);
            }
        });
    }

    let do_create_mission = move || {
        let goal = new_mission_goal.get();
        if goal.trim().is_empty() { return; }
        creating_mission.set(true);
        let missions = missions;
        spawn_local(async move {
            let req = api::CreateMissionRequest {
                goal: goal.clone(),
                constraints: None,
            };
            match api::create_pantheon_mission(&req).await {
                Ok(resp) => {
                    if let Ok(m) = api::fetch_pantheon_missions().await {
                        missions.set(m);
                    }
                    selected_mission.set(Some(resp.id));
                    new_mission_goal.set(String::new());
                }
                Err(e) => {
                    web_sys::window().and_then(|w| w.alert_with_message(&format!("Failed: {}", e)).ok());
                }
            }
            creating_mission.set(false);
        });
    };

    let status_color = |status: &str| -> &str {
        match status {
            "active" | "Active" => "#22c55e",
            "completed" | "Completed" => "#3b82f6",
            "planning" | "Planning" | "assembling" | "Assembling" => "#f59e0b",
            "failed" | "Failed" | "cancelled" | "Cancelled" => "#ef4444",
            "reviewing" | "Reviewing" => "#a855f7",
            "draft" | "Draft" => "#6b7280",
            _ => "#9ca3af",
        }
    };

    view! {
        <div style="padding: 32px; height: 100%; display: flex; flex-direction: column;">
            // Header
            <div style="display: flex; align-items: center; justify-content: space-between; margin-bottom: 24px;">
                <div style="display: flex; align-items: center; gap: 12px;">
                    <Icon name="team" size=20 color="rgba(255,60,20,0.9)".to_string() />
                    <h1 style="font-family: 'Orbitron', monospace; font-size: 14px; letter-spacing: 6px; color: rgba(255,245,240,0.9); margin: 0; text-transform: uppercase;">
                        "Pantheon"
                    </h1>
                </div>

                // Tab switcher
                <div style="display: flex; gap: 2px; background: rgba(255,245,240,0.05); border-radius: 6px; padding: 2px;">
                    <button
                        style=move || format!(
                            "padding: 6px 16px; font-family: 'Rajdhani', sans-serif; font-size: 12px; border: none; border-radius: 4px; cursor: pointer; transition: all 0.2s; {}",
                            if active_tab.get() == "missions" {
                                "background: rgba(255,60,20,0.2); color: rgba(255,245,240,0.9);"
                            } else {
                                "background: transparent; color: rgba(255,245,240,0.5);"
                            }
                        )
                        on:click=move |_| active_tab.set("missions".to_string())
                    >
                        "Missions"
                    </button>
                    <button
                        style=move || format!(
                            "padding: 6px 16px; font-family: 'Rajdhani', sans-serif; font-size: 12px; border: none; border-radius: 4px; cursor: pointer; transition: all 0.2s; {}",
                            if active_tab.get() == "rooms" {
                                "background: rgba(255,60,20,0.2); color: rgba(255,245,240,0.9);"
                            } else {
                                "background: transparent; color: rgba(255,245,240,0.5);"
                            }
                        )
                        on:click=move |_| active_tab.set("rooms".to_string())
                    >
                        "War Rooms"
                    </button>
                </div>
            </div>

            // Error banner
            {move || error.get().map(|e| view! {
                <div style="background: rgba(255,60,20,0.15); border: 1px solid rgba(255,60,20,0.4); border-radius: 8px; padding: 12px 16px; margin-bottom: 16px;">
                    <span style="font-family: 'Rajdhani', sans-serif; font-size: 13px; color: rgba(255,180,160,0.9);">
                        {format!("Error loading Pantheon: {}", e)}
                    </span>
                </div>
            })}

            // Loading
            {move || loading.get().then(|| view! {
                <div style="text-align: center; padding: 48px; color: rgba(255,245,240,0.5); font-family: 'Rajdhani', sans-serif;">
                    "Loading Pantheon..."
                </div>
            })}

            // Main content area
            {move || (!loading.get()).then(|| {
                let tab = active_tab.get();
                if tab == "missions" {
                    view! {
                        <div style="display: flex; gap: 16px; flex: 1; overflow: hidden;">
                            // Left: missions list + create form
                            <div style="width: 380px; display: flex; flex-direction: column; gap: 12px; overflow-y: auto;">
                                // Create mission form
                                <Card>
                                    <h4 style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 3px; color: rgba(255,245,240,0.5); margin: 0 0 10px; text-transform: uppercase;">"New Mission"</h4>
                                    <div style="display: flex; gap: 8px;">
                                        <input
                                            type="text"
                                            placeholder="Describe the mission goal..."
                                            style="flex: 1; background: rgba(255,245,240,0.05); border: 1px solid rgba(255,245,240,0.1); border-radius: 6px; padding: 8px 12px; color: rgba(255,245,240,0.9); font-family: 'Rajdhani', sans-serif; font-size: 13px; outline: none;"
                                            prop:value=move || new_mission_goal.get()
                                            on:input=move |ev| new_mission_goal.set(event_target_value(&ev))
                                            on:keydown=move |ev: web_sys::KeyboardEvent| {
                                                if ev.key() == "Enter" { do_create_mission(); }
                                            }
                                        />
                                        <button
                                            style="background: rgba(255,60,20,0.8); border: none; border-radius: 6px; padding: 8px 14px; color: white; font-family: 'Rajdhani', sans-serif; font-size: 12px; font-weight: 600; cursor: pointer;"
                                            on:click=move |_| do_create_mission()
                                        >
                                            {move || if creating_mission.get() { "Creating..." } else { "Launch" }}
                                        </button>
                                    </div>
                                </Card>

                                // Missions list
                                {move || {
                                    let m = missions.get();
                                    if m.is_empty() {
                                        view! {
                                            <div style="text-align: center; padding: 32px; color: rgba(255,245,240,0.4); font-family: 'Rajdhani', sans-serif; font-size: 13px;">
                                                "No missions yet. Create one above."
                                            </div>
                                        }.into_any()
                                    } else {
                                        view! {
                                            <div>
                                                {m.iter().map(|mission| {
                                                    let id = mission.id.clone();
                                                    let goal = mission.goal.clone();
                                                    let status = mission.status.clone();
                                                    let sc = status_color(&status);
                                                    let progress = mission.progress_pct;
                                                    let tasks_done = mission.tasks_done;
                                                    let tasks_total = mission.tasks_total;
                                                    let team_size = mission.team_size;
                                                    let id_click = id.clone();
                                                    let is_selected = move || selected_mission.get().as_deref() == Some(&id);
                                                    view! {
                                                        <div
                                                            style=move || format!(
                                                                "background: {}; border: 1px solid {}; border-radius: 8px; padding: 14px; margin-bottom: 8px; cursor: pointer; transition: all 0.2s;",
                                                                if is_selected() { "rgba(255,60,20,0.1)" } else { "rgba(255,245,240,0.03)" },
                                                                if is_selected() { "rgba(255,60,20,0.3)" } else { "rgba(255,245,240,0.08)" },
                                                            )
                                                            on:click=move |_| selected_mission.set(Some(id_click.clone()))
                                                        >
                                                            <div style="display: flex; align-items: center; justify-content: space-between; margin-bottom: 6px;">
                                                                <span style="font-family: 'Rajdhani', sans-serif; font-size: 13px; color: rgba(255,245,240,0.9); font-weight: 600;">
                                                                    {goal.chars().take(60).collect::<String>()}
                                                                    {if goal.len() > 60 { "..." } else { "" }}
                                                                </span>
                                                                <span style=format!(
                                                                    "font-family: 'Rajdhani', sans-serif; font-size: 10px; padding: 2px 8px; border-radius: 4px; background: {}20; color: {}; text-transform: uppercase; font-weight: 600;",
                                                                    sc, sc
                                                                )>
                                                                    {status}
                                                                </span>
                                                            </div>
                                                            <div style="display: flex; align-items: center; gap: 16px; margin-top: 8px;">
                                                                <span style="font-family: 'Rajdhani', sans-serif; font-size: 11px; color: rgba(255,245,240,0.5);">
                                                                    {format!("{}/{} tasks", tasks_done, tasks_total)}
                                                                </span>
                                                                <span style="font-family: 'Rajdhani', sans-serif; font-size: 11px; color: rgba(255,245,240,0.5);">
                                                                    {format!("{} agents", team_size)}
                                                                </span>
                                                                <div style="flex: 1; height: 3px; background: rgba(255,245,240,0.08); border-radius: 2px;">
                                                                    <div style=format!(
                                                                        "height: 100%; width: {}%; background: {}; border-radius: 2px; transition: width 0.3s;",
                                                                        progress, sc
                                                                    ) />
                                                                </div>
                                                                <span style="font-family: 'Rajdhani', sans-serif; font-size: 11px; color: rgba(255,245,240,0.5);">
                                                                    {format!("{:.0}%", progress)}
                                                                </span>
                                                            </div>
                                                        </div>
                                                    }
                                                }).collect::<Vec<_>>()}
                                            </div>
                                        }.into_any()
                                    }
                                }}
                            </div>

                            // Right: mission detail
                            <div style="flex: 1; overflow-y: auto;">
                                {move || {
                                    if let Some(m) = mission_detail.get() {
                                        let sc = status_color(&m.status);
                                        view! {
                                            <Card>
                                                <div style="margin-bottom: 16px;">
                                                    <h3 style="font-family: 'Rajdhani', sans-serif; font-size: 16px; color: rgba(255,245,240,0.9); margin: 0 0 8px;">
                                                        {m.goal.clone()}
                                                    </h3>
                                                    <div style="display: flex; gap: 12px; align-items: center;">
                                                        <span style=format!(
                                                            "font-family: 'Rajdhani', sans-serif; font-size: 11px; padding: 2px 10px; border-radius: 4px; background: {}20; color: {};",
                                                            sc, sc
                                                        )>
                                                            {m.status.clone()}
                                                        </span>
                                                        <span style="font-family: 'Rajdhani', sans-serif; font-size: 11px; color: rgba(255,245,240,0.4);">
                                                            {format!("Created: {}", &m.created_at.get(..10).unwrap_or(&m.created_at))}
                                                        </span>
                                                        {(m.tokens_used > 0).then(|| view! {
                                                            <span style="font-family: 'Rajdhani', sans-serif; font-size: 11px; color: rgba(255,245,240,0.4);">
                                                                {format!("Tokens: {}", m.tokens_used)}
                                                            </span>
                                                        })}
                                                    </div>
                                                </div>

                                                // Progress bar
                                                <div style="margin-bottom: 16px;">
                                                    <div style="display: flex; justify-content: space-between; margin-bottom: 4px;">
                                                        <span style="font-family: 'Rajdhani', sans-serif; font-size: 11px; color: rgba(255,245,240,0.5);">
                                                            "Progress"
                                                        </span>
                                                        <span style="font-family: 'Rajdhani', sans-serif; font-size: 11px; color: rgba(255,245,240,0.5);">
                                                            {format!("{:.0}%", m.progress_pct)}
                                                        </span>
                                                    </div>
                                                    <div style="height: 6px; background: rgba(255,245,240,0.08); border-radius: 3px;">
                                                        <div style=format!(
                                                            "height: 100%; width: {}%; background: {}; border-radius: 3px;",
                                                            m.progress_pct, sc
                                                        ) />
                                                    </div>
                                                </div>

                                                // Team
                                                {(!m.team.is_empty()).then(|| view! {
                                                    <div style="margin-bottom: 16px;">
                                                        <h4 style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 3px; color: rgba(255,245,240,0.5); margin: 0 0 8px; text-transform: uppercase;">
                                                            "Team"
                                                        </h4>
                                                        <div style="display: flex; flex-wrap: wrap; gap: 8px;">
                                                            {m.team.iter().map(|member| {
                                                                view! {
                                                                    <div style="background: rgba(255,245,240,0.05); border-radius: 6px; padding: 6px 10px; display: flex; align-items: center; gap: 6px;">
                                                                        <StatusDot status=member.status.clone() />
                                                                        <span style="font-family: 'Rajdhani', sans-serif; font-size: 12px; color: rgba(255,245,240,0.8);">
                                                                            {member.name.clone()}
                                                                        </span>
                                                                        <span style="font-family: 'Rajdhani', sans-serif; font-size: 10px; color: rgba(255,245,240,0.4);">
                                                                            {member.role.clone()}
                                                                        </span>
                                                                    </div>
                                                                }
                                                            }).collect::<Vec<_>>()}
                                                        </div>
                                                    </div>
                                                })}

                                                // Tasks
                                                {(!m.tasks.is_empty()).then(|| view! {
                                                    <div style="margin-bottom: 16px;">
                                                        <h4 style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 3px; color: rgba(255,245,240,0.5); margin: 0 0 8px; text-transform: uppercase;">
                                                            {format!("Tasks ({}/{})", m.tasks_done, m.tasks_total)}
                                                        </h4>
                                                        {m.tasks.iter().map(|task| {
                                                            let tc = status_color(&task.status);
                                                            view! {
                                                                <div style="background: rgba(255,245,240,0.03); border: 1px solid rgba(255,245,240,0.06); border-radius: 6px; padding: 10px 12px; margin-bottom: 6px;">
                                                                    <div style="display: flex; align-items: center; justify-content: space-between;">
                                                                        <span style="font-family: 'Rajdhani', sans-serif; font-size: 12px; color: rgba(255,245,240,0.8);">
                                                                            {task.description.clone()}
                                                                        </span>
                                                                        <span style=format!(
                                                                            "font-family: 'Rajdhani', sans-serif; font-size: 10px; color: {};", tc
                                                                        )>
                                                                            {task.status.clone()}
                                                                        </span>
                                                                    </div>
                                                                    {task.assigned_to.as_ref().map(|a| view! {
                                                                        <span style="font-family: 'Rajdhani', sans-serif; font-size: 10px; color: rgba(255,245,240,0.3); margin-top: 4px; display: block;">
                                                                            {format!("Assigned: {}", a)}
                                                                        </span>
                                                                    })}
                                                                </div>
                                                            }
                                                        }).collect::<Vec<_>>()}
                                                    </div>
                                                })}

                                                // Intervention controls
                                                {(m.status == "active" || m.status == "Active").then(|| {
                                                    let mission_id = m.id.clone();
                                                    let mission_id2 = m.id.clone();
                                                    view! {
                                                        <div style="display: flex; gap: 8px; margin-top: 12px; padding-top: 12px; border-top: 1px solid rgba(255,245,240,0.06);">
                                                            <button
                                                                style="background: rgba(245,158,11,0.2); border: 1px solid rgba(245,158,11,0.4); border-radius: 6px; padding: 6px 14px; color: #f59e0b; font-family: 'Rajdhani', sans-serif; font-size: 12px; cursor: pointer;"
                                                                on:click=move |_| {
                                                                    let id = mission_id.clone();
                                                                    spawn_local(async move {
                                                                        let _ = api::intervene_pantheon_mission(&id, "pause").await;
                                                                    });
                                                                }
                                                            >
                                                                "Pause"
                                                            </button>
                                                            <button
                                                                style="background: rgba(239,68,68,0.2); border: 1px solid rgba(239,68,68,0.4); border-radius: 6px; padding: 6px 14px; color: #ef4444; font-family: 'Rajdhani', sans-serif; font-size: 12px; cursor: pointer;"
                                                                on:click=move |_| {
                                                                    let id = mission_id2.clone();
                                                                    spawn_local(async move {
                                                                        let _ = api::intervene_pantheon_mission(&id, "cancel").await;
                                                                    });
                                                                }
                                                            >
                                                                "Cancel"
                                                            </button>
                                                        </div>
                                                    }
                                                })}

                                                // Activity feed
                                                {(!m.feed.is_empty()).then(|| view! {
                                                    <div style="margin-top: 16px;">
                                                        <h4 style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 3px; color: rgba(255,245,240,0.5); margin: 0 0 8px; text-transform: uppercase;">
                                                            "Activity Feed"
                                                        </h4>
                                                        <div style="max-height: 200px; overflow-y: auto;">
                                                            {m.feed.iter().rev().take(20).map(|entry| {
                                                                view! {
                                                                    <div style="display: flex; gap: 8px; padding: 6px 0; border-bottom: 1px solid rgba(255,245,240,0.04);">
                                                                        <span style="font-family: 'Rajdhani', sans-serif; font-size: 10px; color: rgba(255,60,20,0.7); min-width: 80px;">
                                                                            {entry.agent_name.clone()}
                                                                        </span>
                                                                        <span style="font-family: 'Rajdhani', sans-serif; font-size: 11px; color: rgba(255,245,240,0.7); flex: 1;">
                                                                            {entry.activity.clone()}
                                                                        </span>
                                                                        <span style="font-family: 'Rajdhani', sans-serif; font-size: 10px; color: rgba(255,245,240,0.3);">
                                                                            {entry.timestamp.get(11..19).unwrap_or("").to_string()}
                                                                        </span>
                                                                    </div>
                                                                }
                                                            }).collect::<Vec<_>>()}
                                                        </div>
                                                    </div>
                                                })}
                                            </Card>
                                        }.into_any()
                                    } else {
                                        view! {
                                            <div style="display: flex; align-items: center; justify-content: center; height: 100%; color: rgba(255,245,240,0.3); font-family: 'Rajdhani', sans-serif; font-size: 13px;">
                                                "Select a mission to view details"
                                            </div>
                                        }.into_any()
                                    }
                                }}
                            </div>
                        </div>
                    }.into_any()
                } else {
                    // War Rooms tab
                    view! {
                        <div style="display: flex; gap: 16px; flex: 1; overflow: hidden;">
                            // Left: room list
                            <div style="width: 300px; display: flex; flex-direction: column; gap: 8px; overflow-y: auto;">
                                {move || {
                                    let r = rooms.get();
                                    if r.is_empty() {
                                        view! {
                                            <div style="text-align: center; padding: 32px; color: rgba(255,245,240,0.4); font-family: 'Rajdhani', sans-serif; font-size: 13px;">
                                                "No war rooms. Rooms are created automatically when missions launch."
                                            </div>
                                        }.into_any()
                                    } else {
                                        view! {
                                            <div>
                                                {r.iter().map(|room| {
                                                    let id = room.id.clone();
                                                    let name = room.name.clone();
                                                    let desc = room.description.clone().unwrap_or_default();
                                                    let member_count = room.member_count;
                                                    let room_type = room.room_type.clone();
                                                    let id_click = id.clone();
                                                    let is_selected = move || selected_room.get().as_deref() == Some(&id);
                                                    view! {
                                                        <div
                                                            style=move || format!(
                                                                "background: {}; border: 1px solid {}; border-radius: 8px; padding: 12px; cursor: pointer; transition: all 0.2s;",
                                                                if is_selected() { "rgba(255,60,20,0.1)" } else { "rgba(255,245,240,0.03)" },
                                                                if is_selected() { "rgba(255,60,20,0.3)" } else { "rgba(255,245,240,0.08)" },
                                                            )
                                                            on:click=move |_| selected_room.set(Some(id_click.clone()))
                                                        >
                                                            <div style="display: flex; align-items: center; gap: 8px; margin-bottom: 4px;">
                                                                <span style="font-family: 'Rajdhani', sans-serif; font-size: 14px; color: rgba(255,60,20,0.8);">"#"</span>
                                                                <span style="font-family: 'Rajdhani', sans-serif; font-size: 13px; color: rgba(255,245,240,0.9); font-weight: 600;">
                                                                    {name}
                                                                </span>
                                                                <span style="font-family: 'Rajdhani', sans-serif; font-size: 10px; color: rgba(255,245,240,0.3); margin-left: auto;">
                                                                    {format!("{} members", member_count)}
                                                                </span>
                                                            </div>
                                                            {(!desc.is_empty()).then(|| view! {
                                                                <p style="font-family: 'Rajdhani', sans-serif; font-size: 11px; color: rgba(255,245,240,0.4); margin: 0;">
                                                                    {desc}
                                                                </p>
                                                            })}
                                                            <span style="font-family: 'Rajdhani', sans-serif; font-size: 10px; color: rgba(255,245,240,0.2);">
                                                                {room_type}
                                                            </span>
                                                        </div>
                                                    }
                                                }).collect::<Vec<_>>()}
                                            </div>
                                        }.into_any()
                                    }
                                }}
                            </div>

                            // Right: room messages + input
                            <div style="flex: 1; display: flex; flex-direction: column; overflow: hidden;">
                                {move || {
                                    let rid = selected_room.get();
                                    let msgs = room_messages.get();
                                    if rid.is_none() {
                                        view! {
                                            <div style="display: flex; align-items: center; justify-content: center; height: 100%; color: rgba(255,245,240,0.3); font-family: 'Rajdhani', sans-serif; font-size: 13px;">
                                                "Select a war room to view messages"
                                            </div>
                                        }.into_any()
                                    } else {
                                        let room_id = rid.clone().unwrap_or_default();
                                        let room_id_send = room_id.clone();
                                        let room_id_upload = room_id.clone();
                                        let msg_input = RwSignal::new(String::new());
                                        let room_messages_ref = room_messages;
                                        view! {
                                            // Messages list
                                            <div style="flex: 1; overflow-y: auto; display: flex; flex-direction: column; gap: 4px; padding-bottom: 8px;">
                                                {msgs.iter().map(|msg| {
                                                    let attachments = msg.attachments.clone();
                                                    let is_plan = msg.message_type == "plan_card";
                                                    let plan_meta = if is_plan {
                                                        msg.metadata.as_ref().and_then(|m| serde_json::from_value::<api::PlanCardMeta>(m.clone()).ok())
                                                    } else { None };
                                                    view! {
                                                        <div style="padding: 8px 12px;">
                                                            <div style="display: flex; align-items: baseline; gap: 8px; margin-bottom: 2px;">
                                                                <span style="font-family: 'Rajdhani', sans-serif; font-size: 12px; color: rgba(255,60,20,0.8); font-weight: 600;">
                                                                    {msg.sender_name.clone()}
                                                                </span>
                                                                <span style="font-family: 'Rajdhani', sans-serif; font-size: 10px; color: rgba(255,245,240,0.3);">
                                                                    {msg.timestamp.get(11..19).unwrap_or("").to_string()}
                                                                </span>
                                                                {msg.edited.then(|| view! {
                                                                    <span style="font-family: 'Rajdhani', sans-serif; font-size: 10px; color: rgba(255,245,240,0.2);">"(edited)"</span>
                                                                })}
                                                            </div>
                                                            // Message content
                                                            <p style="font-family: 'Rajdhani', sans-serif; font-size: 13px; color: rgba(255,245,240,0.8); margin: 0; white-space: pre-wrap;">
                                                                {msg.content.clone()}
                                                            </p>
                                                            // Inline attachments
                                                            {(!attachments.is_empty()).then(|| {
                                                                let atts = attachments.clone();
                                                                view! {
                                                                    <div style="display: flex; flex-direction: column; gap: 6px; margin-top: 6px;">
                                                                        {atts.iter().map(|att| {
                                                                            let ct = att.content_type.clone();
                                                                            let url = att.url.clone();
                                                                            let fname = att.filename.clone();
                                                                            let size = att.size;
                                                                            if ct.starts_with("image/") {
                                                                                // Inline image preview
                                                                                view! {
                                                                                    <div style="border-radius: 8px; overflow: hidden; max-width: 400px;">
                                                                                        <img src={url.clone()} alt={fname.clone()}
                                                                                            style="max-width: 100%; border-radius: 8px; cursor: pointer;"
                                                                                            on:click=move |_| { if let Some(w) = web_sys::window() { let _ = w.open_with_url(&url); } }
                                                                                        />
                                                                                        <div style="font-family: 'Rajdhani', sans-serif; font-size: 10px; color: rgba(255,245,240,0.4); padding: 2px 4px;">
                                                                                            {format!("{} ({})", fname, format_size(size))}
                                                                                        </div>
                                                                                    </div>
                                                                                }.into_any()
                                                                            } else if ct.starts_with("audio/") {
                                                                                // Audio player
                                                                                view! {
                                                                                    <div style="padding: 8px; background: rgba(255,255,255,0.03); border-radius: 8px; border: 1px solid rgba(255,60,20,0.1);">
                                                                                        <div style="font-family: 'Rajdhani', sans-serif; font-size: 11px; color: rgba(255,245,240,0.5); margin-bottom: 4px;">
                                                                                            {format!("🎵 {} ({})", fname, format_size(size))}
                                                                                        </div>
                                                                                        <audio controls=true style="width: 100%; height: 32px;">
                                                                                            <source src={url} type={ct} />
                                                                                        </audio>
                                                                                    </div>
                                                                                }.into_any()
                                                                            } else {
                                                                                // Generic file download link
                                                                                view! {
                                                                                    <a href={url} target="_blank"
                                                                                        style="display: inline-flex; align-items: center; gap: 6px; padding: 6px 10px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 6px; text-decoration: none; color: rgba(255,245,240,0.7); font-family: 'Rajdhani', sans-serif; font-size: 12px;">
                                                                                        "📎 " {format!("{} ({})", fname, format_size(size))}
                                                                                    </a>
                                                                                }.into_any()
                                                                            }
                                                                        }).collect::<Vec<_>>()}
                                                                    </div>
                                                                }
                                                            })}
                                                            // Plan card approval UI
                                                            {plan_meta.map(|plan| {
                                                                let plan_id = plan.plan_id.clone();
                                                                let plan_id2 = plan_id.clone();
                                                                let awaiting = plan.status == "awaiting_approval";
                                                                view! {
                                                                    <div style="margin-top: 8px; padding: 12px; background: rgba(255,60,20,0.05); border: 1px solid rgba(255,60,20,0.15); border-radius: 10px;">
                                                                        <div style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,60,20,0.6); margin-bottom: 8px;">"PLAN CARD"</div>
                                                                        <div style="font-family: 'Rajdhani', sans-serif; font-size: 13px; color: rgba(255,245,240,0.8); margin-bottom: 8px;">{plan.goal.clone()}</div>
                                                                        <div style="display: flex; flex-direction: column; gap: 4px; margin-bottom: 10px;">
                                                                            {plan.steps.iter().enumerate().map(|(i, step)| {
                                                                                let status_color = match step.status.as_str() {
                                                                                    "done" => "rgba(100,255,100,0.7)",
                                                                                    "running" => "rgba(255,200,50,0.7)",
                                                                                    "failed" => "rgba(255,80,80,0.7)",
                                                                                    _ => "rgba(255,245,240,0.4)",
                                                                                };
                                                                                view! {
                                                                                    <div style="display: flex; align-items: center; gap: 8px; font-family: 'Rajdhani', sans-serif; font-size: 12px;">
                                                                                        <span style=format!("color: {}; min-width: 16px;", status_color)>{format!("#{}", i + 1)}</span>
                                                                                        <span style="color: rgba(255,245,240,0.7);">{step.description.clone()}</span>
                                                                                        <span style="font-size: 10px; color: rgba(255,245,240,0.3);">{format!("[{}]", step.status)}</span>
                                                                                    </div>
                                                                                }
                                                                            }).collect::<Vec<_>>()}
                                                                        </div>
                                                                        {awaiting.then(|| view! {
                                                                            <div style="display: flex; gap: 8px;">
                                                                                <button
                                                                                    style="padding: 6px 16px; background: rgba(100,255,100,0.15); border: 1px solid rgba(100,255,100,0.3); border-radius: 6px; color: rgba(100,255,100,0.9); font-family: 'Orbitron', monospace; font-size: 10px; cursor: pointer;"
                                                                                    on:click=move |_| {
                                                                                        let pid = plan_id.clone();
                                                                                        spawn_local(async move {
                                                                                            let _ = api::approve_plan(&pid, "user", "User").await;
                                                                                        });
                                                                                    }
                                                                                >"APPROVE"</button>
                                                                                <button
                                                                                    style="padding: 6px 16px; background: rgba(255,80,80,0.15); border: 1px solid rgba(255,80,80,0.3); border-radius: 6px; color: rgba(255,80,80,0.9); font-family: 'Orbitron', monospace; font-size: 10px; cursor: pointer;"
                                                                                    on:click=move |_| {
                                                                                        let pid = plan_id2.clone();
                                                                                        spawn_local(async move {
                                                                                            let _ = api::reject_plan(&pid, "Needs revision", "user", "User").await;
                                                                                        });
                                                                                    }
                                                                                >"REJECT"</button>
                                                                            </div>
                                                                        })}
                                                                    </div>
                                                                }
                                                            })}
                                                        </div>
                                                    }
                                                }).collect::<Vec<_>>()}
                                            </div>
                                            // Message input + file upload
                                            <div style="border-top: 1px solid rgba(255,60,20,0.1); padding: 8px 12px; display: flex; gap: 8px; align-items: center;">
                                                // File upload button
                                                <label style="cursor: pointer; padding: 6px; border-radius: 6px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); display: flex; align-items: center;">
                                                    <input type="file" style="display: none;"
                                                        on:change=move |ev: web_sys::Event| {
                                                            let rid = room_id_upload.clone();
                                                            if let Some(target) = ev.target() {
                                                                let input: web_sys::HtmlInputElement = target.unchecked_into();
                                                                if let Some(files) = input.files() {
                                                                    if let Some(file) = files.get(0) {
                                                                        spawn_local(async move {
                                                                            match api::upload_room_file(&rid, &file, "user", "User", "").await {
                                                                                Ok(msg) => {
                                                                                    room_messages_ref.update(|m| m.push(msg));
                                                                                }
                                                                                Err(e) => {
                                                                                    web_sys::console::warn_1(&format!("Upload failed: {}", e).into());
                                                                                }
                                                                            }
                                                                        });
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    />
                                                    <span style="font-size: 16px;">"📎"</span>
                                                </label>
                                                // Text input
                                                <input type="text"
                                                    style="flex: 1; padding: 8px 12px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 8px; color: rgba(255,245,240,0.9); font-family: 'Rajdhani', sans-serif; font-size: 13px; outline: none;"
                                                    placeholder="Type a message..."
                                                    prop:value=move || msg_input.get()
                                                    on:input=move |ev| msg_input.set(event_target_value(&ev))
                                                    on:keydown=move |ev: web_sys::KeyboardEvent| {
                                                        if ev.key() == "Enter" && !msg_input.get().trim().is_empty() {
                                                            let content = msg_input.get().trim().to_string();
                                                            let rid = room_id_send.clone();
                                                            msg_input.set(String::new());
                                                            let room_msgs = room_messages_ref;
                                                            spawn_local(async move {
                                                                let req = api::SendRoomMessageRequest {
                                                                    sender_id: "user".to_string(),
                                                                    sender_name: "User".to_string(),
                                                                    content,
                                                                    message_type: "chat".to_string(),
                                                                    reply_to: None,
                                                                };
                                                                if let Ok(msg) = api::send_room_message(&rid, &req).await {
                                                                    room_msgs.update(|m| m.push(msg));
                                                                }
                                                            });
                                                        }
                                                    }
                                                />
                                            </div>
                                        }.into_any()
                                    }
                                }}
                            </div>
                        </div>
                    }.into_any()
                }
            })}
        </div>
    }
}

/// Format file size for display
fn format_size(bytes: u64) -> String {
    if bytes < 1024 { format!("{} B", bytes) }
    else if bytes < 1024 * 1024 { format!("{:.1} KB", bytes as f64 / 1024.0) }
    else { format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0)) }
}
