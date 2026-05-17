// ═══════════════════════════════════════════════════════════
// ZEUS — Health / Doctor Page — Wired to API
// ═══════════════════════════════════════════════════════════

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;
use crate::api;
use crate::components::design::*;

#[component]
pub fn HealthPage() -> impl IntoView {
    let doctor = RwSignal::new(api::DoctorResponse::default());
    let stats = RwSignal::new(api::StatsResponse::default());
    let loading = RwSignal::new(true);

    let refresh = move || {
        let doctor = doctor;
        let stats = stats;
        let loading = loading;
        spawn_local(async move {
            loading.set(true);
            if let Ok(d) = api::fetch_doctor().await { doctor.set(d); }
            if let Ok(s) = api::fetch_stats().await { stats.set(s); }
            loading.set(false);
        });
    };

    refresh();

    view! {
        <div style="padding: 32px;">
            <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 24px;">
                <div>
                    <h1 style="font-family: 'Orbitron', monospace; font-size: 14px; letter-spacing: 6px; color: rgba(255,245,240,0.9); margin: 0;">"SYSTEM HEALTH"</h1>
                    <p style="font-size: 12px; color: rgba(255,245,240,0.7); margin: 4px 0 0;">
                        {move || {
                            let d = doctor.get();
                            if loading.get() { "Running diagnostics...".to_string() }
                            else {
                                let pass = d.checks.iter().filter(|c| c.status == "ok" || c.status == "pass").count();
                                format!("{} — {}/{} checks passed", d.overall, pass, d.checks.len())
                            }
                        }}
                    </p>
                </div>
                <div style="display: flex; gap: 8px; align-items: center;">
                    {move || {
                        let d = doctor.get();
                        let (color, label) = if d.healthy {
                            ("rgba(34,197,94,1)", "HEALTHY")
                        } else if d.checks.is_empty() {
                            ("rgba(255,245,240,0.5)", "CHECKING")
                        } else {
                            ("rgba(239,68,68,1)", "DEGRADED")
                        };
                        view! { <Badge text=label color=color /> }
                    }}
                    <Button on_click=Some(Callback::new(move |_| refresh()))>"Re-scan"</Button>
                </div>
            </div>

            // Stats overview
            {move || {
                let s = stats.get();
                view! {
                    <div style="display: flex; gap: 12px; margin-bottom: 24px; flex-wrap: wrap;">
                        <MetricCard label="Model" value={s.model.clone()} icon="cpu" />
                        <MetricCard label="Provider" value={s.provider.clone()} icon="activity" />
                        <MetricCard label="Sessions" value={s.sessions.total.to_string()} icon="sessions" />
                        <MetricCard label="Tools" value={s.tools.total.to_string()} icon="tools" />
                    </div>
                }
            }}

            // Doctor checks
            <Card>
                <SectionTitle>"Diagnostics"</SectionTitle>
                <div style="display: flex; flex-direction: column; gap: 8px;">
                    {move || {
                        doctor.get().checks.into_iter().map(|check| {
                            let (icon_color, bg) = match check.status.as_str() {
                                "ok" | "pass" => ("rgba(34,197,94,1)", "rgba(34,197,94,0.08)"),
                                "warn" | "warning" => ("rgba(234,179,8,1)", "rgba(234,179,8,0.08)"),
                                _ => ("rgba(239,68,68,1)", "rgba(239,68,68,0.08)"),
                            };
                            let status_icon = match check.status.as_str() {
                                "ok" | "pass" => "✓",
                                "warn" | "warning" => "⚠",
                                _ => "✗",
                            };
                            view! {
                                <div style={format!("display: flex; align-items: center; gap: 12px; padding: 10px 14px; background: {}; border-radius: 8px;", bg)}>
                                    <span style={format!("font-size: 14px; color: {}; width: 20px; text-align: center;", icon_color)}>
                                        {status_icon}
                                    </span>
                                    <div style="flex: 1;">
                                        <div style="font-family: 'Orbitron', monospace; font-size: 11px; letter-spacing: 2px; color: rgba(255,245,240,0.9); font-weight: 600;">
                                            {check.name.clone()}
                                        </div>
                                        {(!check.detail.is_empty()).then(|| view! {
                                            <div style="font-size: 11px; color: rgba(255,245,240,0.7); margin-top: 2px;">
                                                {check.detail.clone()}
                                            </div>
                                        })}
                                    </div>
                                    <Badge text={check.status.to_uppercase()} color={icon_color.to_string()} />
                                </div>
                            }
                        }).collect::<Vec<_>>()
                    }}
                </div>
            </Card>
        </div>
    }
}
