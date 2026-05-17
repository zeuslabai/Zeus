// ═══════════════════════════════════════════════════════════
// ZEUS — Analytics Page — Phase 3: Daily + Model breakdown
// ═══════════════════════════════════════════════════════════

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;
use crate::api;
use crate::components::design::*;

#[component]
pub fn AnalyticsPage() -> impl IntoView {
    let costs = RwSignal::new(api::CostsResponse::default());
    let tokens = RwSignal::new(api::TokensResponse::default());
    let providers = RwSignal::new(Vec::<api::ProviderCost>::new());
    let models = RwSignal::new(Vec::<api::ModelAnalytics>::new());
    let daily = RwSignal::new(Vec::<api::DailyAnalytics>::new());

    {
        let costs = costs;
        spawn_local(async move { if let Ok(c) = api::fetch_costs().await { costs.set(c); } });
    }
    {
        let tokens = tokens;
        spawn_local(async move { if let Ok(t) = api::fetch_tokens().await { tokens.set(t); } });
    }
    {
        let providers = providers;
        spawn_local(async move { if let Ok(p) = api::fetch_provider_costs().await { providers.set(p.providers); } });
    }
    {
        let models = models;
        spawn_local(async move { if let Ok(m) = api::fetch_model_analytics().await { models.set(m.models); } });
    }
    {
        let daily = daily;
        spawn_local(async move { if let Ok(d) = api::fetch_daily_analytics(7).await { daily.set(d.daily); } });
    }

    view! {
        <div style="padding: 32px;">
            <h1 style="font-family: 'Orbitron', monospace; font-size: 14px; letter-spacing: 6px; color: rgba(255,245,240,0.9); margin: 0 0 24px;">"ANALYTICS"</h1>
            // Metric Cards
            {move || {
                let c = costs.get();
                let t = tokens.get();
                let total_tokens = t.total_input_tokens + t.total_output_tokens;
                let token_str = if total_tokens > 1_000_000 {
                    format!("{:.1}M", total_tokens as f64 / 1_000_000.0)
                } else if total_tokens > 1_000 {
                    format!("{:.0}K", total_tokens as f64 / 1_000.0)
                } else {
                    total_tokens.to_string()
                };
                view! {
                    <div style="display: flex; gap: 12px; margin-bottom: 24px; flex-wrap: wrap;">
                        <MetricCard label="Total Cost" value={format!("${:.2}", c.this_month)} icon="analytics" trend=Some(format!("+${:.2} today", c.today)) />
                        <MetricCard label="Total Tokens" value=token_str icon="cpu" trend=Some(format!("{}in / {}out",
                            if t.total_input_tokens > 1_000_000 { format!("{:.1}M ", t.total_input_tokens as f64 / 1_000_000.0) } else { format!("{}K ", t.total_input_tokens / 1_000) },
                            if t.total_output_tokens > 1_000_000 { format!("{:.1}M", t.total_output_tokens as f64 / 1_000_000.0) } else { format!("{}K", t.total_output_tokens / 1_000) }
                        )) />
                        <MetricCard label="Sessions" value={c.session_count.to_string()} icon="sessions" />
                        <MetricCard label="Models Used" value={models.get().len().to_string()} icon="agents" />
                    </div>
                }
            }}

            <div style="display: grid; grid-template-columns: 1fr 1fr; gap: 16px; margin-bottom: 16px;">
                // Daily Cost Trend (last 7 days)
                <Card>
                    <SectionTitle>"7-Day Cost Trend"</SectionTitle>
                    {move || {
                        let d = daily.get();
                        if d.is_empty() {
                            view! { <div style="padding: 16px; color: rgba(255,245,240,0.7); font-size: 13px;">"Loading daily data..."</div> }.into_any()
                        } else {
                            let max_cost = d.iter().map(|dp| dp.estimated_cost).fold(0.0f64, f64::max).max(0.01);
                            view! {
                                <div style="display: flex; flex-direction: column; gap: 8px;">
                                    {d.into_iter().map(|dp| {
                                        let pct = (dp.estimated_cost / max_cost) * 100.0;
                                        let date_short = if dp.date.len() >= 10 { dp.date[5..10].to_string() } else { dp.date.clone() };
                                        view! {
                                            <div style="display: flex; align-items: center; gap: 10px;">
                                                <span style="font-family: 'Orbitron', monospace; font-size: 10px; color: rgba(255,245,240,0.7); min-width: 44px;">{date_short}</span>
                                                <div style="flex: 1; height: 3px; background: rgba(255,255,255,0.03); border-radius: 2px; overflow: hidden;">
                                                    <div style={format!("width: {:.0}%; height: 100%; background: #ff3c14; border-radius: 2px;", pct)} />
                                                </div>
                                                <span style="font-family: 'Orbitron', monospace; font-size: 10px; color: rgba(255,245,240,0.9); min-width: 52px; text-align: right;">{format!("${:.2}", dp.estimated_cost)}</span>
                                            </div>
                                        }
                                    }).collect::<Vec<_>>()}
                                </div>
                            }.into_any()
                        }
                    }}
                </Card>

                // Model Breakdown
                <Card>
                    <SectionTitle>"Model Breakdown"</SectionTitle>
                    {move || {
                        let m = models.get();
                        if m.is_empty() {
                            view! { <div style="padding: 16px; color: rgba(255,245,240,0.7); font-size: 13px;">"Loading model data..."</div> }.into_any()
                        } else {
                            let total_cost: f64 = m.iter().map(|md| md.estimated_cost).sum();
                            view! {
                                <div style="display: flex; flex-direction: column; gap: 10px;">
                                    {m.into_iter().map(|md| {
                                        let pct = if total_cost > 0.0 { (md.estimated_cost / total_cost) * 100.0 } else { 0.0 };
                                        let tok_str = if md.total_tokens > 1_000_000 {
                                            format!("{:.1}M", md.total_tokens as f64 / 1_000_000.0)
                                        } else if md.total_tokens > 1_000 {
                                            format!("{:.0}K", md.total_tokens as f64 / 1_000.0)
                                        } else {
                                            md.total_tokens.to_string()
                                        };
                                        let color = if md.model.contains("claude") { "#ff3c14" }
                                            else if md.model.contains("gpt") { "#f97316" }
                                            else if md.model.contains("llama") || md.model.contains("ollama") { "#22c55e" }
                                            else if md.model.contains("groq") { "#3b82f6" }
                                            else { "#eab308" };
                                        view! {
                                            <div>
                                                <div style="display: flex; justify-content: space-between; margin-bottom: 4px;">
                                                    <span style="font-size: 12px; color: rgba(255,245,240,0.9); font-weight: 500;">{md.model.clone()}</span>
                                                    <div style="display: flex; gap: 12px;">
                                                        <span style="font-size: 11px; color: rgba(255,245,240,0.7);">{tok_str}" tok"</span>
                                                        <span style="font-size: 11px; color: rgba(255,245,240,0.7);">{md.requests.to_string()}" req"</span>
                                                        <span style="font-family: 'Orbitron', monospace; font-size: 11px; color: rgba(255,245,240,0.9); font-weight: 600; min-width: 52px; text-align: right;">{format!("${:.2}", md.estimated_cost)}</span>
                                                    </div>
                                                </div>
                                                <ProgressBar value=pct color={color.to_string()} />
                                            </div>
                                        }
                                    }).collect::<Vec<_>>()}
                                </div>
                            }.into_any()
                        }
                    }}
                </Card>
            </div>

            // Provider Breakdown (existing)
            <Card>
                <SectionTitle>"Provider Breakdown"</SectionTitle>
                <div style="display: flex; flex-direction: column; gap: 14px;">
                    {move || {
                        let provs = providers.get();
                        let total_cost: f64 = provs.iter().map(|p| p.cost).sum();
                        provs.into_iter().map(|p| {
                            let pct = if total_cost > 0.0 { (p.cost / total_cost) * 100.0 } else { 0.0 };
                            let token_str = if p.tokens > 1_000_000 {
                                format!("{:.2}M", p.tokens as f64 / 1_000_000.0)
                            } else if p.tokens > 1_000 {
                                format!("{:.0}K", p.tokens as f64 / 1_000.0)
                            } else {
                                p.tokens.to_string()
                            };
                            let color = match p.provider.to_lowercase().as_str() {
                                s if s.contains("anthropic") => "#ff3c14",
                                s if s.contains("openai") => "#f97316",
                                s if s.contains("ollama") => "#22c55e",
                                s if s.contains("groq") => "#3b82f6",
                                _ => "#eab308",
                            };
                            view! {
                                <div>
                                    <div style="display: flex; justify-content: space-between; margin-bottom: 6px;">
                                        <span style="font-size: 13px; color: rgba(255,245,240,0.9); font-weight: 500;">{p.provider.clone()}</span>
                                        <div style="display: flex; gap: 16px;">
                                            <span style="font-size: 12px; color: rgba(255,245,240,0.7);">{token_str}</span>
                                            <span style="font-size: 12px; color: rgba(255,245,240,0.9); font-weight: 600; min-width: 60px; text-align: right;">{format!("${:.2}", p.cost)}</span>
                                            <span style="font-family: 'Orbitron', monospace; font-size: 11px; color: rgba(255,245,240,0.7); min-width: 36px; text-align: right;">{format!("{:.0}%", pct)}</span>
                                        </div>
                                    </div>
                                    <ProgressBar value=pct color={color.to_string()} />
                                </div>
                            }
                        }).collect::<Vec<_>>()
                    }}
                </div>
            </Card>
        </div>
    }
}
