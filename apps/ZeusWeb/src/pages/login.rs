// ═══════════════════════════════════════════════════════════
// ZEUS — Login Page — Phase 2: Wired to API
// ═══════════════════════════════════════════════════════════

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;
use crate::api;
use crate::components::sentient_orb::SentientOrb;

/// Login page — centered orb with auth form
#[component]
pub fn LoginPage() -> impl IntoView {
    let token = RwSignal::new(String::new());
    let loading = RwSignal::new(false);
    let error = RwSignal::new(String::new());
    let success = RwSignal::new(false);

    let on_authenticate = move |_| {
        let t = token.get_untracked();
        if t.is_empty() {
            error.set("Please enter your API token".to_string());
            return;
        }
        loading.set(true);
        error.set(String::new());
        spawn_local(async move {
            match api::auth_token(&t).await {
                Ok(r) => {
                    if r.success {
                        success.set(true);
                        // Redirect to dashboard after short delay
                        let window = web_sys::window().unwrap();
                        let _ = window.location().set_href("/");
                    } else {
                        error.set(if r.message.is_empty() { "Authentication failed".to_string() } else { r.message });
                    }
                }
                Err(e) => { error.set(e); }
            }
            loading.set(false);
        });
    };

    let on_oauth = move |_| {
        loading.set(true);
        error.set(String::new());
        spawn_local(async move {
            match api::auth_login().await {
                Ok(r) => {
                    if !r.authorize_url.is_empty() {
                        let window = web_sys::window().unwrap();
                        let _ = window.location().set_href(&r.authorize_url);
                    } else if let Some(e) = r.error {
                        error.set(e);
                    } else {
                        error.set("OAuth not configured".to_string());
                    }
                }
                Err(e) => { error.set(e); }
            }
            loading.set(false);
        });
    };

    view! {
        <div style="display:flex;flex-direction:column;align-items:center;justify-content:center;min-height:100vh;background:#050508;padding:32px;">
            <SentientOrb size=120 mode="dormant" />
            <h1 style="font-family:'Orbitron',monospace;font-size:24px;font-weight:900;letter-spacing:8px;color:rgba(255,245,240,0.9);margin-top:24px;">"ZEUS"</h1>
            <p style="font-family:'Rajdhani',sans-serif;font-size:11px;letter-spacing:3px;color:rgba(255,245,240,0.5);margin-top:4px;text-transform:uppercase;">"Cognitive Platform"</p>
            <div style="margin-top:40px;width:320px;">
                <div style="margin-bottom:24px;">
                    <label style="font-family:'Orbitron',monospace;font-size:8px;letter-spacing:3px;color:rgba(255,245,240,0.7);text-transform:uppercase;display:block;margin-bottom:6px;">"API TOKEN"</label>
                    <input type="password" placeholder="Enter API token"
                        prop:value=move || token.get()
                        on:input=move |e| token.set(event_target_value(&e))
                        style="width:100%;padding:10px 14px;background:rgba(255,255,255,0.03);border:1px solid rgba(255,60,20,0.1);border-radius:8px;color:rgba(255,245,240,0.9);font-family:'Rajdhani',sans-serif;font-size:14px;outline:none;box-sizing:border-box;" />
                </div>
                {move || (!error.get().is_empty()).then(|| view! {
                    <div style="margin-bottom:12px;padding:8px 12px;background:rgba(239,68,68,0.1);border:1px solid rgba(239,68,68,0.3);border-radius:6px;color:#ef4444;font-size:12px;">
                        {error.get()}
                    </div>
                })}
                <button
                    disabled=move || loading.get()
                    on:click=on_authenticate
                    style="width:100%;padding:12px;background:rgba(255,60,20,0.15);border:1px solid rgba(255,60,20,0.5);border-radius:8px;color:rgba(255,140,80,1);font-family:'Orbitron',monospace;font-size:10px;letter-spacing:3px;text-transform:uppercase;cursor:pointer;margin-bottom:12px;">
                    {move || if loading.get() { "AUTHENTICATING..." } else { "Authenticate" }}
                </button>
                <button
                    disabled=move || loading.get()
                    on:click=on_oauth
                    style="width:100%;padding:10px;background:transparent;border:1px solid rgba(255,60,20,0.2);border-radius:8px;color:rgba(255,245,240,0.7);font-family:'Orbitron',monospace;font-size:9px;letter-spacing:2px;text-transform:uppercase;cursor:pointer;">
                    "OAuth Login"
                </button>
            </div>
        </div>
    }
}
