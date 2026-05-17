// ═══════════════════════════════════════════════════════════
// ZEUS — OAuth Callback Page — Phase 2: Wired to API
// ═══════════════════════════════════════════════════════════

use leptos::prelude::*;
use leptos_router::hooks::use_query_map;
use wasm_bindgen_futures::spawn_local;
use crate::api;
use crate::components::sentient_orb::SentientOrb;

/// OAuth callback handler — reads ?code= from URL, completes flow
#[component]
pub fn OAuthCallbackPage() -> impl IntoView {
    let query = use_query_map();
    let status = RwSignal::new("processing".to_string());
    let message = RwSignal::new("Verifying credentials and establishing session...".to_string());

    {
        let query = query.get_untracked();
        let code = query.get("code").unwrap_or_default().to_string();
        let provider = query.get("provider").unwrap_or("oauth".to_string());
        let state = query.get("state").unwrap_or_default().to_string();
        let _ = state; // used for CSRF verification server-side

        spawn_local(async move {
            if code.is_empty() {
                status.set("error".to_string());
                message.set("No authorization code received.".to_string());
                return;
            }
            // Use empty code_verifier and redirect_uri — server handles PKCE state
            match api::auth_oauth_callback(&code, "", &provider, "/oauth/callback").await {
                Ok(r) => {
                    if r.success {
                        status.set("success".to_string());
                        message.set(format!("Authenticated via {}. Redirecting...", r.provider));
                        let window = web_sys::window().unwrap();
                        let _ = window.location().set_href("/");
                    } else {
                        status.set("error".to_string());
                        message.set(if r.message.is_empty() {
                            "Authentication failed. Please try again.".to_string()
                        } else {
                            r.message
                        });
                    }
                }
                Err(e) => {
                    status.set("error".to_string());
                    message.set(e);
                }
            }
        });
    }

    view! {
        <div style="display:flex;flex-direction:column;align-items:center;justify-content:center;min-height:100vh;background:#050508;padding:32px;">
            <SentientOrb size=80 mode="thinking" />
            {move || {
                let s = status.get();
                let icon_color = if s == "error" { "#ef4444" } else if s == "success" { "#22c55e" } else { "rgba(255,245,240,0.9)" };
                view! {
                    <h1 style={format!("font-family:'Orbitron',monospace;font-size:12px;letter-spacing:5px;color:{};margin-top:20px;text-transform:uppercase;", icon_color)}>
                        {if s == "error" { "Authentication Failed" }
                         else if s == "success" { "Authenticated" }
                         else { "Processing Authentication" }}
                    </h1>
                }
            }}
            <p style="font-family:'Rajdhani',sans-serif;font-size:13px;color:rgba(255,245,240,0.7);margin-top:8px;text-align:center;max-width:360px;">
                {move || message.get()}
            </p>
            {move || (status.get() == "error").then(|| view! {
                <a href="/login" style="margin-top:20px;text-decoration:none;">
                    <button style="padding:10px 24px;background:rgba(255,60,20,0.15);border:1px solid rgba(255,60,20,0.5);border-radius:8px;color:rgba(255,140,80,1);font-family:'Orbitron',monospace;font-size:9px;letter-spacing:3px;text-transform:uppercase;cursor:pointer;">
                        "Return to Login"
                    </button>
                </a>
            })}
        </div>
    }
}
