use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;
use crate::api;
use crate::components::design::*;

#[component]
pub fn SignalPage() -> impl IntoView {
    let channel_id = RwSignal::new(String::new());
    let phone = RwSignal::new(String::new());
    let is_connected = RwSignal::new(false);
    let loaded = RwSignal::new(false);
    let saving = RwSignal::new(false);
    let testing = RwSignal::new(false);
    let msg = RwSignal::new(String::new());
    let is_err = RwSignal::new(false);
    let test_result = RwSignal::new(String::new());

    spawn_local(async move {
        if let Ok(r) = api::fetch_channels().await
            && let Some(ch) = r.channels.iter().find(|c| c.channel_type == "signal" || c.platform == "signal") {
                channel_id.set(ch.id.clone());
                is_connected.set(ch.status == "connected");
                if let Some(v) = ch.config["phone"].as_str() {
                    phone.set(v.to_string());
                }
            }
        loaded.set(true);
    });

    let do_save = move |_| {
        let cid = channel_id.get_untracked();
        let ph = phone.get_untracked();
        saving.set(true);
        msg.set(String::new());
        is_err.set(false);
        spawn_local(async move {
            let config = serde_json::json!({ "phone": ph });
            let result = if cid.is_empty() {
                api::create_channel(&api::CreateChannelReq {
                    channel_type: "signal".into(),
                    name: "Signal".into(),
                    config,
                }).await.map(|_| ())
            } else {
                api::update_channel(&cid, &api::UpdateChannelReq {
                    config: Some(config),
                    name: None,
                    enabled: None,
                }).await.map(|_| ())
            };
            match result {
                Ok(_) => { msg.set("Configuration saved.".into()); }
                Err(e) => { msg.set(e.to_string()); is_err.set(true); }
            }
            saving.set(false);
        });
    };

    let do_test = move |_| {
        let cid = channel_id.get_untracked();
        if cid.is_empty() { return; }
        testing.set(true);
        test_result.set(String::new());
        spawn_local(async move {
            match api::test_channel(&cid).await {
                Ok(r) => {
                    let s = if r.success {
                        format!("Connected \u{2713} ({}ms)", r.latency_ms)
                    } else {
                        format!("Failed \u{2717} \u{2014} {}", r.message)
                    };
                    test_result.set(s);
                }
                Err(e) => test_result.set(format!("Error: {}", e)),
            }
            testing.set(false);
        });
    };

    view! {
        <div style="min-height: 100vh; background: #0a0a0a; padding: 32px; font-family: Rajdhani, sans-serif;">
            <div style="max-width: 640px; margin: 0 auto;">
                // Header
                <div style="display: flex; align-items: center; gap: 12px; margin-bottom: 32px;">
                    <button
                        on:click=move |_| { let _ = web_sys::window().unwrap().location().assign("/channels"); }
                        style="background: none; border: none; color: rgba(255,245,240,0.7); font-size: 20px; cursor: pointer; padding: 0;"
                    >"\u{2190}"</button>
                    <div>
                        <h1 style="margin: 0; font-family: 'Orbitron', monospace; font-size: 14px; letter-spacing: 6px; color: rgba(255,245,240,0.9);">
                            "SIGNAL"
                        </h1>
                        <p style="margin: 0; font-size: 13px; color: rgba(255,245,240,0.7);">
                            "Signal messenger via signal-cli daemon"
                        </p>
                    </div>
                    <div style="margin-left: auto;">
                        <Show when=move || is_connected.get()>
                            <span style="padding: 4px 10px; border-radius: 4px; font-size: 11px; font-family: 'Orbitron', monospace; letter-spacing: 0.05em; background: rgba(0,255,100,0.1); color: rgba(0,255,100,0.8); border: 1px solid rgba(0,255,100,0.2);">
                                "CONNECTED"
                            </span>
                        </Show>
                        <Show when=move || !is_connected.get()>
                            <span style="padding: 4px 10px; border-radius: 4px; font-size: 11px; font-family: 'Orbitron', monospace; letter-spacing: 0.05em; background: rgba(255,255,255,0.03); color: rgba(255,245,240,0.5); border: 1px solid rgba(255,60,20,0.1);">
                                "INACTIVE"
                            </span>
                        </Show>
                    </div>
                </div>

                // Info card
                <div style="background: rgba(0,180,255,0.04); border: 1px solid rgba(0,180,255,0.12); border-radius: 8px; padding: 16px 20px; margin-bottom: 24px;">
                    <div style="display: flex; align-items: center; gap: 8px; margin-bottom: 8px;">
                        <span style="font-size: 15px;">"\u{2139}"</span>
                        <span style="font-family: 'Orbitron', monospace; font-size: 12px; font-weight: 700; color: rgba(0,180,255,0.8); letter-spacing: 0.08em;">"SETUP REQUIRED"</span>
                    </div>
                    <p style="margin: 0; color: rgba(255,245,240,0.5); font-size: 13px; line-height: 1.7;">
                        "signal-cli must be installed and the phone number registered/linked."
                    </p>
                </div>

                // Config card
                <div style="background: rgba(255,245,240,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 8px; padding: 24px; margin-bottom: 24px;">
                    <div>
                        <label style="display: block; font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,245,240,0.7); margin-bottom: 8px;">
                            "PHONE NUMBER"
                        </label>
                        <input
                            type="text"
                            placeholder="+15551234567"
                            prop:value=move || phone.get()
                            on:input=move |e| phone.set(event_target_value(&e))
                            style="width: 100%; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 6px; padding: 10px 14px; color: rgba(255,245,240,0.9); font-family: monospace; font-size: 15px; box-sizing: border-box; outline: none;"
                        />
                        <p style="margin: 6px 0 0; font-size: 12px; color: rgba(255,245,240,0.5);">
                            "Include country code (e.g. +1 for US)"
                        </p>
                    </div>
                </div>

                // Action buttons
                <div style="display: flex; gap: 12px;">
                    <div style="flex: 1;">
                        <Button primary=true on_click=Some(Callback::new(do_save))>
                            {move || if saving.get() { "Saving..." } else { "Save Configuration" }}
                        </Button>
                    </div>
                    <Show when=move || !channel_id.get().is_empty()>
                        <Button on_click=Some(Callback::new(do_test))>
                            {move || if testing.get() { "Testing..." } else { "Test" }}
                        </Button>
                    </Show>
                </div>

                // Feedback
                <Show when=move || !msg.get().is_empty()>
                    <div style=move || format!(
                        "margin-top: 16px; padding: 12px 16px; border-radius: 6px; font-size: 13px; {}",
                        if is_err.get() {
                            "background: rgba(255,60,20,0.15); border: 1px solid rgba(255,60,20,0.2); color: rgba(255,100,60,0.9);"
                        } else {
                            "background: rgba(0,255,100,0.05); border: 1px solid rgba(0,255,100,0.15); color: rgba(0,255,100,0.8);"
                        }
                    )>
                        {move || msg.get()}
                    </div>
                </Show>

                <Show when=move || !test_result.get().is_empty()>
                    <div style="margin-top: 12px; padding: 12px 16px; border-radius: 6px; font-size: 13px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); color: rgba(255,245,240,0.5);">
                        {move || test_result.get()}
                    </div>
                </Show>
            </div>
        </div>
    }
}
