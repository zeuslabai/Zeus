// Simple redirect to /onboarding
use leptos::prelude::*;

#[component]
pub fn OnboardPage() -> impl IntoView {
    Effect::new(move |_| {
        if let Some(win) = web_sys::window() {
            let _ = win.location().set_href("/onboarding");
        }
    });

    view! {
        <div style="min-height: 100vh; background: #050508; display: flex; align-items: center; justify-content: center;">
            <div style="font-family: 'Orbitron', monospace; font-size: 11px; letter-spacing: 4px; color: rgba(255,245,240,0.7);">
                "Redirecting to onboarding..."
            </div>
        </div>
    }
}
