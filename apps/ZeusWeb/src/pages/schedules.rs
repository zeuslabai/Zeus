// ═══════════════════════════════════════════════════════════
// ZEUS — Schedules Page — Cron Jobs & Task Scheduling
// ═══════════════════════════════════════════════════════════

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;
use crate::api;
use crate::components::design::*;

#[component]
pub fn SchedulesPage() -> impl IntoView {
    let schedules = RwSignal::new(Vec::<api::Schedule>::new());
    let total = RwSignal::new(0u32);
    let loading = RwSignal::new(true);
    let show_create = RwSignal::new(false);
    let new_name = RwSignal::new(String::new());
    let new_cron = RwSignal::new(String::new());
    let new_type = RwSignal::new("shell".to_string());
    let new_payload = RwSignal::new(String::new());
    let message = RwSignal::new(String::new());

    // Fetch on mount
    {
        spawn_local(async move {
            if let Ok(s) = api::fetch_schedules().await {
                total.set(s.total);
                schedules.set(s.schedules);
            }
            loading.set(false);
        });
    }

    let create_schedule = move |_| {
        let name = new_name.get();
        let cron = new_cron.get();
        let task_type = new_type.get();
        let payload = new_payload.get();
        if name.trim().is_empty() || cron.trim().is_empty() { return; }
        let req = api::CreateScheduleReq {
            name: name.clone(),
            cron: cron.clone(),
            task_type: task_type.clone(),
            task_payload: payload.clone(),
        };
        spawn_local(async move {
            match api::create_schedule(&req).await {
                Ok(_) => {
                    message.set("Schedule created".to_string());
                    show_create.set(false);
                    new_name.set(String::new());
                    new_cron.set(String::new());
                    new_payload.set(String::new());
                    if let Ok(s) = api::fetch_schedules().await {
                        total.set(s.total);
                        schedules.set(s.schedules);
                    }
                }
                Err(e) => message.set(format!("Error: {}", e)),
            }
        });
    };

    let toggle_schedule = move |id: String, enabled: bool| {
        let body = serde_json::json!({ "enabled": !enabled });
        spawn_local(async move {
            match api::update_schedule(&id, &body).await {
                Ok(_) => {
                    if let Ok(s) = api::fetch_schedules().await {
                        total.set(s.total);
                        schedules.set(s.schedules);
                    }
                }
                Err(e) => message.set(format!("Error: {}", e)),
            }
        });
    };

    let delete_schedule = move |id: String| {
        spawn_local(async move {
            match api::delete_schedule(&id).await {
                Ok(_) => {
                    message.set("Schedule deleted".to_string());
                    if let Ok(s) = api::fetch_schedules().await {
                        total.set(s.total);
                        schedules.set(s.schedules);
                    }
                }
                Err(e) => message.set(format!("Error: {}", e)),
            }
        });
    };

    view! {
        <div style="padding: 32px;">
            <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 24px;">
                <div>
                    <h1 style="font-family: 'Orbitron', monospace; font-size: 14px; letter-spacing: 6px; color: rgba(255,245,240,0.9); margin: 0;">"SCHEDULES"</h1>
                    <p style="color: rgba(255,245,240,0.7); font-size: 12px;">
                        {move || {
                            if loading.get() { "Loading schedules...".to_string() }
                            else {
                                let s = schedules.get();
                                let active = s.iter().filter(|sc| sc.enabled).count();
                                format!("{} schedules • {} active", total.get(), active)
                            }
                        }}
                    </p>
                </div>
                <div style="display: flex; gap: 8px; align-items: center;">
                    {move || {
                        let msg = message.get();
                        (!msg.is_empty()).then(|| view! { <Badge text=msg color="#22c55e".to_string() /> })
                    }}
                    <Button primary=true on_click=Some(Callback::new(move |_| show_create.set(!show_create.get())))>
                        <Icon name="plus" size=12 /> " New Schedule"
                    </Button>
                </div>
            </div>

            // Create form
            <Show when=move || show_create.get()>
                <Card style="margin-bottom: 16px;">
                    <div style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 3px; color: rgba(255,245,240,0.7); margin-bottom: 12px;">"CREATE SCHEDULE"</div>
                    <div style="display: flex; flex-direction: column; gap: 10px;">
                        <div style="display: flex; gap: 10px;">
                            <input
                                type="text" placeholder="Name"
                                prop:value=move || new_name.get()
                                on:input=move |ev| new_name.set(event_target_value(&ev))
                                style="flex: 1; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 6px; padding: 8px 12px; color: rgba(255,245,240,0.9); font-size: 12px;"
                            />
                            <input
                                type="text" placeholder="Cron (e.g. */5 * * * *)"
                                prop:value=move || new_cron.get()
                                on:input=move |ev| new_cron.set(event_target_value(&ev))
                                style="width: 200px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 6px; padding: 8px 12px; color: rgba(255,245,240,0.9); font-family: 'Orbitron', monospace; font-size: 11px;"
                            />
                        </div>
                        <div style="display: flex; gap: 8px; align-items: center;">
                            <span style="font-size: 10px; color: rgba(255,245,240,0.5);">"Type:"</span>
                            {["shell", "chat", "heartbeat", "memory_sync"].into_iter().map(|t| {
                                let typ = t.to_string();
                                let t_clone = t.to_string();
                                view! {
                                    <button
                                        on:click=move |_| new_type.set(t_clone.clone())
                                        style={move || format!(
                                            "padding: 4px 10px; border-radius: 4px; font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 1px; cursor: pointer; border: 1px solid {}; background: {}; color: {};",
                                            if new_type.get() == typ { "rgba(255,60,20,0.4)" } else { "rgba(255,60,20,0.1)" },
                                            if new_type.get() == typ { "rgba(255,60,20,0.1)" } else { "transparent" },
                                            if new_type.get() == typ { "rgba(255,245,240,0.9)" } else { "rgba(255,245,240,0.6)" },
                                        )}
                                    >{t.to_uppercase()}</button>
                                }
                            }).collect::<Vec<_>>()}
                        </div>
                        <textarea
                            placeholder="Task payload (command, message, or config)"
                            prop:value=move || new_payload.get()
                            on:input=move |ev| new_payload.set(event_target_value(&ev))
                            rows=3
                            style="background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 6px; padding: 8px 12px; color: rgba(255,245,240,0.9); font-family: 'Orbitron', monospace; font-size: 11px; resize: vertical;"
                        />
                        <div style="display: flex; gap: 8px; justify-content: flex-end;">
                            <Button on_click=Some(Callback::new(move |_| show_create.set(false)))>"Cancel"</Button>
                            <Button primary=true on_click=Some(Callback::new(create_schedule))>"Create"</Button>
                        </div>
                    </div>
                </Card>
            </Show>

            // Schedule list
            <div style="display: flex; flex-direction: column; gap: 8px;">
                {move || schedules.get().into_iter().map(|s| {
                    let sid = s.id.clone();
                    let sid2 = s.id.clone();
                    let enabled = s.enabled;
                    let type_color = match s.task_type.as_str() {
                        "shell" => "#ff3c14",
                        "chat" => "#3b82f6",
                        "heartbeat" => "#22c55e",
                        _ => "#eab308",
                    };
                    view! {
                        <Card>
                            <div style="display: flex; align-items: center; gap: 12px;">
                                <button
                                    on:click=move |_| toggle_schedule(sid.clone(), enabled)
                                    style={move || format!(
                                        "width: 40px; height: 22px; border-radius: 11px; border: none; cursor: pointer; position: relative; transition: background 0.2s; flex-shrink: 0; background: {};",
                                        if s.enabled { "#ff3c14" } else { "rgba(255,255,255,0.1)" }
                                    )}
                                >
                                    <div style={move || format!(
                                        "width: 16px; height: 16px; border-radius: 50%; background: white; position: absolute; top: 3px; transition: left 0.2s; left: {};",
                                        if s.enabled { "21px" } else { "3px" }
                                    )} />
                                </button>
                                <div style="flex: 1; min-width: 0;">
                                    <div style="display: flex; align-items: center; gap: 8px; margin-bottom: 4px;">
                                        <span style="font-size: 13px; font-weight: 500; color: rgba(255,245,240,0.9);">{s.name.clone()}</span>
                                        <Badge text={s.task_type.clone()} color=type_color.to_string() />
                                        <code style="font-family: 'Orbitron', monospace; font-size: 10px; color: rgba(255,245,240,0.6); background: rgba(255,255,255,0.03); padding: 2px 6px; border-radius: 3px;">
                                            {s.cron.clone()}
                                        </code>
                                    </div>
                                    <div style="display: flex; gap: 12px; font-size: 10px; color: rgba(255,245,240,0.5);">
                                        {(!s.task_payload.is_empty()).then(|| view! {
                                            <span style="overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 300px;">
                                                {s.task_payload.clone()}
                                            </span>
                                        })}
                                        {(!s.last_run.is_empty()).then(|| view! {
                                            <span>"Last: "{s.last_run.clone()}</span>
                                        })}
                                        {(!s.next_run.is_empty()).then(|| view! {
                                            <span>"Next: "{s.next_run.clone()}</span>
                                        })}
                                    </div>
                                </div>
                                <button
                                    on:click={
                                        let pid = s.id.clone();
                                        let is_en = s.enabled;
                                        move |_| {
                                            let pid = pid.clone();
                                            if is_en {
                                                spawn_local(async move {
                                                    let _ = api::pause_schedule(&pid).await;
                                                    if let Ok(s) = api::fetch_schedules().await {
                                                        total.set(s.total);
                                                        schedules.set(s.schedules);
                                                    }
                                                });
                                            } else {
                                                spawn_local(async move {
                                                    let _ = api::resume_schedule(&pid).await;
                                                    if let Ok(s) = api::fetch_schedules().await {
                                                        total.set(s.total);
                                                        schedules.set(s.schedules);
                                                    }
                                                });
                                            }
                                        }
                                    }
                                    style="font-family: 'Orbitron', monospace; font-size: 7px; letter-spacing: 1px; padding: 3px 8px; border-radius: 4px; cursor: pointer; background: rgba(255,60,20,0.08); border: 1px solid rgba(255,60,20,0.15); color: rgba(255,60,20,0.6);"
                                >{if s.enabled { "PAUSE" } else { "RESUME" }}</button>
                                <button
                                    on:click=move |_| delete_schedule(sid2.clone())
                                    style="background: none; border: none; color: #ef4444; cursor: pointer; font-size: 14px; padding: 4px 8px; opacity: 0.4;"
                                    title="Delete"
                                >"×"</button>
                            </div>
                        </Card>
                    }
                }).collect::<Vec<_>>()}
            </div>

            <Show when=move || !loading.get() && schedules.get().is_empty()>
                <Card>
                    <div style="text-align: center; padding: 32px;">
                        <div style="font-family: 'Orbitron', monospace; font-size: 11px; letter-spacing: 3px; color: rgba(255,245,240,0.5); margin-bottom: 8px;">"NO SCHEDULES"</div>
                        <div style="font-size: 12px; color: rgba(255,245,240,0.5);">"Create cron schedules for automated tasks, heartbeats, and memory syncs"</div>
                    </div>
                </Card>
            </Show>
        </div>
    }
}
