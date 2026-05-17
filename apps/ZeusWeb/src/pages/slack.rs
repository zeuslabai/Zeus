use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;
use crate::api;
use crate::components::design::*;

#[component]
pub fn SlackPage() -> impl IntoView {
    let channel_id   = RwSignal::new(String::new());
    let bot_token    = RwSignal::new(String::new());
    let app_token    = RwSignal::new(String::new());
    let is_connected = RwSignal::new(false);
    let loaded       = RwSignal::new(false);
    let saving       = RwSignal::new(false);
    let testing      = RwSignal::new(false);
    let msg          = RwSignal::new(String::new());
    let is_err       = RwSignal::new(false);
    let test_result  = RwSignal::new(String::new());

    spawn_local(async move {
        if let Ok(r) = api::fetch_channels().await
            && let Some(ch) = r.channels.iter().find(|c| c.channel_type == "slack" || c.platform == "slack") {
                channel_id.set(ch.id.clone());
                is_connected.set(ch.status == "connected");
                bot_token.set(ch.config["bot_token"].as_str().unwrap_or("").to_string());
                app_token.set(ch.config["app_token"].as_str().unwrap_or("").to_string());
            }
        loaded.set(true);
    });

    let do_save = move |_| {
        let bt = bot_token.get_untracked();
        if bt.trim().is_empty() { is_err.set(true); msg.set("Bot token is required".into()); return; }
        let at = app_token.get_untracked();
        let cid = channel_id.get_untracked();
        saving.set(true); is_err.set(false); msg.set(String::new());
        spawn_local(async move {
            let config = serde_json::json!({ "bot_token": bt, "app_token": at });
            let result = if cid.is_empty() {
                api::create_channel(&api::CreateChannelReq { channel_type: "slack".into(), name: "Slack".into(), config }).await.map(|_| ())
            } else {
                api::update_channel(&cid, &api::UpdateChannelReq { config: Some(config), name: None, enabled: None }).await.map(|_| ())
            };
            match result {
                Ok(_)  => { msg.set("Saved".into()); is_err.set(false); }
                Err(e) => { msg.set(format!("Error: {}", e)); is_err.set(true); }
            }
            saving.set(false);
        });
    };

    let do_test = move |_| {
        let cid = channel_id.get_untracked();
        if cid.is_empty() { test_result.set("Save configuration first".into()); return; }
        testing.set(true); test_result.set(String::new());
        spawn_local(async move {
            match api::test_channel(&cid).await {
                Ok(r)  => test_result.set(format!("{} ({}ms)", if r.success { "Connected \u{2713}" } else { "Failed \u{2717}" }, r.latency_ms)),
                Err(e) => test_result.set(format!("Error: {}", e)),
            }
            testing.set(false);
        });
    };

    let input_style = "width: 100%; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.15); border-radius: 8px; padding: 10px 14px; color: rgba(255,245,240,0.9); font-family: 'Rajdhani', sans-serif; font-size: 14px; box-sizing: border-box; outline: none;";
    let label_style = "font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,245,240,0.7); margin-bottom: 6px;";

    view! {
        <div style="padding: 32px; max-width: 640px;">
            <div style="display: flex; align-items: center; gap: 16px; margin-bottom: 32px;">
                <button style="font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 2px; text-transform: uppercase; background: transparent; border: 1px solid rgba(255,60,20,0.1); color: rgba(255,245,240,0.7); padding: 6px 12px; border-radius: 6px; cursor: pointer;"
                    on:click=move |_| { let _ = web_sys::window().unwrap().location().assign("/channels"); }
                >"\u{2190} Channels"</button>
                <div>
                    <h1 style="font-family: 'Orbitron', monospace; font-size: 14px; letter-spacing: 6px; color: rgba(255,245,240,0.9); margin: 0;">"SLACK"</h1>
                    <p style="font-family: 'Rajdhani', sans-serif; font-size: 13px; color: rgba(255,245,240,0.7); margin: 4px 0 0;">{move || {
                        if !loaded.get() { "Loading...".to_string() }
                        else if is_connected.get() { "Connected \u{2014} Workspace active".to_string() }
                        else if !channel_id.get().is_empty() { "Configured \u{2014} Not connected".to_string() }
                        else { "Not configured".to_string() }
                    }}</p>
                </div>
                <Show when=move || is_connected.get()><StatusDot status="connected".to_string() /></Show>
            </div>
            <Show when=move || loaded.get()>
                <Card>
                    <div style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 3px; color: rgba(255,245,240,0.7); margin-bottom: 20px;">"APP CONFIGURATION"</div>
                    <div style="display: flex; flex-direction: column; gap: 16px;">
                        <div>
                            <div style=label_style>"BOT TOKEN *"</div>
                            <input type="password" placeholder="xoxb-..." style=input_style
                                prop:value=move || bot_token.get()
                                on:input=move |ev| bot_token.set(event_target_value(&ev))
                            />
                            <div style="font-family: 'Rajdhani', sans-serif; font-size: 12px; color: rgba(255,245,240,0.5); margin-top: 4px;">"OAuth &amp; Permissions \u{2192} Bot User OAuth Token"</div>
                        </div>
                        <div>
                            <div style=label_style>"APP TOKEN"</div>
                            <input type="password" placeholder="xapp-..." style=input_style
                                prop:value=move || app_token.get()
                                on:input=move |ev| app_token.set(event_target_value(&ev))
                            />
                            <div style="font-family: 'Rajdhani', sans-serif; font-size: 12px; color: rgba(255,245,240,0.5); margin-top: 4px;">"Required for Socket Mode (connections:write scope)"</div>
                        </div>
                    </div>
                    <Show when=move || !msg.get().is_empty()>
                        <div style={move || format!("margin-top: 12px; font-family: 'Rajdhani', sans-serif; font-size: 13px; color: {};", if is_err.get() { "rgba(255,60,20,0.9)" } else { "rgba(80,200,120,0.9)" })}>{move || msg.get()}</div>
                    </Show>
                    <Show when=move || !test_result.get().is_empty()>
                        <div style="margin-top: 8px; font-family: 'Rajdhani', sans-serif; font-size: 13px; color: rgba(255,245,240,0.7);">{move || test_result.get()}</div>
                    </Show>
                    <div style="display: flex; gap: 10px; margin-top: 24px; padding-top: 20px; border-top: 1px solid rgba(255,60,20,0.15);">
                        <Show when=move || !channel_id.get().is_empty()>
                            <Button on_click=Some(Callback::new(do_test))>{move || if testing.get() { "Testing..." } else { "Test Connection" }}</Button>
                        </Show>
                        <div style="flex: 1;"></div>
                        <Button primary=true on_click=Some(Callback::new(do_save))>{move || if saving.get() { "Saving..." } else { "Save" }}</Button>
                    </div>
                </Card>
            </Show>
        </div>
    }
}
