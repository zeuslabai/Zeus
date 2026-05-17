// ═══════════════════════════════════════════════════════════
// ZEUS — Security Page — Phase 3: Audit log + rotation status
// ═══════════════════════════════════════════════════════════

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;
use crate::api;
use crate::components::design::*;

#[component]
pub fn SecurityPage() -> impl IntoView {
    let threats = RwSignal::new(Vec::<api::Threat>::new());
    let permissions = RwSignal::new(api::PermissionsResponse::default());
    let audit = RwSignal::new(Vec::<api::SecurityAuditEntry>::new());
    let audit_total = RwSignal::new(0u32);

    {
        let threats = threats;
        spawn_local(async move { if let Ok(t) = api::fetch_threats().await { threats.set(t.threats); } });
    }
    {
        let permissions = permissions;
        spawn_local(async move { if let Ok(p) = api::fetch_permissions().await { permissions.set(p); } });
    }
    let rotation = RwSignal::new(Option::<api::RotationStatusResponse>::None);
    let rotating = RwSignal::new(false);
    let confirm_rotate = RwSignal::new(false);

    {
        spawn_local(async move {
            if let Ok(r) = api::fetch_rotation_status().await { rotation.set(Some(r)); }
        });
    }

    {
        let audit = audit;
        let audit_total = audit_total;
        spawn_local(async move {
            if let Ok(a) = api::fetch_audit_log().await {
                audit_total.set(a.total);
                audit.set(a.entries);
            }
        });
    }

    view! {
        <div style="padding: 32px;">
            <h1 style="font-family: 'Orbitron', monospace; font-size: 14px; letter-spacing: 6px; color: rgba(255,245,240,0.9); margin: 0 0 24px;">"SECURITY"</h1>
            // Metric Cards
            {move || {
                let t = threats.get();
                let p = permissions.get();
                let blocked = t.iter().filter(|th| th.threat_type == "blocked" || th.detail.to_lowercase().contains("blocked")).count();
                let level = if p.global.level.is_empty() { "STANDARD".to_string() } else { p.global.level.to_uppercase() };
                view! {
                    <div style="display: flex; gap: 12px; margin-bottom: 24px; flex-wrap: wrap;">
                        <MetricCard label="Security Level" value=level icon="security" />
                        <MetricCard label="Threats Blocked" value={blocked.to_string()} icon="sandbox" />
                        <MetricCard label="Audit Events" value={audit_total.get().to_string()} icon="activity" />
                        <MetricCard label="Threat Log" value={t.len().to_string()} icon="approvals" />
                    </div>
                }
            }}

            // Permission Controls
            <Card style="margin-bottom: 16px;">
                <SectionTitle>"Global Permissions"</SectionTitle>
                <div style="display: flex; gap: 16px; flex-wrap: wrap; margin-top: 12px;">
                    {move || {
                        let p = permissions.get();
                        let shell = p.global.shell_access;
                        let file_w = p.global.file_write;
                        let web = p.global.web_access;
                        let level = p.global.level.clone();
                        vec![
                            ("Shell Access", shell, "shell"),
                            ("File Write", file_w, "file"),
                            ("Web Access", web, "web"),
                        ].into_iter().map(|(label, enabled, key)| {
                            let key = key.to_string();
                            let level = level.clone();
                            view! {
                                <div style="display: flex; align-items: center; gap: 10px; padding: 8px 14px; background: rgba(255,255,255,0.02); border-radius: 8px; border: 1px solid rgba(255,60,20,0.06);">
                                    <button
                                        on:click={
                                            let key = key.clone();
                                            let level = level.clone();
                                            move |_| {
                                                let p = permissions.get_untracked();
                                                let mut new_p = api::GlobalPerms {
                                                    shell_access: p.global.shell_access,
                                                    file_write: p.global.file_write,
                                                    web_access: p.global.web_access,
                                                    level: level.clone(),
                                                };
                                                match key.as_str() {
                                                    "shell" => new_p.shell_access = !new_p.shell_access,
                                                    "file" => new_p.file_write = !new_p.file_write,
                                                    "web" => new_p.web_access = !new_p.web_access,
                                                    _ => {}
                                                }
                                                spawn_local(async move {
                                                    let _ = api::update_permissions(&new_p).await;
                                                    if let Ok(pr) = api::fetch_permissions().await { permissions.set(pr); }
                                                });
                                            }
                                        }
                                        style={format!(
                                            "width: 36px; height: 20px; border-radius: 10px; border: none; cursor: pointer; position: relative; background: {};",
                                            if enabled { "rgba(34,197,94,0.6)" } else { "rgba(255,255,255,0.1)" }
                                        )}
                                    >
                                        <div style={format!(
                                            "width: 14px; height: 14px; border-radius: 50%; background: white; position: absolute; top: 3px; left: {};",
                                            if enabled { "19px" } else { "3px" }
                                        )} />
                                    </button>
                                    <span style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,245,240,0.9);">{label}</span>
                                </div>
                            }
                        }).collect::<Vec<_>>()
                    }}
                </div>
            </Card>

            // Key Rotation
            <Card style="margin-bottom: 16px;">
                <div style="display: flex; justify-content: space-between; align-items: center;">
                    <SectionTitle>"API Key Rotation"</SectionTitle>
                    {move || if confirm_rotate.get() {
                        view! {
                            <div style="display: flex; gap: 8px; align-items: center;">
                                <span style="font-size: 11px; color: rgba(255,245,240,0.7);">"Rotate key? This cannot be undone."</span>
                                <Button primary=true small=true on_click=Some(Callback::new(move |_| {
                                    confirm_rotate.set(false);
                                    rotating.set(true);
                                    spawn_local(async move {
                                        match api::rotate_api_key().await {
                                            Ok(_) => { if let Ok(r) = api::fetch_rotation_status().await { rotation.set(Some(r)); } }
                                            Err(e) => web_sys::console::warn_1(&format!("Rotation failed: {}", e).into()),
                                        }
                                        rotating.set(false);
                                    });
                                }))>"Confirm"</Button>
                                <Button small=true on_click=Some(Callback::new(move |_| { confirm_rotate.set(false); }))>"Cancel"</Button>
                            </div>
                        }.into_any()
                    } else {
                        view! {
                            <Button primary=true small=true on_click=Some(Callback::new(move |_| {
                                confirm_rotate.set(true);
                            }))>{move || if rotating.get() { "Rotating..." } else { "Rotate Key" }}</Button>
                        }.into_any()
                    }}
                </div>
                {move || rotation.get().map(|r| view! {
                    <div style="display: flex; gap: 16px; margin-top: 12px; font-size: 12px; color: rgba(255,245,240,0.7);">
                        <span>"Last: "{r.last_rotation.clone().unwrap_or_else(|| "never".to_string())}</span>
                        <span>"Next: "{r.next_rotation.clone().unwrap_or_else(|| "n/a".to_string())}</span>
                        <span>"Count: "{r.rotation_count.to_string()}</span>
                        <Badge text={if r.enabled { "ENABLED".to_string() } else { "DISABLED".to_string() }} color={if r.enabled { "var(--z-green)".to_string() } else { "var(--z-yellow)".to_string() }} />
                    </div>
                })}
            </Card>

            <div style="display: grid; grid-template-columns: 1fr 1fr; gap: 16px;">
                // Threat Log
                <Card>
                    <SectionTitle>"Recent Threat Log"</SectionTitle>
                    {move || {
                        let t = threats.get();
                        if t.is_empty() {
                            view! {
                                <div style="padding: 20px; text-align: center; color: rgba(255,245,240,0.7); font-size: 13px;">
                                    "No threats recorded"
                                </div>
                            }.into_any()
                        } else {
                            view! {
                                <div style="display: flex; flex-direction: column;">
                                    {t.into_iter().map(|threat| {
                                        let dot_color = match threat.severity.as_str() {
                                            "high" | "critical" => "#ef4444",
                                            "medium" | "warning" => "#eab308",
                                            _ => "#22c55e",
                                        };
                                        let badge_text = threat.threat_type.clone();
                                        let badge_color = if badge_text == "blocked" { "#ef4444".to_string() } else { "#22c55e".to_string() };
                                        view! {
                                            <div style="display: flex; align-items: center; gap: 12px; padding: 12px 0; border-bottom: 1px solid rgba(255,60,20,0.1);">
                                                <div style={format!("width: 8px; height: 8px; border-radius: 50%; background: {}; box-shadow: 0 0 8px {};", dot_color, dot_color)} />
                                                <div style="flex: 1;">
                                                    <div style="font-size: 13px; color: rgba(255,245,240,0.9);">{threat.detail.clone()}</div>
                                                    <div style="font-size: 11px; color: rgba(255,245,240,0.7); margin-top: 2px;">{threat.timestamp.clone()}</div>
                                                </div>
                                                <Badge text=badge_text color=badge_color />
                                            </div>
                                        }
                                    }).collect::<Vec<_>>()}
                                </div>
                            }.into_any()
                        }
                    }}
                </Card>

                // Audit Log
                <Card>
                    <SectionTitle>"Security Audit Log"</SectionTitle>
                    {move || {
                        let a = audit.get();
                        if a.is_empty() {
                            view! {
                                <div style="padding: 20px; text-align: center; color: rgba(255,245,240,0.7); font-size: 13px;">
                                    "No audit entries"
                                </div>
                            }.into_any()
                        } else {
                            view! {
                                <div style="display: flex; flex-direction: column; max-height: 400px; overflow-y: auto;">
                                    {a.into_iter().take(20).map(|entry| {
                                        let sev_color = match entry.severity.as_str() {
                                            "critical" | "high" => "#ef4444",
                                            "medium" | "warning" => "#eab308",
                                            "info" => "#3b82f6",
                                            _ => "#22c55e",
                                        };
                                        let outcome_color = if entry.outcome == "denied" || entry.outcome == "blocked" { "#ef4444".to_string() } else { "#22c55e".to_string() };
                                        view! {
                                            <div style="display: flex; align-items: flex-start; gap: 10px; padding: 10px 0; border-bottom: 1px solid rgba(255,60,20,0.06);">
                                                <div style={format!("width: 6px; height: 6px; border-radius: 50%; background: {}; margin-top: 5px; flex-shrink: 0;", sev_color)} />
                                                <div style="flex: 1; min-width: 0;">
                                                    <div style="display: flex; align-items: center; gap: 6px; margin-bottom: 2px;">
                                                        <span style="font-family: 'Orbitron', monospace; font-size: 10px; color: rgba(255,245,240,0.9); letter-spacing: 1px;">
                                                            {entry.tool.clone()}
                                                        </span>
                                                        {if !entry.action.is_empty() {
                                                            view! { <span style="font-size: 10px; color: rgba(255,245,240,0.5);">" → "{entry.action.clone()}</span> }.into_any()
                                                        } else {
                                                            view! { <span /> }.into_any()
                                                        }}
                                                    </div>
                                                    <div style="font-size: 11px; color: rgba(255,245,240,0.7); overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">{entry.detail.clone()}</div>
                                                    <div style="font-size: 10px; color: rgba(255,245,240,0.5); margin-top: 2px;">
                                                        {entry.user.clone()}" • "{entry.timestamp.clone()}
                                                    </div>
                                                </div>
                                                {if !entry.outcome.is_empty() {
                                                    view! { <Badge text={entry.outcome.clone()} color=outcome_color /> }.into_any()
                                                } else {
                                                    view! { <span /> }.into_any()
                                                }}
                                            </div>
                                        }
                                    }).collect::<Vec<_>>()}
                                </div>
                            }.into_any()
                        }
                    }}
                </Card>
            </div>
        </div>
    }
}
