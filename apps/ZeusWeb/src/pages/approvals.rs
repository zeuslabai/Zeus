// ═══════════════════════════════════════════════════════════
// ZEUS — Approvals Page — Wired to API
// ═══════════════════════════════════════════════════════════

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;
use crate::api;
use crate::components::design::*;
use crate::components::diff_viewer::{DiffViewer, detect_diff_args};

#[component]
pub fn ApprovalsPage() -> impl IntoView {
    let approvals = RwSignal::new(Vec::<api::PendingApproval>::new());
    let loading = RwSignal::new(true);

    let refresh = move || {
        let approvals = approvals;
        let loading = loading;
        spawn_local(async move {
            loading.set(true);
            if let Ok(a) = api::fetch_approvals().await {
                approvals.set(a);
            }
            loading.set(false);
        });
    };

    // Initial load
    refresh();

    view! {
        <div style="padding: 32px;">
            <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 24px;">
                <div>
                    <h1 style="font-family: 'Orbitron', monospace; font-size: 14px; letter-spacing: 6px; color: rgba(255,245,240,0.9); margin: 0;">"APPROVALS"</h1>
                    <p style="font-size: 12px; color: rgba(255,245,240,0.7); margin: 4px 0 0;">
                        {move || {
                            let a = approvals.get();
                            if loading.get() { "Loading...".to_string() }
                            else if a.is_empty() { "All clear — no actions pending your approval.".to_string() }
                            else { format!("{} pending", a.len()) }
                        }}
                    </p>
                </div>
                <Button on_click=Some(Callback::new(move |_| refresh()))>"Refresh"</Button>
            </div>
            <div style="display: flex; flex-direction: column; gap: 10px;">
                {move || {
                    let items = approvals.get();
                    if items.is_empty() && !loading.get() {
                        vec![view! {
                            <Card>
                                <div style="text-align: center; padding: 40px; color: rgba(255,245,240,0.5);">
                                    <Icon name="security" size=32 />
                                    <div style="font-family: 'Orbitron', monospace; font-size: 11px; letter-spacing: 3px; margin-top: 12px;">
                                        "ALL CLEAR"
                                    </div>
                                    <div style="font-size: 12px; margin-top: 4px;">"No operations are waiting — agents are running clean."</div>
                                </div>
                            </Card>
                        }.into_any()]
                    } else {
                        items.into_iter().map(|a| {
                            let id = a.id.clone();
                            let id_approve = id.clone();
                            let id_deny = id.clone();
                            let approvals_sig = approvals;
                            let args_str = serde_json::to_string_pretty(&a.args).unwrap_or_default();
                            let diff_patch = detect_diff_args(&a.args);
                            let agent_label = a.agent_id.clone().unwrap_or_else(|| "system".to_string());
                            let risk_label = a.risk.label();
                            let risk_color = a.risk.color();

                            view! {
                                <Card>
                                    <div style="display: flex; align-items: flex-start; gap: 14px;">
                                        <div style="width: 40px; height: 40px; border-radius: 10px; background: rgba(234,179,8,0.15); display: flex; align-items: center; justify-content: center; flex-shrink: 0;">
                                            <Icon name="security" size=18 color="rgba(234,179,8,0.8)".to_string() />
                                        </div>
                                        <div style="flex: 1;">
                                            <div style="font-family: 'Orbitron', monospace; font-size: 12px; color: rgba(255,245,240,0.9); letter-spacing: 2px; font-weight: 600; margin-bottom: 4px;">
                                                {a.tool_name.clone()}
                                            </div>
                                            <div style="display: flex; align-items: center; gap: 8px; margin-bottom: 4px;">
                                                <span style=format!("font-size: 9px; font-family: 'Orbitron', monospace; letter-spacing: 2px; padding: 2px 8px; border-radius: 4px; font-weight: 700; color: {}; background: {}; border: 1px solid {};", risk_color, "rgba(0,0,0,0.3)", risk_color)>
                                                    {risk_label}
                                                </span>
                                            </div>
                                            <div style="font-size: 11px; color: rgba(255,245,240,0.7); margin-bottom: 8px;">
                                                "Agent: "{agent_label}" • "{a.created_at.clone()}
                                            </div>
                                            {move || {
                                                if let Some(ref patch) = diff_patch {
                                                    let sig = leptos::prelude::signal(patch.clone()).0;
                                                    view! { <DiffViewer patch=sig /> }.into_any()
                                                } else {
                                                    view! {
                                                        <pre style="font-size: 11px; color: rgba(255,245,240,0.5); background: rgba(0,0,0,0.3); padding: 8px; border-radius: 6px; overflow-x: auto; margin: 0; max-height: 120px; overflow-y: auto;">
                                                            {args_str.clone()}
                                                        </pre>
                                                    }.into_any()
                                                }
                                            }}
                                        </div>
                                        <div style="display: flex; flex-direction: column; gap: 6px; flex-shrink: 0;">
                                            <button
                                                on:click=move |_| {
                                                    let id = id_approve.clone();
                                                    let approvals_sig = approvals_sig;
                                                    spawn_local(async move {
                                                        if let Err(e) = api::approve_execution(&id).await { web_sys::console::error_1(&format!("API error: {}", e).into()); }
                                                        if let Ok(a) = api::fetch_approvals().await {
                                                            approvals_sig.set(a);
                                                        }
                                                    });
                                                }
                                                style="padding: 6px 14px; background: rgba(34,197,94,0.15); border: 1px solid rgba(34,197,94,0.3); border-radius: 6px; color: rgba(34,197,94,1); font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; cursor: pointer;"
                                            >
                                                "APPROVE"
                                            </button>
                                            <button
                                                on:click=move |_| {
                                                    let id = id_deny.clone();
                                                    let approvals_sig = approvals_sig;
                                                    spawn_local(async move {
                                                        if let Err(e) = api::deny_execution(&id, None).await { web_sys::console::error_1(&format!("API error: {}", e).into()); }
                                                        if let Ok(a) = api::fetch_approvals().await {
                                                            approvals_sig.set(a);
                                                        }
                                                    });
                                                }
                                                style="padding: 6px 14px; background: rgba(239,68,68,0.15); border: 1px solid rgba(239,68,68,0.3); border-radius: 6px; color: rgba(239,68,68,1); font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; cursor: pointer;"
                                            >
                                                "DENY"
                                            </button>
                                        </div>
                                    </div>
                                </Card>
                            }.into_any()
                        }).collect::<Vec<_>>()
                    }
                }}
            </div>
        </div>
    }
}
