use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;
use crate::api;
use crate::components::design::*;

#[component]
pub fn IMessagePage() -> impl IntoView {
    let channel_id = RwSignal::new(String::new());
    let is_connected = RwSignal::new(false);
    let loaded = RwSignal::new(false);
    let saving = RwSignal::new(false);
    let testing = RwSignal::new(false);
    let msg = RwSignal::new(String::new());
    let is_err = RwSignal::new(false);
    let test_result = RwSignal::new(String::new());

    spawn_local(async move {
        if let Ok(r) = api::fetch_channels().await
            && let Some(ch) = r.channels.iter().find(|c| c.channel_type == "imessage" || c.platform == "imessage") {
                channel_id.set(ch.id.clone());
                is_connected.set(ch.status == "connected");
            }
        loaded.set(true);
    });

    let do_enable = move |_| {
        let cid = channel_id.get_untracked();
        saving.set(true);
        msg.set(String::new());
        is_err.set(false);
        spawn_local(async move {
            let config = serde_json::json!({});
            let result = if cid.is_empty() {
                api::create_channel(&api::CreateChannelReq {
                    channel_type: "imessage".into(),
                    name: "iMessage".into(),
                    config,
                }).await.map(|_| ())
            } else {
                api::update_channel(&cid, &api::UpdateChannelReq {
                    config: Some(config),
                    name: None,
                    enabled: Some(true),
                }).await.map(|_| ())
            };
            match result {
                Ok(_) => { msg.set("iMessage enabled.".into()); is_connected.set(true); }
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
                            "IMESSAGE"
                        </h1>
                        <p style="margin: 0; font-size: 13px; color: rgba(255,245,240,0.7);">
                            "Apple iMessage via AppleScript bridge"
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

                // Requirements card
                <div style="background: rgba(255,150,0,0.06); border: 1px solid rgba(255,150,0,0.15); border-radius: 8px; padding: 20px; margin-bottom: 24px;">
                    <div style="display: flex; align-items: center; gap: 8px; margin-bottom: 12px;">
                        <span style="font-size: 16px;">"\u{26A0}"</span>
                        <span style="font-family: 'Orbitron', monospace; font-size: 13px; font-weight: 700; color: rgba(255,150,0,0.9); letter-spacing: 0.08em;">"REQUIREMENTS"</span>
                    </div>
                    <ul style="margin: 0; padding-left: 20px; color: rgba(255,245,240,0.6); font-size: 14px; line-height: 1.8;">
                        <li>"Requires macOS \u{2014} not available on FreeBSD or Linux"</li>
                        <li>"Zeus must run on a Mac with Messages.app signed in"</li>
                        <li>"AppleScript automation permission required"</li>
                        <li>"iMessage will be sent from your Apple ID"</li>
                    </ul>
                </div>

                // Info card
                <div style="background: rgba(255,245,240,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 8px; padding: 20px; margin-bottom: 24px;">
                    <p style="margin: 0; color: rgba(255,245,240,0.6); font-size: 14px; line-height: 1.7;">
                        "iMessage integration uses AppleScript to interface with Messages.app on macOS. "
                        "No API keys or configuration are required \u{2014} Zeus bridges directly to the native app. "
                        "Outgoing messages appear as sent from your Apple ID."
                    </p>
                </div>

                // Action buttons
                <div style="display: flex; flex-direction: column; gap: 12px;">
                    <Button
                        primary=true
                        on_click=Some(Callback::new(do_enable))
                    >
                        {move || if saving.get() { "Enabling..." } else if is_connected.get() { "Re-enable iMessage" } else { "Enable iMessage" }}
                    </Button>

                    <Show when=move || !channel_id.get().is_empty()>
                        <Button on_click=Some(Callback::new(do_test))>
                            {move || if testing.get() { "Testing..." } else { "Test Connection" }}
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
