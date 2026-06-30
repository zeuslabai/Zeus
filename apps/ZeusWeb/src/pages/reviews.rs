// ═══════════════════════════════════════════════════════════
// ZEUS — Peer Reviews / QA Page — S21 Phase 4 P0
// Submit agent outputs for review, approve/reject with scoring
// ═══════════════════════════════════════════════════════════

use crate::api;
use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

fn verdict_color(v: &str) -> &'static str {
    match v {
        "approved" => "rgba(34,197,94,0.8)",
        "rejected" => "rgba(239,68,68,0.8)",
        "pending" => "rgba(234,179,8,0.8)",
        _ => "rgba(255,245,240,0.4)",
    }
}

fn score_bar(score: f64) -> impl IntoView {
    let pct = (score.clamp(0.0, 1.0) * 100.0) as u32;
    let color = if score >= 0.8 { "rgba(34,197,94,0.7)" }
        else if score >= 0.5 { "rgba(234,179,8,0.7)" }
        else { "rgba(239,68,68,0.7)" };
    view! {
        <div style="display: flex; align-items: center; gap: 8px;">
            <div style="flex: 1; height: 4px; background: rgba(255,255,255,0.05); border-radius: 2px; overflow: hidden;">
                <div style=format!("height: 100%; width: {}%; background: {}; border-radius: 2px;", pct, color) />
            </div>
            <span style=format!("font-size: 11px; color: {}; font-weight: 700; min-width: 32px;", color)>{format!("{:.0}%", score * 100.0)}</span>
        </div>
    }
}

#[component]
pub fn ReviewsPage() -> impl IntoView {
    let reviews = RwSignal::new(Vec::<serde_json::Value>::new());
    let loading = RwSignal::new(true);
    let selected_id = RwSignal::new(Option::<String>::None);
    let selected = RwSignal::new(Option::<api::ReviewDetailResponse>::None);
    let toast = RwSignal::new(String::new());
    let toast_ok = RwSignal::new(true);
    // Submit form
    let show_submit = RwSignal::new(false);
    let sub_task_id = RwSignal::new(String::new());
    let sub_agent_id = RwSignal::new(String::new());
    let sub_output = RwSignal::new(String::new());
    let submitting = RwSignal::new(false);
    // Verdict form
    let show_verdict = RwSignal::new(false);
    let verd_reviewer = RwSignal::new(String::new());
    let verd_score = RwSignal::new("0.8".to_string());
    let verd_comments = RwSignal::new(String::new());
    let verd_loading = RwSignal::new(false);

    let set_toast = move |ok: bool, msg: String| {
        toast_ok.set(ok);
        toast.set(msg.clone());
        let toast = toast;
        spawn_local(async move {
            gloo_timers::future::TimeoutFuture::new(3000).await;
            if toast.get_untracked() == msg { toast.set(String::new()); }
        });
    };

    let reload = move || {
        loading.set(true);
        spawn_local(async move {
            match api::list_reviews(50).await {
                Ok(r) => reviews.set(r.reviews),
                Err(e) => set_toast(false, format!("Load error: {e}")),
            }
            loading.set(false);
        });
    };
    reload();

    let select_review = move |id: String| {
        selected_id.set(Some(id.clone()));
        selected.set(None);
        show_verdict.set(false);
        spawn_local(async move {
            if let Ok(r) = api::get_review(&id).await {
                selected.set(Some(r));
            }
        });
    };

    let do_submit = move |_: leptos::ev::MouseEvent| {
        let task_id = sub_task_id.get_untracked();
        let agent_id = sub_agent_id.get_untracked();
        let output = sub_output.get_untracked();
        if task_id.is_empty() || agent_id.is_empty() || output.is_empty() {
            set_toast(false, "All fields required".into()); return;
        }
        submitting.set(true);
        spawn_local(async move {
            match api::submit_review(&task_id, &agent_id, &output).await {
                Ok(r) => {
                    set_toast(true, format!("Submitted: {} · {} reviewers assigned", r.submission_id, r.reviewers_assigned.len()));
                    sub_task_id.set(String::new());
                    sub_agent_id.set(String::new());
                    sub_output.set(String::new());
                    show_submit.set(false);
                    reload();
                }
                Err(e) => set_toast(false, format!("Error: {e}")),
            }
            submitting.set(false);
        });
    };

    let do_verdict = move |approve: bool| {
        let id = match selected_id.get_untracked() { Some(id) => id, None => return };
        let reviewer = verd_reviewer.get_untracked();
        if reviewer.is_empty() { set_toast(false, "Reviewer ID required".into()); return; }
        let score: Option<f64> = verd_score.get_untracked().parse().ok();
        let comments = verd_comments.get_untracked();
        verd_loading.set(true);
        spawn_local(async move {
            let comments_ref: Option<&str> = if comments.is_empty() { None } else { Some(&comments) };
            let result = if approve {
                api::approve_review(&id, &reviewer, score, comments_ref).await
            } else {
                api::reject_review(&id, &reviewer, score, comments_ref).await
            };
            match result {
                Ok(r) => {
                    set_toast(true, format!("Verdict: {} · score {:.2}", r.verdict, r.score));
                    show_verdict.set(false);
                    select_review(id);
                    reload();
                }
                Err(e) => set_toast(false, format!("Error: {e}")),
            }
            verd_loading.set(false);
        });
    };

    view! {
        <div style="padding: 32px; max-width: 1200px; margin: 0 auto; font-family: 'Rajdhani', sans-serif; color: rgba(255,245,240,0.9);">
            // Header
            <div style="display: flex; align-items: center; justify-content: space-between; margin-bottom: 28px;">
                <div>
                    <h1 style="font-family: 'Orbitron', monospace; font-size: 20px; font-weight: 700; letter-spacing: 4px; color: rgba(255,245,240,0.9); margin: 0 0 4px 0;">"PEER REVIEWS"</h1>
                    <p style="font-size: 13px; color: rgba(255,245,240,0.5); margin: 0;">"Submit agent outputs for multi-agent quality review"</p>
                </div>
                <button
                    on:click=move |_| show_submit.update(|v| *v = !*v)
                    style="padding: 10px 20px; background: rgba(255,60,20,0.15); border: 1px solid rgba(255,60,20,0.4); border-radius: 8px; color: rgba(255,140,80,1); font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; cursor: pointer;"
                >"+ SUBMIT FOR REVIEW"</button>
            </div>

            // Toast
            {move || (!toast.get().is_empty()).then(|| view! {
                <div style=move || format!(
                    "margin-bottom: 16px; padding: 10px 16px; border-radius: 8px; font-size: 13px; background: {}; border: 1px solid {};",
                    if toast_ok.get() { "rgba(34,197,94,0.1)" } else { "rgba(239,68,68,0.1)" },
                    if toast_ok.get() { "rgba(34,197,94,0.3)" } else { "rgba(239,68,68,0.3)" }
                )>{toast.get()}</div>
            })}

            // Submit form
            {move || show_submit.get().then(|| view! {
                <div style="background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.15); border-radius: 12px; padding: 20px; margin-bottom: 20px;">
                    <div style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 2px; color: rgba(255,60,20,0.6); margin-bottom: 16px;">"SUBMIT OUTPUT FOR REVIEW"</div>
                    <div style="display: flex; gap: 12px; margin-bottom: 12px;">
                        <div style="flex: 1;">
                            <label style="font-size: 11px; color: rgba(255,245,240,0.5); display: block; margin-bottom: 4px; text-transform: uppercase; letter-spacing: 1px;">"Task ID"</label>
                            <input
                                placeholder="task-abc123"
                                prop:value=move || sub_task_id.get()
                                on:input=move |e| sub_task_id.set(event_target_value(&e))
                                style="width: 100%; padding: 9px 12px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.2); border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 13px; outline: none; box-sizing: border-box;"
                            />
                        </div>
                        <div style="flex: 1;">
                            <label style="font-size: 11px; color: rgba(255,245,240,0.5); display: block; margin-bottom: 4px; text-transform: uppercase; letter-spacing: 1px;">"Agent ID"</label>
                            <input
                                placeholder="agent-xyz"
                                prop:value=move || sub_agent_id.get()
                                on:input=move |e| sub_agent_id.set(event_target_value(&e))
                                style="width: 100%; padding: 9px 12px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.2); border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 13px; outline: none; box-sizing: border-box;"
                            />
                        </div>
                    </div>
                    <div style="margin-bottom: 12px;">
                        <label style="font-size: 11px; color: rgba(255,245,240,0.5); display: block; margin-bottom: 4px; text-transform: uppercase; letter-spacing: 1px;">"Agent Output"</label>
                        <textarea
                            rows="5"
                            placeholder="Paste the agent's output to be reviewed..."
                            prop:value=move || sub_output.get()
                            on:input=move |e| sub_output.set(event_target_value(&e))
                            style="width: 100%; padding: 10px 12px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.2); border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 13px; outline: none; box-sizing: border-box; resize: vertical; font-family: monospace; line-height: 1.5;"
                        />
                    </div>
                    <button
                        disabled=move || submitting.get()
                        on:click=do_submit
                        style="padding: 10px 24px; background: rgba(255,60,20,0.2); border: 1px solid rgba(255,60,20,0.5); border-radius: 8px; color: rgba(255,140,80,1); font-size: 13px; cursor: pointer;"
                    >{move || if submitting.get() { "Submitting..." } else { "Submit for Review" }}</button>
                </div>
            })}

            // Two-pane layout
            <div style="display: flex; gap: 20px; align-items: flex-start;">
                // Left: review list
                <div style="width: 300px; flex-shrink: 0;">
                    {move || {
                        if loading.get() {
                            view! { <div style="color: rgba(255,245,240,0.4); font-size: 13px; padding: 20px 0;">"Loading..."</div> }.into_any()
                        } else {
                            let rs = reviews.get();
                            if rs.is_empty() {
                                view! {
                                    <div style="text-align: center; padding: 40px 20px; color: rgba(255,245,240,0.3); border: 1px dashed rgba(255,60,20,0.1); border-radius: 12px;">
                                        <div style="font-size: 32px; margin-bottom: 12px;">"🔍"</div>
                                        <div style="font-size: 13px;">"No reviews yet"</div>
                                        <div style="font-size: 11px; margin-top: 4px; color: rgba(255,245,240,0.2);">"Submit an agent output to start"</div>
                                    </div>
                                }.into_any()
                            } else {
                                view! {
                                    <div style="display: flex; flex-direction: column; gap: 8px;">
                                    {rs.into_iter().map(|r| {
                                        let id = r.get("submission_id").or_else(|| r.get("id"))
                                            .and_then(|v| v.as_str()).unwrap_or("").to_string();
                                        let verdict = r.get("verdict").and_then(|v| v.as_str()).unwrap_or("pending").to_string();
                                        let score = r.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
                                        let agent = r.get("agent_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                        let id2 = id.clone();
                                        let is_sel = {
                                            let id = id.clone();
                                            Memo::new(move |_| selected_id.get().as_deref() == Some(&id))
                                        };
                                        view! {
                                            <div
                                                style=move || format!(
                                                    "padding: 12px 14px; border-radius: 10px; cursor: pointer; border: 1px solid {}; background: {};",
                                                    if is_sel.get() { "rgba(255,60,20,0.4)" } else { "rgba(255,60,20,0.1)" },
                                                    if is_sel.get() { "rgba(255,60,20,0.08)" } else { "rgba(255,255,255,0.02)" },
                                                )
                                                on:click=move |_| select_review(id2.clone())
                                            >
                                                <div style="font-family: monospace; font-size: 10px; color: rgba(255,245,240,0.4); margin-bottom: 4px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">{id}</div>
                                                {(!agent.is_empty()).then(|| view! {
                                                    <div style="font-size: 12px; color: rgba(255,245,240,0.6); margin-bottom: 6px;">{agent}</div>
                                                })}
                                                <div style="display: flex; gap: 8px; align-items: center;">
                                                    <span style=move || format!(
                                                        "font-size: 10px; padding: 2px 8px; border-radius: 20px; font-weight: 600; background: rgba(255,255,255,0.05); color: {};",
                                                        verdict_color(&verdict)
                                                    )>{verdict.to_uppercase()}</span>
                                                    {(score > 0.0).then(|| view! {
                                                        <span style="font-size: 11px; color: rgba(255,245,240,0.5);">{format!("{:.0}%", score * 100.0)}</span>
                                                    })}
                                                </div>
                                            </div>
                                        }
                                    }).collect_view()}
                                    </div>
                                }.into_any()
                            }
                        }
                    }}
                </div>

                // Right: detail
                <div style="flex: 1; min-width: 0;">
                    {move || match selected.get() {
                        None => Some(view! {
                            <div style="display: flex; align-items: center; justify-content: center; height: 300px; color: rgba(255,245,240,0.3); font-size: 14px; border: 1px dashed rgba(255,60,20,0.1); border-radius: 12px;">
                                "← Select a review to inspect"
                            </div>
                        }.into_any()),
                        Some(detail) => {
                            let sid = detail.submission_id.clone();
                            Some(view! {
                                <div>
                                    // Header
                                    <div style="background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.15); border-radius: 12px; padding: 20px; margin-bottom: 16px;">
                                        <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 12px;">
                                            <div style="font-family: monospace; font-size: 11px; color: rgba(255,245,240,0.5);">{sid.clone()}</div>
                                            <div style="display: flex; gap: 8px;">
                                                <button
                                                    on:click=move |_| show_verdict.update(|v| *v = !*v)
                                                    style="padding: 7px 16px; background: rgba(255,60,20,0.15); border: 1px solid rgba(255,60,20,0.3); border-radius: 6px; color: rgba(255,140,80,0.9); font-size: 12px; cursor: pointer;"
                                                >"Cast Verdict"</button>
                                            </div>
                                        </div>
                                        <div style="font-size: 13px; color: rgba(255,245,240,0.5);">{format!("{} review(s)", detail.review_count)}</div>
                                    </div>

                                    // Verdict form
                                    {move || show_verdict.get().then(|| {
                                        view! {
                                            <div style="background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.15); border-radius: 12px; padding: 20px; margin-bottom: 16px;">
                                                <div style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 2px; color: rgba(255,60,20,0.6); margin-bottom: 12px;">"CAST VERDICT"</div>
                                                <div style="display: flex; gap: 12px; margin-bottom: 12px; flex-wrap: wrap;">
                                                    <input
                                                        placeholder="Reviewer ID"
                                                        prop:value=move || verd_reviewer.get()
                                                        on:input=move |e| verd_reviewer.set(event_target_value(&e))
                                                        style="flex: 1; min-width: 150px; padding: 9px 12px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.2); border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 13px; outline: none;"
                                                    />
                                                    <input
                                                        placeholder="Score 0.0–1.0"
                                                        prop:value=move || verd_score.get()
                                                        on:input=move |e| verd_score.set(event_target_value(&e))
                                                        style="width: 120px; padding: 9px 12px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.2); border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 13px; outline: none;"
                                                    />
                                                </div>
                                                <textarea
                                                    rows="3"
                                                    placeholder="Comments (optional)"
                                                    prop:value=move || verd_comments.get()
                                                    on:input=move |e| verd_comments.set(event_target_value(&e))
                                                    style="width: 100%; padding: 9px 12px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.2); border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 13px; outline: none; box-sizing: border-box; resize: vertical; margin-bottom: 12px;"
                                                />
                                                <div style="display: flex; gap: 10px;">
                                                    <button
                                                        disabled=move || verd_loading.get()
                                                        on:click=move |_| do_verdict(true)
                                                        style="padding: 9px 20px; background: rgba(34,197,94,0.1); border: 1px solid rgba(34,197,94,0.4); border-radius: 8px; color: rgba(34,197,94,0.9); font-size: 13px; cursor: pointer;"
                                                    >"✓ Approve"</button>
                                                    <button
                                                        disabled=move || verd_loading.get()
                                                        on:click=move |_| do_verdict(false)
                                                        style="padding: 9px 20px; background: rgba(239,68,68,0.1); border: 1px solid rgba(239,68,68,0.4); border-radius: 8px; color: rgba(239,68,68,0.9); font-size: 13px; cursor: pointer;"
                                                    >"✕ Reject"</button>
                                                </div>
                                            </div>
                                        }
                                    })}

                                    // Existing reviews
                                    {(!detail.reviews.is_empty()).then(|| view! {
                                        <div style="background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.15); border-radius: 12px; padding: 20px; margin-bottom: 16px;">
                                            <div style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 2px; color: rgba(255,60,20,0.6); margin-bottom: 12px;">"REVIEW VERDICTS"</div>
                                            <div style="display: flex; flex-direction: column; gap: 12px;">
                                            {detail.reviews.iter().map(|r| {
                                                let verdict = r.verdict.clone();
                                                let score = r.score;
                                                let reviewer = r.reviewer_id.clone();
                                                let comments = r.comments.clone();
                                                let dims = r.dimensions.clone();
                                                view! {
                                                    <div style="padding: 14px; background: rgba(255,255,255,0.02); border: 1px solid rgba(255,60,20,0.08); border-radius: 8px;">
                                                        <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 8px;">
                                                            <span style="font-size: 13px; font-weight: 600; color: rgba(255,245,240,0.8);">{reviewer}</span>
                                                            <span style=format!("font-size: 10px; padding: 2px 10px; border-radius: 20px; font-weight: 700; background: rgba(255,255,255,0.05); color: {};", verdict_color(&verdict))>{verdict.to_uppercase()}</span>
                                                        </div>
                                                        {score_bar(score)}
                                                        {(!dims.is_empty()).then(|| view! {
                                                            <div style="display: flex; gap: 10px; margin-top: 8px; flex-wrap: wrap;">
                                                            {dims.iter().map(|(k, v)| {
                                                                let k = k.clone();
                                                                let v = *v;
                                                                view! {
                                                                    <div style="font-size: 10px; color: rgba(255,245,240,0.5);">
                                                                        {format!("{}: {:.0}%", k, v * 100.0)}
                                                                    </div>
                                                                }
                                                            }).collect_view()}
                                                            </div>
                                                        })}
                                                        {(!comments.is_empty()).then(|| view! {
                                                            <div style="margin-top: 8px; font-size: 13px; color: rgba(255,245,240,0.6); line-height: 1.5;">{comments}</div>
                                                        })}
                                                    </div>
                                                }
                                            }).collect_view()}
                                            </div>
                                        </div>
                                    })}

                                    // Raw entries
                                    {(!detail.entries.is_empty()).then(|| view! {
                                        <div style="background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.15); border-radius: 12px; padding: 20px;">
                                            <div style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 2px; color: rgba(255,60,20,0.6); margin-bottom: 12px;">"SUBMITTED ENTRIES"</div>
                                            <div style="display: flex; flex-direction: column; gap: 8px;">
                                            {detail.entries.iter().map(|e| {
                                                let txt = serde_json::to_string_pretty(e).unwrap_or_default();
                                                view! {
                                                    <pre style="font-family: monospace; font-size: 11px; color: rgba(255,245,240,0.6); background: rgba(0,0,0,0.2); padding: 12px; border-radius: 6px; overflow-x: auto; margin: 0; white-space: pre-wrap; word-break: break-all;">{txt}</pre>
                                                }
                                            }).collect_view()}
                                            </div>
                                        </div>
                                    })}
                                </div>
                            }.into_any())
                        }
                    }}
                </div>
            </div>
        </div>
    }
}
