// ═══════════════════════════════════════════════════════════
// ZEUS — Page Visibility — pause polling when tab is hidden
// Usage: use_tab_visible() returns a reactive signal
// ═══════════════════════════════════════════════════════════

use leptos::prelude::*;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;

/// Returns a reactive signal that is `true` when the tab is visible, `false` when hidden.
/// Use this to gate polling intervals so we don't waste CPU/network on background tabs.
pub fn use_tab_visible() -> ReadSignal<bool> {
    let (visible, set_visible) = signal(true);

    // Check initial state
    if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
        if let Ok(hidden) = js_sys::Reflect::get(&doc, &"hidden".into()) {
            if hidden.as_bool() == Some(true) {
                set_visible.set(false);
            }
        }
    }

    // Listen for visibility changes
    if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
        let cb = Closure::wrap(Box::new(move || {
            if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
                if let Ok(hidden) = js_sys::Reflect::get(&doc, &"hidden".into()) {
                    set_visible.set(hidden.as_bool() != Some(true));
                }
            }
        }) as Box<dyn FnMut()>);
        let _ = doc.add_event_listener_with_callback(
            "visibilitychange",
            cb.as_ref().unchecked_ref(),
        );
        cb.forget(); // Lives for app lifetime
    }

    visible
}
