// ═══════════════════════════════════════════════════════════
// ZEUS — Batch Jobs Page — S21 Phase 4 P0
// Create batch requests, track status, view results
// ═══════════════════════════════════════════════════════════

use crate::api;
use crate::components::design::*;
use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

fn batch_status_color(s: &str) -> &'static str {
    match s {
        "completed" => "rgba(34,197,94,0.8)",
        "processing" | "in_progress" => "rgba(234,179,8,0.8)",
        "failed" | "error" => "rgba(239,68,68,0.8)",
        "queued" => "rgba(147,197,253,0.8)",
        _ => "rgba(255,245,240,0.4)",
    }
}

fn fmt_ts_u64(ts: u64) -> String {
    // Unix seconds → simple display
    if ts == 0 { return "—".to_string(); }
    format!("t+{ts}s")
}

#[component]
pub fn BatchPage() -> impl IntoView {
    let batches = RwSignal::new(Vec::<api::BatchResponse>::new());
    let selected_id = RwSignal::new(Option::<String>::None);
    let selected_batch = RwSignal::new(Option::<api::BatchResponse>::None);
    let results = RwSignal::new(Vec::<api::BatchResult>::new());
    let loading_results = RwSignal::new(false);
    let toast = RwSignal::new(String::new());
    let toast_ok = RwSignal::new(true);
    // Create form
    let show_create = RwSignal::new(false);
    let new_model = RwSignal::new("anthropic/claude-sonnet-4-6".to_string());
    let new_requests = RwSignal::new(String::new());
    let creating = RwSignal::new(false);
    // Lookup form
    let lookup_id = RwSignal::new(String::new());
    let looking_up = RwSignal::new(false);

    let set_toast = move |ok: bool, msg: String| {
        toast_ok.set(ok);
        toast.set(msg.clone());
        let toast = toast;
        spawn_local(async move {
            gloo_timers::future::TimeoutFuture::new(3000).await;
            if toast.get_untracked() == msg { toast.set(String::new()); }
        });
    };

    let load_batch_detail = move |id: String| {
        selected_id.set(Some(id.clone()));
        loading_results.set(true);
        results.set(vec![]);
        spawn_local(async move {
            if let Ok(b) = api::fetch_batch(&id).await {
                selected_batch.set(Some(b));
            }
            if let Ok(r) = api::fetch_batch_results(&id).await {
                results.set(r.results);
            }
            loading_results.set(false);
        });
    };

    let do_lookup = Callback::new(move |_: leptos::ev::MouseEvent| {
        let id = lookup_id.get_untracked().trim().to_string();
        if id.is_empty() { set_toast(false, "Enter a batch ID".into()); return; }
        looking_up.set(true);
        let id2 = id.clone();
        spawn_local(async move {
            match api::fetch_batch(&id2).await {
                Ok(b) => {
                    // Add to list if not already there
                    batches.update(|v| {
                        if !v.iter().any(|x| x.id == b.id) { v.insert(0, b.clone()); }
                    });
                    load_batch_detail(b.id);
                    lookup_id.set(String::new());
                }
                Err(e) => set_toast(false, format!("Not found: {e}")),
            }
            looking_up.set(false);
        });
    });

    let do_create = move |_: leptos::ev::MouseEvent| {
        let model = new_model.get_untracked();
        let raw = new_requests.get_untracked();
        if raw.trim().is_empty() {
            set_toast(false, "Add at least one prompt (one per line)".into());
            return;
        }
        let prompts: Vec<serde_json::Value> = raw
            .lines()
            .filter(|l| !l.trim().is_empty())
            .enumerate()
            .map(|(i, l)| serde_json::json!({
                "custom_id": format!("req-{}", i),
                "messages": [{"role": "user", "content": l.trim()}],
            }))
            .collect();
        creating.set(true);
        spawn_local(async move {
            let body = serde_json::json!({ "model": model, "requests": prompts });
            match api::create_batch(&body).await {
                Ok(b) => {
                    set_toast(true, format!("Batch queued: {}", b.id));
                    batches.update(|v| v.insert(0, b.clone()));
                    load_batch_detail(b.id);
                    new_requests.set(String::new());
                    show_create.set(false);
                }
                Err(e) => set_toast(false, format!("Error: {e}")),
            }
            creating.set(false);
        });
    };

    view! {
        <div style="padding: 32px; max-width: 1200px; margin: 0 auto; font-family: 'Rajdhani', sans-serif; color: rgba(255,245,240,0.9);">
            // Header
            <div style="display: flex; align-items: center; justify-content: space-between; margin-bottom: 28px;">
                <div>
                    <h1 style="font-family: 'Orbitron', monospace; font-size: 20px; font-weight: 700; letter-spacing: 4px; color: rgba(255,245,240,0.9); margin: 0 0 4px 0;">"BATCH JOBS"</h1>
                    <p style="font-size: 13px; color: rgba(255,245,240,0.5); margin: 0;">"Queue multiple LLM requests for async processing"</p>
                </div>
                <Button primary=true on_click=Some(Callback::new(move |_| show_create.update(|v| *v = !*v)))>
                    <Icon name="plus" size=12 /> " NEW BATCH"
                </Button>
            </div>

            // Toast
            {move || (!toast.get().is_empty()).then(|| view! {
                <div style=move || format!(
                    "margin-bottom: 16px; padding: 10px 16px; border-radius: 8px; font-size: 13px; background: {}; border: 1px solid {};",
                    if toast_ok.get() { "rgba(34,197,94,0.1)" } else { "rgba(239,68,68,0.1)" },
                    if toast_ok.get() { "rgba(34,197,94,0.3)" } else { "rgba(239,68,68,0.3)" }
                )>{toast.get()}</div>
            })}

            // Create form
            {move || show_create.get().then(|| view! {
                <div style="background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.15); border-radius: 12px; padding: 20px; margin-bottom: 20px;">
                    <div style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 2px; color: rgba(255,60,20,0.6); margin-bottom: 16px;">"NEW BATCH REQUEST"</div>
                    <div style="margin-bottom: 12px;">
                        <label style="font-size: 11px; color: rgba(255,245,240,0.5); display: block; margin-bottom: 6px; letter-spacing: 1px; text-transform: uppercase;">"Model"</label>
                        <input
                            prop:value=move || new_model.get()
                            on:input=move |e| new_model.set(event_target_value(&e))
                            style="width: 100%; padding: 10px 14px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.2); border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 14px; outline: none; box-sizing: border-box;"
                        />
                    </div>
                    <div style="margin-bottom: 12px;">
                        <label style="font-size: 11px; color: rgba(255,245,240,0.5); display: block; margin-bottom: 6px; letter-spacing: 1px; text-transform: uppercase;">"Prompts (one per line)"</label>
                        <textarea
                            rows="6"
                            placeholder="Summarize the Q3 report\nTranslate this to French: Hello\nWhat is the capital of Japan?"
                            prop:value=move || new_requests.get()
                            on:input=move |e| new_requests.set(event_target_value(&e))
                            style="width: 100%; padding: 10px 14px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.2); border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 13px; outline: none; box-sizing: border-box; resize: vertical; font-family: monospace; line-height: 1.5;"
                        />
                    </div>
                    <Button primary=true on_click=Some(Callback::new(move |_| do_create(leptos::ev::MouseEvent::new("click").unwrap())))>
                        {move || if creating.get() { "Queuing..." } else { "Queue Batch" }}
                    </Button>
                </div>
            })}

            // Lookup by ID
            <div style="background: rgba(255,255,255,0.02); border: 1px solid rgba(255,60,20,0.1); border-radius: 10px; padding: 16px; margin-bottom: 20px; display: flex; gap: 10px; align-items: center;">
                <input
                    placeholder="Look up batch by ID..."
                    prop:value=move || lookup_id.get()
                    on:input=move |e| lookup_id.set(event_target_value(&e))
                    on:keydown=move |e: leptos::ev::KeyboardEvent| { if e.key() == "Enter" { do_lookup.run(leptos::ev::MouseEvent::new("click").unwrap()); } }
                    style="flex: 1; padding: 9px 14px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.15); border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 13px; outline: none; font-family: monospace;"
                />
                <Button on_click=Some(Callback::new(move |_| do_lookup.run(leptos::ev::MouseEvent::new("click").unwrap())))>
                    {move || if looking_up.get() { "Looking up..." } else { "Look Up" }}
                </Button>
            </div>

            // Two-pane layout
            <div style="display: flex; gap: 20px; align-items: flex-start;">
                // Left: batch list
                <div style="width: 320px; flex-shrink: 0;">
                    {move || {
                        let bs = batches.get();
                        if bs.is_empty() {
                            view! {
                                <div style="text-align: center; padding: 40px 20px; color: rgba(255,245,240,0.3); border: 1px dashed rgba(255,60,20,0.1); border-radius: 12px;">
                                    <div style="font-size: 32px; margin-bottom: 12px;">"📦"</div>
                                    <div style="font-size: 13px;">"No batches yet — submit prompts above to get started."</div>
                                    <div style="font-size: 11px; margin-top: 4px; color: rgba(255,245,240,0.2);">"Create one or look up an existing batch ID"</div>
                                </div>
                            }.into_any()
                        } else {
                            view! {
                                <div style="display: flex; flex-direction: column; gap: 8px;">
                                {bs.into_iter().map(|b| {
                                    let id = b.id.clone();
                                    let id2 = id.clone();
                                    let is_sel = Memo::new(move |_| {
                                        selected_id.get().as_deref() == Some(&id)
                                    });
                                    view! {
                                        <div
                                            style=move || format!(
                                                "padding: 14px; border-radius: 10px; cursor: pointer; border: 1px solid {}; background: {};",
                                                if is_sel.get() { "rgba(255,60,20,0.4)" } else { "rgba(255,60,20,0.1)" },
                                                if is_sel.get() { "rgba(255,60,20,0.08)" } else { "rgba(255,255,255,0.02)" },
                                            )
                                            on:click=move |_| load_batch_detail(id2.clone())
                                        >
                                            <div style="font-family: monospace; font-size: 11px; color: rgba(255,245,240,0.5); margin-bottom: 6px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">{b.id.clone()}</div>
                                            <div style="display: flex; gap: 8px; align-items: center; flex-wrap: wrap;">
                                                <span style=move || format!(
                                                    "font-size: 10px; padding: 2px 8px; border-radius: 20px; font-weight: 600; background: rgba(255,255,255,0.05); color: {};",
                                                    batch_status_color(&b.status)
                                                )>{b.status.clone()}</span>
                                                <span style="font-size: 10px; color: rgba(255,245,240,0.5);">{format!("{}/{} done", b.completed, b.total)}</span>
                                                {(b.failed > 0).then(|| view! {
                                                    <span style="font-size: 10px; color: rgba(239,68,68,0.7);">{format!("{} failed", b.failed)}</span>
                                                })}
                                            </div>
                                            {(!b.model.is_empty()).then(|| view! {
                                                <div style="font-size: 11px; color: rgba(255,245,240,0.3); margin-top: 4px;">{b.model.clone()}</div>
                                            })}
                                        </div>
                                    }
                                }).collect_view()}
                                </div>
                            }.into_any()
                        }
                    }}
                </div>

                // Right: detail
                <div style="flex: 1; min-width: 0;">
                    {move || match selected_batch.get() {
                        None => Some(view! {
                            <div style="display: flex; align-items: center; justify-content: center; height: 300px; color: rgba(255,245,240,0.3); font-size: 14px; border: 1px dashed rgba(255,60,20,0.1); border-radius: 12px;">
                                "← Select a batch to view results"
                            </div>
                        }.into_any()),
                        Some(b) => Some(view! {
                            <div>
                                // Batch header
                                <div style="background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.15); border-radius: 12px; padding: 20px; margin-bottom: 16px;">
                                    <div style="display: flex; justify-content: space-between; align-items: flex-start; margin-bottom: 12px;">
                                        <div style="font-family: monospace; font-size: 12px; color: rgba(255,245,240,0.6);">{b.id.clone()}</div>
                                        <span style=move || format!(
                                            "font-size: 11px; padding: 3px 10px; border-radius: 20px; font-weight: 700; background: rgba(255,255,255,0.05); color: {};",
                                            batch_status_color(&b.status)
                                        )>{b.status.clone().to_uppercase()}</span>
                                    </div>
                                    {(!b.model.is_empty()).then(|| view! {
                                        <div style="font-size: 12px; color: rgba(255,245,240,0.5); margin-bottom: 12px;">{b.model.clone()}</div>
                                    })}
                                    <div style="display: flex; gap: 24px; flex-wrap: wrap;">
                                        <div>
                                            <div style="font-size: 10px; color: rgba(255,245,240,0.4); text-transform: uppercase; letter-spacing: 1px;">"Total"</div>
                                            <div style="font-size: 24px; font-weight: 700; color: rgba(255,245,240,0.8);">{b.total}</div>
                                        </div>
                                        <div>
                                            <div style="font-size: 10px; color: rgba(255,245,240,0.4); text-transform: uppercase; letter-spacing: 1px;">"Completed"</div>
                                            <div style="font-size: 24px; font-weight: 700; color: rgba(34,197,94,0.8);">{b.completed}</div>
                                        </div>
                                        <div>
                                            <div style="font-size: 10px; color: rgba(255,245,240,0.4); text-transform: uppercase; letter-spacing: 1px;">"Failed"</div>
                                            <div style="font-size: 24px; font-weight: 700; color: rgba(239,68,68,0.8);">{b.failed}</div>
                                        </div>
                                        <div>
                                            <div style="font-size: 10px; color: rgba(255,245,240,0.4); text-transform: uppercase; letter-spacing: 1px;">"Created"</div>
                                            <div style="font-size: 14px; font-weight: 600; color: rgba(255,245,240,0.6);">{fmt_ts_u64(b.created_at)}</div>
                                        </div>
                                        {b.completed_at.map(|t| view! {
                                            <div>
                                                <div style="font-size: 10px; color: rgba(255,245,240,0.4); text-transform: uppercase; letter-spacing: 1px;">"Finished"</div>
                                                <div style="font-size: 14px; font-weight: 600; color: rgba(255,245,240,0.6);">{fmt_ts_u64(t)}</div>
                                            </div>
                                        })}
                                    </div>
                                    // Progress bar
                                    {(b.total > 0).then(|| {
                                        let pct = (b.completed as f64 / b.total as f64 * 100.0) as u32;
                                        view! {
                                            <div style="margin-top: 16px;">
                                                <div style="height: 4px; background: rgba(255,255,255,0.05); border-radius: 2px; overflow: hidden;">
                                                    <div style=format!(
                                                        "height: 100%; width: {}%; background: rgba(34,197,94,0.6); border-radius: 2px; transition: width 0.3s;",
                                                        pct
                                                    ) />
                                                </div>
                                                <div style="font-size: 10px; color: rgba(255,245,240,0.4); margin-top: 4px;">{format!("{}% complete", pct)}</div>
                                            </div>
                                        }
                                    })}
                                </div>

                                // Results
                                <div style="background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.15); border-radius: 12px; padding: 20px;">
                                    <div style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 2px; color: rgba(255,60,20,0.6); margin-bottom: 12px;">"RESULTS"</div>
                                    {move || {
                                        if loading_results.get() {
                                            Some(view! { <div style="color: rgba(255,245,240,0.4); font-size: 13px;">"Loading results..."</div> }.into_any())
                                        } else {
                                            let rs = results.get();
                                            if rs.is_empty() {
                                                Some(view! { <div style="color: rgba(255,245,240,0.3); font-size: 13px;">"No results yet — batch may still be processing."</div> }.into_any())
                                            } else {
                                                Some(view! {
                                                    <div style="display: flex; flex-direction: column; gap: 10px;">
                                                    {rs.into_iter().map(|r| {
                                                        let status_c = batch_status_color(&r.status);
                                                        view! {
                                                            <div style="padding: 14px; background: rgba(255,255,255,0.02); border: 1px solid rgba(255,60,20,0.08); border-radius: 8px;">
                                                                <div style="display: flex; justify-content: space-between; margin-bottom: 8px;">
                                                                    <span style="font-family: monospace; font-size: 11px; color: rgba(255,245,240,0.5);">{r.custom_id.clone()}</span>
                                                                    <span style=format!("font-size: 10px; padding: 2px 8px; border-radius: 20px; background: rgba(255,255,255,0.05); color: {};", status_c)>{r.status.clone()}</span>
                                                                </div>
                                                                {r.error.as_ref().map(|e| view! {
                                                                    <div style="font-size: 12px; color: rgba(239,68,68,0.8); padding: 6px 10px; background: rgba(239,68,68,0.05); border-radius: 4px;">{e.clone()}</div>
                                                                })}
                                                                {r.response.as_ref().map(|resp| view! {
                                                                    <div>
                                                                        <div style="font-size: 13px; color: rgba(255,245,240,0.85); line-height: 1.6; white-space: pre-wrap;">{resp.content.clone()}</div>
                                                                        <div style="display: flex; gap: 16px; margin-top: 8px; flex-wrap: wrap;">
                                                                            {(!resp.model.is_empty()).then(|| view! {
                                                                                <span style="font-size: 10px; color: rgba(255,245,240,0.3);">{resp.model.clone()}</span>
                                                                            })}
                                                                            {(resp.input_tokens > 0).then(|| view! {
                                                                                <span style="font-size: 10px; color: rgba(255,245,240,0.3);">{format!("↑{} ↓{} tokens", resp.input_tokens, resp.output_tokens)}</span>
                                                                            })}
                                                                        </div>
                                                                    </div>
                                                                })}
                                                            </div>
                                                        }
                                                    }).collect_view()}
                                                    </div>
                                                }.into_any())
                                            }
                                        }
                                    }}
                                </div>
                            </div>
                        }.into_any()),
                    }}
                </div>
            </div>
        </div>
    }
}
