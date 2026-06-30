// ═══════════════════════════════════════════════════════════
// ZEUS — Webhooks Manager — S21 Phase 4 P1
// Inbound triggers + outbound delivery management
// ═══════════════════════════════════════════════════════════

use crate::api;
use crate::components::design::*;
use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

#[component]
pub fn WebhooksPage() -> impl IntoView {
    let health = RwSignal::new(Option::<api::WebhookHealthResponse>::None);
    let triggers = RwSignal::new(Vec::<serde_json::Value>::new());
    let outbound = RwSignal::new(Vec::<api::OutboundWebhook>::new());
    let loading = RwSignal::new(true);
    let toast = RwSignal::new(String::new());
    let toast_ok = RwSignal::new(true);
    let active_tab = RwSignal::new(0u8); // 0=inbound 1=outbound
    // Inbound create form
    let show_trigger = RwSignal::new(false);
    let trig_event = RwSignal::new(String::new());
    let trig_source = RwSignal::new(String::new());
    let trig_action = RwSignal::new(String::new());
    let creating_trig = RwSignal::new(false);
    // Outbound create form
    let show_outbound = RwSignal::new(false);
    let out_url = RwSignal::new(String::new());
    let out_events = RwSignal::new(String::new());
    let out_secret = RwSignal::new(String::new());
    let creating_out = RwSignal::new(false);

    let set_toast = move |ok: bool, msg: String| {
        toast_ok.set(ok);
        toast.set(msg.clone());
        let toast = toast;
        spawn_local(async move {
            gloo_timers::future::TimeoutFuture::new(3000).await;
            if toast.get_untracked() == msg { toast.set(String::new()); }
        });
    };

    let reload = move || {
        loading.set(true);
        spawn_local(async move {
            if let Ok(h) = api::fetch_webhook_health().await { health.set(Some(h)); }
            if let Ok(t) = api::fetch_webhook_triggers().await { triggers.set(t.triggers); }
            if let Ok(o) = api::fetch_outbound_webhooks().await { outbound.set(o.webhooks); }
            loading.set(false);
        });
    };
    reload();

    let do_create_trigger = move |_: leptos::ev::MouseEvent| {
        let event = trig_event.get_untracked();
        let source = trig_source.get_untracked();
        let action = trig_action.get_untracked();
        if event.is_empty() { set_toast(false, "Event name required".into()); return; }
        creating_trig.set(true);
        spawn_local(async move {
            let body = serde_json::json!({ "event": event, "source": source, "action": action });
            match api::create_webhook_trigger(&body).await {
                Ok(_) => {
                    set_toast(true, "Trigger created".into());
                    trig_event.set(String::new());
                    trig_source.set(String::new());
                    trig_action.set(String::new());
                    show_trigger.set(false);
                    reload();
                }
                Err(e) => set_toast(false, format!("Error: {e}")),
            }
            creating_trig.set(false);
        });
    };

    let do_delete_trigger = move |id: String| {
        spawn_local(async move {
            if api::delete_webhook_trigger(&id).await.is_ok() {
                set_toast(true, "Trigger deleted".into());
                reload();
            } else {
                set_toast(false, "Delete failed".into());
            }
        });
    };

    let do_toggle_trigger = move |id: String, enabled: bool| {
        spawn_local(async move {
            let result = if enabled {
                api::disable_webhook_trigger(&id).await
            } else {
                api::enable_webhook_trigger(&id).await
            };
            if result.is_ok() {
                set_toast(true, if enabled { "Trigger disabled".into() } else { "Trigger enabled".into() });
                reload();
            }
        });
    };

    let do_create_outbound = move |_: leptos::ev::MouseEvent| {
        let url = out_url.get_untracked();
        let events_raw = out_events.get_untracked();
        if url.is_empty() { set_toast(false, "URL required".into()); return; }
        let events: Vec<String> = events_raw.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
        let secret = out_secret.get_untracked();
        creating_out.set(true);
        spawn_local(async move {
            let mut body = serde_json::json!({ "url": url, "events": events });
            if !secret.is_empty() { body["secret"] = serde_json::Value::String(secret); }
            match api::register_outbound_webhook(&body).await {
                Ok(w) => {
                    set_toast(true, format!("Registered: {}", w.url));
                    out_url.set(String::new());
                    out_events.set(String::new());
                    out_secret.set(String::new());
                    show_outbound.set(false);
                    reload();
                }
                Err(e) => set_toast(false, format!("Error: {e}")),
            }
            creating_out.set(false);
        });
    };

    let do_delete_outbound = move |id: String| {
        spawn_local(async move {
            if api::delete_outbound_webhook(&id).await.is_ok() {
                set_toast(true, "Webhook deleted".into());
                reload();
            } else {
                set_toast(false, "Delete failed".into());
            }
        });
    };

    view! {
        <div style="padding: 32px; max-width: 1100px; margin: 0 auto; font-family: 'Rajdhani', sans-serif; color: rgba(255,245,240,0.9);">
            // Header
            <div style="margin-bottom: 28px;">
                <h1 style="font-family: 'Orbitron', monospace; font-size: 20px; font-weight: 700; letter-spacing: 4px; color: rgba(255,245,240,0.9); margin: 0 0 4px 0;">"WEBHOOKS"</h1>
                <p style="font-size: 13px; color: rgba(255,245,240,0.5); margin: 0;">"Inbound event triggers and outbound delivery endpoints"</p>
            </div>

            // Toast
            {move || (!toast.get().is_empty()).then(|| view! {
                <div style=move || format!(
                    "margin-bottom: 16px; padding: 10px 16px; border-radius: 8px; font-size: 13px; background: {}; border: 1px solid {};",
                    if toast_ok.get() { "rgba(34,197,94,0.1)" } else { "rgba(239,68,68,0.1)" },
                    if toast_ok.get() { "rgba(34,197,94,0.3)" } else { "rgba(239,68,68,0.3)" }
                )>{toast.get()}</div>
            })}

            // Health status
            {move || health.get().map(|h| view! {
                <div style="background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.15); border-radius: 12px; padding: 16px 20px; margin-bottom: 20px; display: flex; gap: 24px; flex-wrap: wrap; align-items: center;">
                    <div>
                        <div style="font-size: 10px; color: rgba(255,245,240,0.4); text-transform: uppercase; letter-spacing: 1px; margin-bottom: 2px;">"Status"</div>
                        <span style=format!("font-size: 12px; font-weight: 700; color: {};",
                            if h.status == "ok" { "rgba(34,197,94,0.8)" } else { "rgba(239,68,68,0.8)" }
                        )>{h.status.to_uppercase()}</span>
                    </div>
                    <div>
                        <div style="font-size: 10px; color: rgba(255,245,240,0.4); text-transform: uppercase; letter-spacing: 1px; margin-bottom: 2px;">"Endpoint"</div>
                        <span style="font-family: monospace; font-size: 12px; color: rgba(255,245,240,0.7);">{h.endpoint}</span>
                    </div>
                    <div>
                        <div style="font-size: 10px; color: rgba(255,245,240,0.4); text-transform: uppercase; letter-spacing: 1px; margin-bottom: 2px;">"Content-Type"</div>
                        <span style="font-size: 12px; color: rgba(255,245,240,0.6);">{h.content_type}</span>
                    </div>
                    <div>
                        <div style="font-size: 10px; color: rgba(255,245,240,0.4); text-transform: uppercase; letter-spacing: 1px; margin-bottom: 2px;">"Signature Verify"</div>
                        <span style=format!("font-size: 12px; font-weight: 600; color: {};",
                            if h.signature_verification { "rgba(34,197,94,0.8)" } else { "rgba(234,179,8,0.8)" }
                        )>{if h.signature_verification { "ON" } else { "OFF" }}</span>
                    </div>
                    {(!h.accepts.is_empty()).then(|| view! {
                        <div>
                            <div style="font-size: 10px; color: rgba(255,245,240,0.4); text-transform: uppercase; letter-spacing: 1px; margin-bottom: 2px;">"Accepts"</div>
                            <span style="font-size: 12px; color: rgba(255,245,240,0.6);">{h.accepts.join(", ")}</span>
                        </div>
                    })}
                </div>
            })}

            // Tabs
            <div style="display: flex; gap: 4px; margin-bottom: 20px; background: rgba(255,255,255,0.02); border-radius: 10px; padding: 4px; width: fit-content;">
                {[("Inbound Triggers", 0u8), ("Outbound Webhooks", 1u8)].iter().map(|(label, idx)| {
                    let idx = *idx;
                    let label = *label;
                    view! {
                        <button
                            on:click=move |_| active_tab.set(idx)
                            style=move || format!(
                                "padding: 8px 18px; border-radius: 8px; border: none; cursor: pointer; font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; transition: all 0.15s; background: {}; color: {};",
                                if active_tab.get() == idx { "rgba(255,60,20,0.2)" } else { "transparent" },
                                if active_tab.get() == idx { "rgba(255,140,80,1)" } else { "rgba(255,245,240,0.5)" },
                            )
                        >{label}</button>
                    }
                }).collect_view()}
            </div>

            // ── INBOUND TRIGGERS TAB ──
            {move || (active_tab.get() == 0).then(|| view! {
                <div>
                    <div style="display: flex; justify-content: flex-end; margin-bottom: 16px;">
                        <Button primary=true on_click=Some(Callback::new(move |_| show_trigger.update(|v| *v = !*v)))>
                            <Icon name="plus" size=12 /> " ADD TRIGGER"
                        </Button>
                    </div>

                    {move || show_trigger.get().then(|| view! {
                        <div style="background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.15); border-radius: 12px; padding: 20px; margin-bottom: 16px;">
                            <div style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 2px; color: rgba(255,60,20,0.6); margin-bottom: 12px;">"NEW INBOUND TRIGGER"</div>
                            <div style="display: flex; gap: 10px; flex-wrap: wrap; margin-bottom: 10px;">
                                <input placeholder="Event name (e.g. push, release)"
                                    prop:value=move || trig_event.get()
                                    on:input=move |e| trig_event.set(event_target_value(&e))
                                    style="flex: 1; min-width: 160px; padding: 9px 12px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.2); border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 13px; outline: none;"
                                />
                                <input placeholder="Source (github, custom...)"
                                    prop:value=move || trig_source.get()
                                    on:input=move |e| trig_source.set(event_target_value(&e))
                                    style="flex: 1; min-width: 140px; padding: 9px 12px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.2); border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 13px; outline: none;"
                                />
                                <input placeholder="Action to execute"
                                    prop:value=move || trig_action.get()
                                    on:input=move |e| trig_action.set(event_target_value(&e))
                                    style="flex: 1; min-width: 140px; padding: 9px 12px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.2); border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 13px; outline: none;"
                                />
                            </div>
                            <Button primary=true on_click=Some(Callback::new(move |_| do_create_trigger(leptos::ev::MouseEvent::new("click").unwrap())))>
                                {move || if creating_trig.get() { "Creating..." } else { "Create Trigger" }}
                            </Button>
                        </div>
                    })}

                    {move || {
                        let ts = triggers.get();
                        if loading.get() {
                            view! { <div style="color: rgba(255,245,240,0.4); font-size: 13px;">"Loading..."</div> }.into_any()
                        } else if ts.is_empty() {
                            view! {
                                <div style="text-align: center; padding: 48px; color: rgba(255,245,240,0.3); border: 1px dashed rgba(255,60,20,0.1); border-radius: 12px;">
                                    <div style="font-size: 32px; margin-bottom: 12px;">"🔔"</div>
                                    <div>"No inbound triggers configured"</div>
                                </div>
                            }.into_any()
                        } else {
                            view! {
                                <div style="display: flex; flex-direction: column; gap: 8px;">
                                {ts.into_iter().map(|t| {
                                    let id = t.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                    let event = t.get("event").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                    let source = t.get("source").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                    let action = t.get("action").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                    let enabled = t.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true);
                                    let id_del = id.clone();
                                    let id_tog = id.clone();
                                    view! {
                                        <div style="display: flex; align-items: center; gap: 12px; padding: 14px 16px; background: rgba(255,255,255,0.02); border: 1px solid rgba(255,60,20,0.08); border-radius: 10px;">
                                            <div style="flex: 1; min-width: 0;">
                                                <div style="display: flex; gap: 8px; align-items: center; flex-wrap: wrap;">
                                                    <span style="font-weight: 600; font-size: 14px;">{event}</span>
                                                    {(!source.is_empty()).then(|| view! { <span style="font-size: 11px; color: rgba(255,60,20,0.6); padding: 1px 8px; background: rgba(255,60,20,0.08); border-radius: 4px;">{source}</span> })}
                                                    <span style=move || format!("font-size: 10px; padding: 2px 8px; border-radius: 20px; font-weight: 600; background: rgba(255,255,255,0.05); color: {};",
                                                        if enabled { "rgba(34,197,94,0.7)" } else { "rgba(255,245,240,0.3)" }
                                                    )>{if enabled { "ENABLED" } else { "DISABLED" }}</span>
                                                </div>
                                                {(!action.is_empty()).then(|| view! {
                                                    <div style="font-family: monospace; font-size: 11px; color: rgba(255,245,240,0.4); margin-top: 4px;">{action}</div>
                                                })}
                                            </div>
                                            <button on:click=move |_| do_toggle_trigger(id_tog.clone(), enabled)
                                                style="padding: 6px 12px; background: rgba(255,255,255,0.04); border: 1px solid rgba(255,255,255,0.08); border-radius: 6px; color: rgba(255,245,240,0.6); font-size: 11px; cursor: pointer;">
                                                {if enabled { "Disable" } else { "Enable" }}
                                            </button>
                                            <button on:click=move |_| do_delete_trigger(id_del.clone())
                                                style="padding: 6px 12px; background: rgba(239,68,68,0.08); border: 1px solid rgba(239,68,68,0.2); border-radius: 6px; color: rgba(239,68,68,0.7); font-size: 11px; cursor: pointer;">
                                                "Delete"
                                            </button>
                                        </div>
                                    }
                                }).collect_view()}
                                </div>
                            }.into_any()
                        }
                    }}
                </div>
            })}

            // ── OUTBOUND WEBHOOKS TAB ──
            {move || (active_tab.get() == 1).then(|| view! {
                <div>
                    <div style="display: flex; justify-content: flex-end; margin-bottom: 16px;">
                        <Button primary=true on_click=Some(Callback::new(move |_| show_outbound.update(|v| *v = !*v)))>
                            <Icon name="plus" size=12 /> " REGISTER ENDPOINT"
                        </Button>
                    </div>

                    {move || show_outbound.get().then(|| view! {
                        <div style="background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.15); border-radius: 12px; padding: 20px; margin-bottom: 16px;">
                            <div style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 2px; color: rgba(255,60,20,0.6); margin-bottom: 12px;">"NEW OUTBOUND WEBHOOK"</div>
                            <div style="display: flex; flex-direction: column; gap: 10px;">
                                <input placeholder="https://your-server.com/webhook"
                                    prop:value=move || out_url.get()
                                    on:input=move |e| out_url.set(event_target_value(&e))
                                    style="width: 100%; padding: 9px 12px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.2); border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 13px; outline: none; box-sizing: border-box;"
                                />
                                <input placeholder="Events (comma-separated: chat.message, tool.executed, mission.complete)"
                                    prop:value=move || out_events.get()
                                    on:input=move |e| out_events.set(event_target_value(&e))
                                    style="width: 100%; padding: 9px 12px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.2); border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 13px; outline: none; box-sizing: border-box;"
                                />
                                <input type="password" placeholder="HMAC secret (optional)"
                                    prop:value=move || out_secret.get()
                                    on:input=move |e| out_secret.set(event_target_value(&e))
                                    style="width: 100%; padding: 9px 12px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.2); border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 13px; outline: none; box-sizing: border-box;"
                                />
                                <Button primary=true on_click=Some(Callback::new(move |_| do_create_outbound(leptos::ev::MouseEvent::new("click").unwrap())))>
                                    {move || if creating_out.get() { "Registering..." } else { "Register" }}
                                </Button>
                            </div>
                        </div>
                    })}

                    {move || {
                        let ow = outbound.get();
                        if ow.is_empty() {
                            view! {
                                <div style="text-align: center; padding: 48px; color: rgba(255,245,240,0.3); border: 1px dashed rgba(255,60,20,0.1); border-radius: 12px;">
                                    <div style="font-size: 32px; margin-bottom: 12px;">"📡"</div>
                                    <div>"No outbound webhooks registered"</div>
                                </div>
                            }.into_any()
                        } else {
                            view! {
                                <div style="display: flex; flex-direction: column; gap: 8px;">
                                {ow.into_iter().map(|w| {
                                    let id = w.id.clone();
                                    view! {
                                        <div style="padding: 16px; background: rgba(255,255,255,0.02); border: 1px solid rgba(255,60,20,0.08); border-radius: 10px;">
                                            <div style="display: flex; justify-content: space-between; align-items: flex-start; margin-bottom: 8px;">
                                                <div style="font-family: monospace; font-size: 13px; color: rgba(255,245,240,0.8); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; flex: 1; margin-right: 12px;">{w.url}</div>
                                                <div style="display: flex; gap: 8px; align-items: center; flex-shrink: 0;">
                                                    <span style=format!("font-size: 10px; padding: 2px 8px; border-radius: 20px; font-weight: 600; background: rgba(255,255,255,0.05); color: {};",
                                                        if w.enabled { "rgba(34,197,94,0.7)" } else { "rgba(255,245,240,0.3)" }
                                                    )>{if w.enabled { "ACTIVE" } else { "DISABLED" }}</span>
                                                    <button on:click=move |_| do_delete_outbound(id.clone())
                                                        style="padding: 5px 10px; background: rgba(239,68,68,0.08); border: 1px solid rgba(239,68,68,0.2); border-radius: 6px; color: rgba(239,68,68,0.7); font-size: 11px; cursor: pointer;">
                                                        "Remove"
                                                    </button>
                                                </div>
                                            </div>
                                            <div style="display: flex; gap: 8px; flex-wrap: wrap; margin-bottom: 6px;">
                                            {w.events.iter().map(|ev| view! {
                                                <span style="font-size: 10px; padding: 2px 8px; border-radius: 4px; background: rgba(255,60,20,0.08); color: rgba(255,60,20,0.6);">{ev.clone()}</span>
                                            }).collect_view()}
                                            </div>
                                            <div style="display: flex; gap: 16px; font-size: 11px; color: rgba(255,245,240,0.4);">
                                                {(w.failure_count > 0).then(|| view! {
                                                    <span style="color: rgba(239,68,68,0.6);">{format!("{} failures", w.failure_count)}</span>
                                                })}
                                                {w.last_triggered_at.as_ref().map(|t| view! {
                                                    <span>"Last: "{t.clone()}</span>
                                                })}
                                            </div>
                                        </div>
                                    }
                                }).collect_view()}
                                </div>
                            }.into_any()
                        }
                    }}
                </div>
            })}
        </div>
    }
}
