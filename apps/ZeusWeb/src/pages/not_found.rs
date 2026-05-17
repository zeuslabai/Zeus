use leptos::prelude::*;
use crate::components::sentient_orb::SentientOrb;

/// 404 Not Found page
#[component]
pub fn NotFoundPage() -> impl IntoView {
    view! {
        <div style="display:flex;flex-direction:column;align-items:center;justify-content:center;min-height:60vh;padding:32px;">
            <SentientOrb size=100 mode="dormant" />
            <h1 style="font-family:'Orbitron',monospace;font-size:48px;font-weight:900;color:#ff3c14;margin-top:24px;">404</h1>
            <p style="font-family:'Orbitron',monospace;font-size:12px;letter-spacing:5px;color:rgba(255,245,240,0.7);margin-top:8px;text-transform:uppercase;">Signal Lost</p>
            <p style="font-family:'Rajdhani',sans-serif;font-size:14px;color:rgba(255,245,240,0.5);margin-top:12px;text-align:center;max-width:400px;">
                "The requested neural pathway does not exist. Zeus cannot locate this resource."
            </p>
            <a href="/" style="margin-top:24px;font-family:'Orbitron',monospace;font-size:9px;letter-spacing:2px;text-transform:uppercase;color:rgba(255,140,80,1);background:rgba(255,60,20,0.15);border:1px solid rgba(255,60,20,0.5);padding:10px 20px;border-radius:6px;text-decoration:none;cursor:pointer;">
                "Return to Command Center"
            </a>
        </div>
    }
}
