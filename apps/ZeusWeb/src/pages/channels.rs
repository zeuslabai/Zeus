// ═══════════════════════════════════════════════════════════
// ZEUS — Channels Page — Phase 3: Navigate + add-modal
// ═══════════════════════════════════════════════════════════

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;
use crate::api;
use crate::components::design::*;

struct ChannelType {
    id:    &'static str,
    label: &'static str,
    desc:  &'static str,
}

const CHANNEL_TYPES: &[ChannelType] = &[
    ChannelType { id: "telegram",  label: "TELEGRAM",  desc: "Bot API" },
    ChannelType { id: "discord",   label: "DISCORD",   desc: "Bot gateway" },
    ChannelType { id: "slack",     label: "SLACK",     desc: "Socket Mode" },
    ChannelType { id: "email",     label: "EMAIL",     desc: "IMAP / SMTP" },
    ChannelType { id: "imessage",  label: "IMESSAGE",  desc: "macOS only" },
    ChannelType { id: "whatsapp",  label: "WHATSAPP",  desc: "Cloud API" },
    ChannelType { id: "signal",    label: "SIGNAL",    desc: "signal-cli" },
    ChannelType { id: "matrix",    label: "MATRIX",    desc: "Native SDK" },
    ChannelType { id: "instagram", label: "INSTAGRAM", desc: "Graph API" },
    ChannelType { id: "tiktok",    label: "TIKTOK",    desc: "Post-only" },
];

#[component]
pub fn ChannelsPage() -> impl IntoView {
    let channels = RwSignal::new(Vec::<api::Channel>::new());
    let loading  = RwSignal::new(true);
    let show     = RwSignal::new(false);

    {
        let channels = channels;
        let loading  = loading;
        spawn_local(async move {
            if let Ok(c) = api::fetch_channels().await { channels.set(c.channels); }
            loading.set(false);
        });
    }

    let reload_channels = move || {
        spawn_local(async move {
            if let Ok(c) = api::fetch_channels().await { channels.set(c.channels); }
        });
    };

    view! {
        // ── Add Channel modal ────────────────────────────────
        <Show when=move || show.get()>
            <div style="position: fixed; inset: 0; background: rgba(0,0,0,0.78); z-index: 1000; display: flex; align-items: center; justify-content: center;">
                <div style="background: #0d0704; border: 1px solid rgba(255,60,20,0.25); border-radius: 16px; padding: 32px; width: 560px; max-width: 92vw; box-shadow: 0 0 60px rgba(255,60,20,0.15);">
                    <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 20px;">
                        <div style="font-family: 'Orbitron', monospace; font-size: 11px; letter-spacing: 5px; color: rgba(255,245,240,0.9);">"ADD CHANNEL"</div>
                        <button
                            style="background: transparent; border: none; color: rgba(255,245,240,0.7); font-size: 18px; cursor: pointer; padding: 0 4px;"
                            on:click=move |_| show.set(false)
                        >"\u{00D7}"</button>
                    </div>
                    <div style="font-family: 'Rajdhani', sans-serif; font-size: 13px; color: rgba(255,245,240,0.7); margin-bottom: 20px; line-height: 1.6;">"Select a platform — you'll be taken to its settings page."</div>
                    <div style="display: grid; grid-template-columns: 1fr 1fr; gap: 10px;">
                        {CHANNEL_TYPES.iter().map(|ct| {
                            let path = format!("/channels/{}", ct.id);
                            view! {
                                <button
                                    style="background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 10px; padding: 14px 16px; text-align: left; cursor: pointer;"
                                    on:click=move |_| { let _ = web_sys::window().unwrap().location().assign(&path); }
                                >
                                    <div style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 3px; color: rgba(255,245,240,0.9); margin-bottom: 4px;">{ct.label}</div>
                                    <div style="font-family: 'Rajdhani', sans-serif; font-size: 12px; color: rgba(255,245,240,0.7);">{ct.desc}</div>
                                </button>
                            }
                        }).collect::<Vec<_>>()}
                    </div>
                </div>
            </div>
        </Show>

        // ── Page ─────────────────────────────────────────────
        <div style="padding: 32px;">
            <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 24px;">
                <div>
                    <h1 style="font-family: 'Orbitron', monospace; font-size: 14px; letter-spacing: 6px; color: rgba(255,245,240,0.9); margin: 0;">"CHANNELS"</h1>
                    <p style="font-size: 12px; color: rgba(255,245,240,0.7); margin: 4px 0 0;">{move || {
                        let ch = channels.get();
                        let connected = ch.iter().filter(|c| c.status == "connected").count();
                        if loading.get()      { "Loading channels...".to_string() }
                        else if ch.is_empty() { "No channels configured".to_string() }
                        else { format!("{} adapters \u{2022} {} connected", ch.len(), connected) }
                    }}</p>
                </div>
                <Button primary=true on_click=Some(Callback::new(move |_| show.set(true)))>
                    <Icon name="plus" size=12 /> " Add Channel"
                </Button>
            </div>

            {move || {
                if !loading.get() && channels.get().is_empty() {
                    view! {
                        <div style="display: flex; flex-direction: column; align-items: center; justify-content: center; min-height: 300px; gap: 16px;">
                            <div style="width: 56px; height: 56px; border-radius: 14px; background: rgba(255,60,20,0.15); border: 1px solid rgba(255,60,20,0.1); display: flex; align-items: center; justify-content: center;">
                                <Icon name="channels" size=24 color="rgba(255,60,20,0.6)".to_string() />
                            </div>
                            <div style="text-align: center;">
                                <div style="font-family: 'Orbitron', monospace; font-size: 12px; letter-spacing: 4px; color: rgba(255,245,240,0.9); margin-bottom: 8px;">"NO CHANNELS CONFIGURED"</div>
                                <div style="font-family: 'Rajdhani', sans-serif; font-size: 13px; color: rgba(255,245,240,0.7); max-width: 360px; line-height: 1.6;">"Connect Zeus to Telegram or Discord to get started."</div>
                            </div>
                            <Button primary=true on_click=Some(Callback::new(move |_| show.set(true)))>
                                <Icon name="plus" size=12 /> " Connect First Channel"
                            </Button>
                        </div>
                    }.into_any()
                } else {
                    view! {
                        <div style="display: grid; grid-template-columns: repeat(auto-fill, minmax(280px, 1fr)); gap: 16px;">
                            {move || channels.get().into_iter().map(|c| {
                                let is_connected = c.status == "connected";
                                let status_str   = c.status.clone();
                                let status_label = c.status.clone();
                                let ch_type      = if c.channel_type.is_empty() { c.platform.clone() } else { c.channel_type.clone() };
                                view! {
                                    <Card glow=is_connected>
                                        <div style="display: flex; align-items: center; gap: 12px; margin-bottom: 14px;">
                                            <div style={format!("width: 40px; height: 40px; border-radius: 10px; display: flex; align-items: center; justify-content: center; background: {};",
                                                if is_connected { "rgba(255,60,20,0.15)" } else { "rgba(255,255,255,0.03)" }
                                            )}>
                                                <Icon name="channels" size=18 color={if is_connected { "rgba(255,60,20,0.6)" } else { "rgba(255,245,240,0.5)" }.to_string()} />
                                            </div>
                                            <div style="flex: 1;">
                                                <div style="font-family: 'Rajdhani', sans-serif; font-size: 14px; font-weight: 600; color: rgba(255,245,240,0.9);">{c.name.clone()}</div>
                                                <div style="display: flex; align-items: center; gap: 6px; margin-top: 2px;">
                                                    <StatusDot status=status_str />
                                                    <span style="font-family: 'Rajdhani', sans-serif; font-size: 11px; color: rgba(255,245,240,0.7);">{status_label}</span>
                                                </div>
                                            </div>
                                        </div>
                                        <div style="display: flex; justify-content: space-between; align-items: center; padding: 10px 0; border-top: 1px solid rgba(255,60,20,0.1);">
                                            <span style="font-family: 'Rajdhani', sans-serif; font-size: 11px; color: rgba(255,245,240,0.7);">{c.message_count.to_string()}" messages"</span>
                                            <div style="display: flex; gap: 6px; align-items: center;">
                                                <button
                                                    style="font-family: 'Orbitron', monospace; font-size: 7px; letter-spacing: 1px; padding: 4px 8px; border-radius: 5px; cursor: pointer; background: rgba(239,68,68,0.08); border: 1px solid rgba(239,68,68,0.15); color: rgba(239,68,68,0.6);"
                                                    on:click={
                                                        let cid = c.id.clone();
                                                        move |_| {
                                                            let cid = cid.clone();
                                                            spawn_local(async move {
                                                                if let Err(e) = api::delete_channel(&cid).await { web_sys::console::error_1(&format!("API error: {}", e).into()); }
                                                                reload_channels();
                                                            });
                                                        }
                                                    }
                                                >"DEL"</button>
                                                <button
                                                    style={format!("font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 2px; text-transform: uppercase; padding: 4px 10px; border-radius: 6px; cursor: pointer; background: {}; border: {}; color: {};",
                                                        if is_connected { "rgba(255,60,20,0.15)" } else { "transparent" },
                                                        if is_connected { "1px solid rgba(255,60,20,0.5)" } else { "1px solid rgba(255,60,20,0.1)" },
                                                        if is_connected { "rgba(255,140,80,1)" } else { "rgba(255,245,240,0.7)" }
                                                    )}
                                                    on:click={
                                                        let cid2 = c.id.clone();
                                                        let ch_type2 = ch_type.clone();
                                                        let is_conn = is_connected;
                                                        move |_| {
                                                            let cid2 = cid2.clone();
                                                            let ch_type2 = ch_type2.clone();
                                                            if is_conn {
                                                                spawn_local(async move {
                                                                    if let Err(e) = api::disconnect_channel(&cid2).await { web_sys::console::error_1(&format!("API error: {}", e).into()); }
                                                                    reload_channels();
                                                                });
                                                            } else {
                                                                let url = format!("/channels/{}", ch_type2);
                                                                let _ = web_sys::window().unwrap().location().assign(&url);
                                                            }
                                                        }
                                                    }
                                                >
                                                    {if is_connected { "Disconnect" } else { "Connect" }}
                                                </button>
                                            </div>
                                        </div>
                                    </Card>
                                }
                            }).collect::<Vec<_>>()}
                        </div>
                    }.into_any()
                }
            }}
        </div>
    }
}
