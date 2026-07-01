// ═══════════════════════════════════════════════════════════
// ZEUS — Agora Marketplace Page — Phase 3: Server-side featured + stats
// ═══════════════════════════════════════════════════════════

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;
use crate::api;
use crate::components::design::*;

const CATEGORIES: &[&str] = &[
    "all", "development", "messaging", "infrastructure",
    "security", "writing", "research", "data", "automation",
];

fn trust_badge_html(level: i8) -> impl IntoView {
    if level < 0 {
        return view! { <span /> }.into_any();
    }
    let (label, color, bg) = match level {
        2 => ("TRUSTED", "rgba(34,197,94,0.9)", "rgba(34,197,94,0.12)"),
        1 => ("BASIC", "rgba(234,179,8,0.9)", "rgba(234,179,8,0.12)"),
        _ => ("RESTRICTED", "rgba(239,68,68,0.9)", "rgba(239,68,68,0.12)"),
    };
    let icon = match level {
        2 => "\u{1f6e1}\u{fe0f}",  // shield
        1 => "\u{26a0}\u{fe0f}",   // warning
        _ => "\u{1f512}",          // lock
    };
    view! {
        <span style=format!(
            "display: inline-flex; align-items: center; gap: 3px; padding: 1px 6px; border-radius: 10px; \
            font-family: 'Orbitron', monospace; font-size: 7px; letter-spacing: 1px; font-weight: 700; \
            color: {}; background: {};", color, bg
        )>
            <span style="font-size: 8px;">{icon}</span>
            {label}
        </span>
    }.into_any()
}

#[component]
pub fn AgoraPage() -> impl IntoView {
    let listings = RwSignal::new(Vec::<api::MarketplaceListing>::new());
    let featured = RwSignal::new(Vec::<api::MarketplaceListing>::new());
    let stats = RwSignal::new(Option::<api::MarketplaceStats>::None);
    let search = RwSignal::new(String::new());
    let active_category = RwSignal::new("all".to_string());
    let loading = RwSignal::new(true);
    let toast_msg = RwSignal::new(String::new());
    let toast_ok = RwSignal::new(true);
    let acquiring = RwSignal::new(Option::<String>::None);
    let rating_skill = RwSignal::new(Option::<api::MarketplaceListing>::None);
    let rating_reviews = RwSignal::new(Vec::<api::SkillRating>::new());
    let rating_loading = RwSignal::new(false);
    let user_score = RwSignal::new(0u8);
    let user_comment = RwSignal::new(String::new());
    let rating_submitting = RwSignal::new(false);
    let rating_error = RwSignal::new(String::new());

    // ── Tab state ──
    let active_tab = RwSignal::new("marketplace".to_string());
    let bounties = RwSignal::new(Vec::<api::Bounty>::new());
    let bounties_loading = RwSignal::new(false);
    let bounties_loaded = RwSignal::new(false);
    let economy = RwSignal::new(Option::<api::PantheonEconomyResponse>::None);
    let economy_loading = RwSignal::new(false);
    let economy_loaded = RwSignal::new(false);
    let claiming = RwSignal::new(Option::<String>::None);
    // ── Phase 6: Hiring + Reputation signals ──
    let hiring_agents = RwSignal::new(Vec::<api::FleetAgent>::new());
    let hiring_loading = RwSignal::new(false);
    let hiring_loaded = RwSignal::new(false);
    let hiring_search = RwSignal::new(String::new());
    let hiring_selected = RwSignal::new(Option::<api::FleetAgent>::None);
    let hiring_task = RwSignal::new(String::new());
    let hiring_submitting = RwSignal::new(false);
    let hiring_result = RwSignal::new(String::new());
    let rep_agent_id = RwSignal::new(String::new());
    let rep_data = RwSignal::new(Option::<api::ReputationResponse>::None);
    let rep_loading = RwSignal::new(false);
    // Staking signals
    let stake_agent_id = RwSignal::new(String::new());
    let stake_amount = RwSignal::new(String::new());
    let stake_purpose = RwSignal::new(String::new());
    let unstake_id = RwSignal::new(String::new());
    let transfer_to = RwSignal::new(String::new());
    let transfer_amount = RwSignal::new(String::new());
    let transfer_note = RwSignal::new(String::new());
    let staking_op = RwSignal::new("stake".to_string()); // "stake" | "unstake" | "transfer"
    let staking_busy = RwSignal::new(false);
    let staking_result = RwSignal::new(String::new());
    let staking_ok = RwSignal::new(true);
    let wallet_preview = RwSignal::new(Option::<api::Wallet>::None);
    let tx_history: RwSignal<Vec<api::Transaction>> = RwSignal::new(Vec::new());
    let active_stakes: RwSignal<Vec<api::Stake>> = RwSignal::new(Vec::new());
    let stakes_loading = RwSignal::new(false);

    // Claim bounty handler
    let do_claim = move |bid: String, btitle: String| {
        claiming.set(Some(bid.clone()));
        toast_msg.set(String::new());
        spawn_local(async move {
            match api::claim_bounty(&bid, "web-user").await {
                Ok(_) => {
                    toast_ok.set(true);
                    toast_msg.set(format!("Claimed: {}", btitle));
                    if let Ok(r) = api::fetch_bounties(Some("open")).await { bounties.set(r.bounties); }
                }
                Err(e) => { toast_ok.set(false); toast_msg.set(format!("Claim failed: {}", e)); }
            }
            claiming.set(None);
        });
    };

    // Initial load — fetch listings, featured, and stats in parallel
    {
        spawn_local(async move {
            let (listings_res, featured_res, stats_res) = (
                api::fetch_marketplace_listings(None, None, None, None).await,
                api::fetch_marketplace_featured(5).await,
                api::fetch_marketplace_stats().await,
            );
            if let Ok(r) = listings_res { listings.set(r.listings); }
            if let Ok(r) = featured_res { featured.set(r.listings); }
            if let Ok(s) = stats_res { stats.set(Some(s)); }
            loading.set(false);
        });
    }

    // Acquire handler
    let do_acquire = move |lid: String, lname: String, lprice: u64| {
        acquiring.set(Some(lid.clone()));
        toast_msg.set(String::new());
        spawn_local(async move {
            let trade_req = api::TradeRequest {
                buyer_id: "local".to_string(),
                skill_id: lid.clone(),
                offered_price: lprice,
            };
            match api::marketplace_trade(&trade_req).await {
                Ok(_) => {
                    let install_req = api::InstallSkillReq { name: lname.clone(), content: None };
                    match api::install_skill(&install_req).await {
                        Ok(_) => { toast_ok.set(true); toast_msg.set(format!("Installed: {}", lname)); }
                        Err(e) => { toast_ok.set(true); toast_msg.set(format!("Acquired {} (install: {})", lname, e)); }
                    }
                }
                Err(e) => { toast_ok.set(false); toast_msg.set(format!("Failed: {}", e)); }
            }
            acquiring.set(None);
            if let Ok(r) = api::fetch_marketplace_listings(None, None, None, None).await { listings.set(r.listings); }
        });
    };

    // Open rating modal
    let open_rating = move |listing: api::MarketplaceListing| {
        let skill_id = listing.id.clone();
        rating_skill.set(Some(listing));
        rating_loading.set(true);
        rating_reviews.set(Vec::new());
        user_score.set(0);
        user_comment.set(String::new());
        rating_error.set(String::new());
        spawn_local(async move {
            if let Ok(resp) = api::fetch_skill_ratings(&skill_id).await {
                rating_reviews.set(resp.ratings);
            }
            rating_loading.set(false);
        });
    };

    // Submit rating
    let do_submit_rating = move || {
        let score = user_score.get();
        if score == 0 { return; }
        let skill = rating_skill.get();
        let Some(skill) = skill else { return };
        let skill_id = skill.id.clone();
        let comment = user_comment.get();
        let comment_opt = if comment.is_empty() { None } else { Some(comment.as_str().to_string()) };
        rating_submitting.set(true);
        rating_error.set(String::new());
        spawn_local(async move {
            match api::submit_skill_rating(&skill_id, "web-user", score as f64, comment_opt.as_deref()).await {
                Ok(_) => {
                    user_score.set(0);
                    user_comment.set(String::new());
                    if let Ok(resp) = api::fetch_skill_ratings(&skill_id).await {
                        rating_reviews.set(resp.ratings);
                    }
                    if let Ok(r) = api::fetch_marketplace_listings(None, None, None, None).await {
                        listings.set(r.listings);
                    }
                }
                Err(e) => { rating_error.set(e); }
            }
            rating_submitting.set(false);
        });
    };

    view! {
        <div style="padding: 32px;">
            // ── Toast notification ──
            <Show when=move || !toast_msg.get().is_empty()>
                <div style=move || format!(
                    "position: fixed; top: 20px; right: 20px; z-index: 9999; padding: 12px 20px; border-radius: 10px; \
                    font-size: 13px; font-family: 'Rajdhani', sans-serif; \
                    background: {}; border: 1px solid {}; color: rgba(255,245,240,0.9); \
                    box-shadow: 0 4px 20px rgba(0,0,0,0.4); cursor: pointer;",
                    if toast_ok.get() { "rgba(34,197,94,0.15)" } else { "rgba(239,68,68,0.15)" },
                    if toast_ok.get() { "rgba(34,197,94,0.3)" } else { "rgba(239,68,68,0.3)" },
                ) on:click=move |_| toast_msg.set(String::new())>
                    {move || toast_msg.get()}
                </div>
            </Show>

            // ── Header + Stats ──
            <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 24px;">
                <div>
                    <h1 style="font-family: 'Orbitron', monospace; font-size: 14px; letter-spacing: 6px; color: rgba(255,245,240,0.9); margin: 0;">"AGORA MARKETPLACE"</h1>
                    <p style="font-size: 12px; color: rgba(255,245,240,0.7); margin: 4px 0 0;">{move || {
                        if loading.get() { return "Loading marketplace...".to_string(); }
                        match stats.get() {
                            Some(s) => format!("{} skills \u{00b7} {} active \u{00b7} {} trades \u{00b7} {} downloads",
                                s.total_listings, s.active_listings, s.total_trades, s.total_downloads),
                            None => {
                                let l = listings.get();
                                if l.is_empty() { "No listings available".to_string() }
                                else {
                                    let free = l.iter().filter(|s| s.price_tokens == 0).count();
                                    format!("{} skills \u{00b7} {} free", l.len(), free)
                                }
                            }
                        }
                    }}</p>
                </div>
                <Button primary=true on_click=Some(Callback::new(move |_| {
                    let _ = web_sys::window().unwrap().location().assign("/skills");
                }))>
                    <Icon name="skills" size=12 /> " My Skills"
                </Button>
            </div>

            // ── Tab Bar ──
            // design: left raw — stateful toggle (bg+border+color flip per active_tab signal)
            <div style="display: flex; gap: 4px; margin-bottom: 24px; border-bottom: 1px solid rgba(255,60,20,0.08); padding-bottom: 12px;">
                <button
                    on:click=move |_| active_tab.set("marketplace".to_string())
                    style=move || format!(
                        "padding: 8px 18px; border-radius: 8px; font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; \
                        cursor: pointer; border: 1px solid {}; background: {}; color: {};",
                        if active_tab.get() == "marketplace" { "rgba(255,60,20,0.4)" } else { "rgba(255,245,240,0.08)" },
                        if active_tab.get() == "marketplace" { "rgba(255,60,20,0.15)" } else { "transparent" },
                        if active_tab.get() == "marketplace" { "rgba(255,245,240,0.9)" } else { "rgba(255,245,240,0.35)" },
                    )>"\u{1f6d2} MARKETPLACE"</button>
                <button
                    on:click=move |_| {
                        active_tab.set("bounties".to_string());
                        if !bounties_loaded.get() {
                            bounties_loading.set(true);
                            spawn_local(async move {
                                if let Ok(r) = api::fetch_bounties(Some("open")).await { bounties.set(r.bounties); }
                                bounties_loading.set(false);
                                bounties_loaded.set(true);
                            });
                        }
                    }
                    style=move || format!(
                        "padding: 8px 18px; border-radius: 8px; font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; \
                        cursor: pointer; border: 1px solid {}; background: {}; color: {};",
                        if active_tab.get() == "bounties" { "rgba(234,179,8,0.4)" } else { "rgba(255,245,240,0.08)" },
                        if active_tab.get() == "bounties" { "rgba(234,179,8,0.12)" } else { "transparent" },
                        if active_tab.get() == "bounties" { "rgba(234,179,8,0.95)" } else { "rgba(255,245,240,0.35)" },
                    )>"\u{1f4b0} BOUNTIES"</button>
                <button
                    on:click=move |_| {
                        active_tab.set("economy".to_string());
                        if !economy_loaded.get() {
                            economy_loading.set(true);
                            spawn_local(async move {
                                if let Ok(r) = api::fetch_pantheon_economy().await { economy.set(Some(r)); }
                                economy_loading.set(false);
                                economy_loaded.set(true);
                            });
                        }
                    }
                    style=move || format!(
                        "padding: 8px 18px; border-radius: 8px; font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; \
                        cursor: pointer; border: 1px solid {}; background: {}; color: {};",
                        if active_tab.get() == "economy" { "rgba(34,197,94,0.4)" } else { "rgba(255,245,240,0.08)" },
                        if active_tab.get() == "economy" { "rgba(34,197,94,0.12)" } else { "transparent" },
                        if active_tab.get() == "economy" { "rgba(34,197,94,0.95)" } else { "rgba(255,245,240,0.35)" },
                    )>"\u{1f4ca} ECONOMY"</button>
                <button
                    on:click=move |_| {
                        active_tab.set("hiring".to_string());
                        if !hiring_loaded.get() {
                            hiring_loading.set(true);
                            spawn_local(async move {
                                if let Ok(agents) = api::fetch_fleet_agents().await { hiring_agents.set(agents); }
                                hiring_loading.set(false);
                                hiring_loaded.set(true);
                            });
                        }
                    }
                    style=move || format!(
                        "padding: 8px 18px; border-radius: 8px; font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; \
                        cursor: pointer; border: 1px solid {}; background: {}; color: {};",
                        if active_tab.get() == "hiring" { "rgba(59,130,246,0.4)" } else { "rgba(255,245,240,0.08)" },
                        if active_tab.get() == "hiring" { "rgba(59,130,246,0.12)" } else { "transparent" },
                        if active_tab.get() == "hiring" { "rgba(59,130,246,0.95)" } else { "rgba(255,245,240,0.35)" },
                    )>"\u{1f465} HIRING"</button>
                <button
                    on:click=move |_| active_tab.set("reputation".to_string())
                    style=move || format!(
                        "padding: 8px 18px; border-radius: 8px; font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; \
                        cursor: pointer; border: 1px solid {}; background: {}; color: {};",
                        if active_tab.get() == "reputation" { "rgba(168,85,247,0.4)" } else { "rgba(255,245,240,0.08)" },
                        if active_tab.get() == "reputation" { "rgba(168,85,247,0.12)" } else { "transparent" },
                        if active_tab.get() == "reputation" { "rgba(168,85,247,0.95)" } else { "rgba(255,245,240,0.35)" },
                    )>"\u{2b50} REPUTATION"</button>
                <button
                    on:click=move |_| active_tab.set("staking".to_string())
                    style=move || format!(
                        "padding: 8px 18px; border-radius: 8px; font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; \
                        cursor: pointer; border: 1px solid {}; background: {}; color: {};",
                        if active_tab.get() == "staking" { "rgba(255,60,20,0.4)" } else { "rgba(255,245,240,0.08)" },
                        if active_tab.get() == "staking" { "rgba(255,60,20,0.15)" } else { "transparent" },
                        if active_tab.get() == "staking" { "rgba(255,245,240,0.9)" } else { "rgba(255,245,240,0.35)" },
                    )>"\u{1f4b8} STAKING"</button>
            </div>

            // ── Bounties Tab ──
            <Show when=move || active_tab.get() == "bounties">
                {move || {
                    if bounties_loading.get() {
                        return view! { <div style="text-align: center; padding: 60px; color: rgba(255,245,240,0.3); font-size: 13px;">"Loading bounties..."</div> }.into_any();
                    }
                    let blist = bounties.get();
                    if blist.is_empty() {
                        return view! {
                            <div style="display: flex; flex-direction: column; align-items: center; padding: 60px 0; gap: 12px;">
                                <div style="font-size: 36px;">"\u{1f4b0}"</div>
                                <div style="font-family: 'Orbitron', monospace; font-size: 12px; letter-spacing: 4px; color: rgba(255,245,240,0.4);">"NO OPEN BOUNTIES"</div>
                                <div style="font-size: 12px; color: rgba(255,245,240,0.3);">"Bounties appear here when agents post tasks for hire"</div>
                            </div>
                        }.into_any();
                    }
                    let dc = do_claim;
                    view! {
                        <div>
                            <div style="font-size: 12px; color: rgba(255,245,240,0.35); margin-bottom: 16px;">{format!("{} open bounties", blist.len())}</div>
                            <div style="display: grid; grid-template-columns: repeat(auto-fill, minmax(320px, 1fr)); gap: 14px;">
                                {blist.into_iter().map(|b| {
                                    let bid = b.id.clone();
                                    let btitle = b.title.clone();
                                    let bid2 = b.id.clone();
                                    let dc2 = dc;
                                    view! {
                                        <Card>
                                            <div style="display: flex; justify-content: space-between; align-items: flex-start; margin-bottom: 10px;">
                                                <div style="font-family: 'Orbitron', monospace; font-size: 12px; color: rgba(255,245,240,0.9); font-weight: 600; flex: 1; margin-right: 10px;">{b.title.clone()}</div>
                                                <div style="font-family: 'Orbitron', monospace; font-size: 12px; padding: 4px 10px; border-radius: 8px; background: rgba(234,179,8,0.1); border: 1px solid rgba(234,179,8,0.25); white-space: nowrap; color: rgba(234,179,8,1); flex-shrink: 0;">
                                                    {format!("{} \u{26a1}", b.reward_credits)}
                                                </div>
                                            </div>
                                            {(!b.description.is_empty()).then(|| view! {
                                                <div style="font-size: 12px; color: rgba(255,245,240,0.7); margin-bottom: 10px; line-height: 1.5; display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden;">
                                                    {b.description.clone()}
                                                </div>
                                            })}
                                            <div style="display: flex; justify-content: space-between; align-items: center; padding-top: 8px; border-top: 1px solid rgba(255,60,20,0.08);">
                                                <div style="font-size: 10px; color: rgba(255,245,240,0.3);">
                                                    "posted by "{b.poster_name.clone()}
                                                </div>
                                                <Button small=true primary=true on_click=Some(Callback::new(move |_| dc2(bid.clone(), btitle.clone())))>
                                                    {move || {
                                                        let c = claiming.get();
                                                        if c.as_deref() == Some(bid2.as_str()) { "Claiming..." } else { "Claim" }
                                                    }}
                                                </Button>
                                            </div>
                                        </Card>
                                    }
                                }).collect::<Vec<_>>()}
                            </div>
                        </div>
                    }.into_any()
                }}
            </Show>

            // ── Economy Tab ──
            <Show when=move || active_tab.get() == "economy">
                {move || {
                    if economy_loading.get() {
                        return view! { <div style="text-align: center; padding: 60px; color: rgba(255,245,240,0.3); font-size: 13px;">"Loading economy data..."</div> }.into_any();
                    }
                    let eco = economy.get();
                    let Some(eco) = eco else {
                        return view! {
                            <div style="text-align: center; padding: 60px 0; color: rgba(255,245,240,0.3); font-size: 13px;">
                                "Economy data unavailable — switch to Bounties tab to load"
                            </div>
                        }.into_any();
                    };
                    let agents = eco.agents.clone();
                    let stats = eco.stats.clone();
                    view! {
                        <div>
                            // Stats overview cards
                            <div style="display: grid; grid-template-columns: repeat(auto-fill, minmax(180px, 1fr)); gap: 12px; margin-bottom: 28px;">
                                {[
                                    ("TOTAL VOLUME", stats.get("total_volume").and_then(|v| v.as_u64()).map(|n| format!("{} \u{26a1}", n)).unwrap_or_else(|| "\u{2014}".to_string())),
                                    ("ACTIVE AGENTS", stats.get("active_agents").and_then(|v| v.as_u64()).map(|n| n.to_string()).unwrap_or_else(|| "\u{2014}".to_string())),
                                    ("OPEN BOUNTIES", stats.get("open_bounties").and_then(|v| v.as_u64()).map(|n| n.to_string()).unwrap_or_else(|| "\u{2014}".to_string())),
                                    ("TRADES TODAY", stats.get("trades_today").and_then(|v| v.as_u64()).map(|n| n.to_string()).unwrap_or_else(|| "\u{2014}".to_string())),
                                ].iter().map(|(label, value)| {
                                    let l = *label;
                                    let v = value.clone();
                                    view! {
                                        <Card style="background: rgba(34,197,94,0.04); border: 1px solid rgba(34,197,94,0.1); padding: 16px;">
                                            <div style="font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 2px; color: rgba(255,245,240,0.35); margin-bottom: 8px;">{l}</div>
                                            <div style="font-family: 'Orbitron', monospace; font-size: 22px; color: rgba(34,197,94,0.9);">{v}</div>
                                        </Card>
                                    }
                                }).collect::<Vec<_>>()}
                            </div>
                            // Agent wallet leaderboard
                            {(!agents.is_empty()).then(move || {
                                view! {
                                    <div>
                                        <div style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 4px; color: rgba(255,245,240,0.5); margin-bottom: 14px;">"AGENT WALLETS"</div>
                                        <div style="display: grid; grid-template-columns: repeat(auto-fill, minmax(250px, 1fr)); gap: 10px;">
                                            {agents.iter().map(|agent| {
                                                let name = agent.get("agent_name").or_else(|| agent.get("agent_id"))
                                                    .and_then(|v| v.as_str()).unwrap_or("Unknown").to_string();
                                                let balance = agent.get("balance").and_then(|v| v.as_u64()).unwrap_or(0);
                                                let reputation = agent.get("reputation").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                                let (badge_color, badge_bg) = match reputation.as_str() {
                                                    "Elite" => ("rgba(234,179,8,0.9)", "rgba(234,179,8,0.1)"),
                                                    "Trusted" => ("rgba(34,197,94,0.9)", "rgba(34,197,94,0.1)"),
                                                    "Reliable" => ("rgba(59,130,246,0.9)", "rgba(59,130,246,0.1)"),
                                                    "Rising" => ("rgba(168,85,247,0.9)", "rgba(168,85,247,0.1)"),
                                                    _ => ("rgba(255,245,240,0.3)", "rgba(255,245,240,0.05)"),
                                                };
                                                let first = name.chars().next().unwrap_or('?').to_uppercase().to_string();
                                                view! {
                                                    <div style="background: rgba(255,245,240,0.03); border: 1px solid rgba(255,60,20,0.08); border-radius: 12px; padding: 14px; display: flex; align-items: center; gap: 12px;">
                                                        <div style="width: 36px; height: 36px; border-radius: 50%; background: rgba(255,60,20,0.12); display: flex; align-items: center; justify-content: center; font-size: 14px; font-weight: 700; color: rgba(255,245,240,0.6); flex-shrink: 0;">
                                                            {first}
                                                        </div>
                                                        <div style="flex: 1; min-width: 0;">
                                                            <div style="font-size: 13px; font-weight: 600; color: rgba(255,245,240,0.85); white-space: nowrap; overflow: hidden; text-overflow: ellipsis;">{name}</div>
                                                            <div style="display: flex; align-items: center; gap: 8px; margin-top: 4px;">
                                                                <span style="font-family: 'Orbitron', monospace; font-size: 11px; color: rgba(255,140,80,0.9);">{format!("{} \u{26a1}", balance)}</span>
                                                                {(!reputation.is_empty()).then(move || view! {
                                                                    <span style=format!("font-size: 9px; padding: 2px 7px; border-radius: 10px; background: {}; color: {}; font-family: 'Orbitron', monospace; font-weight: 700; letter-spacing: 1px;", badge_bg, badge_color)>
                                                                        {reputation}
                                                                    </span>
                                                                })}
                                                            </div>
                                                        </div>
                                                    </div>
                                                }
                                            }).collect::<Vec<_>>()}
                                        </div>
                                    </div>
                                }
                            })}
                        </div>
                    }.into_any()
                }}
            </Show>

            // ── Marketplace Tab content (Featured + Categories + Listings) ──
            <Show when=move || active_tab.get() == "marketplace">

            // ── Featured Section (server-side) ──
            {move || {
                let feat = featured.get();
                if loading.get() || feat.is_empty() {
                    return view! { <div /> }.into_any();
                }

                let acquire = do_acquire;
                view! {
                    <div style="margin-bottom: 28px;">
                        <div style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 4px; color: rgba(255,245,240,0.5); margin-bottom: 12px;">
                            "\u{2b50} FEATURED"
                        </div>
                        <div style="display: flex; gap: 14px; overflow-x: auto; padding-bottom: 8px;">
                            {feat.into_iter().map(|listing| {
                                let lid = listing.id.clone();
                                let lname = listing.name.clone();
                                let lprice = listing.price_tokens;
                                let acquire = acquire;
                                view! {
                                    <div style="min-width: 260px; max-width: 300px; flex-shrink: 0; \
                                        background: linear-gradient(135deg, rgba(255,60,20,0.08), rgba(255,140,80,0.04)); \
                                        border: 1px solid rgba(255,60,20,0.2); border-radius: 14px; padding: 16px;">
                                        <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 8px;">
                                            <div style="display: flex; align-items: center; gap: 8px;">
                                                <div style="font-family: 'Orbitron', monospace; font-size: 12px; color: rgba(255,245,240,0.95); font-weight: 600;">
                                                    {listing.name.clone()}
                                                </div>
                                                {trust_badge_html(listing.trust_level())}
                                            </div>
                                            <div style="font-family: 'Orbitron', monospace; font-size: 10px; color: rgba(255,140,80,1);">
                                                {if listing.price_tokens == 0 { "FREE".to_string() } else { format!("{} \u{26a1}", listing.price_tokens) }}
                                            </div>
                                        </div>
                                        <div style="font-size: 11px; color: rgba(255,245,240,0.4); margin-bottom: 10px; line-height: 1.4; \
                                            display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden;">
                                            {if listing.description.is_empty() { "No description.".to_string() } else { listing.description.clone() }}
                                        </div>
                                        <div style="display: flex; justify-content: space-between; align-items: center;">
                                            <div style="display: flex; gap: 10px; font-size: 10px; color: rgba(255,245,240,0.35);">
                                                {
                                                    let listing_for_rate = listing.clone();
                                                    let open_rating = open_rating;
                                                    view! {
                                                        <span style="cursor: pointer;" on:click=move |_| open_rating(listing_for_rate.clone())>
                                                            "\u{2605} "{format!("{:.1}", listing.rating)}
                                                        </span>
                                                    }
                                                }
                                                <span>{listing.downloads}" dl"</span>
                                            </div>
                                            <Button small=true primary=true on_click=Some(Callback::new(move |_| {
                                                acquire(lid.clone(), lname.clone(), lprice);
                                            }))>"Get"</Button>
                                        </div>
                                    </div>
                                }
                            }).collect::<Vec<_>>()}
                        </div>
                    </div>
                }.into_any()
            }}

            // ── Category Tabs ──
            <div style="display: flex; gap: 4px; margin-bottom: 14px; flex-wrap: wrap;">
                {CATEGORIES.iter().map(|cat| {
                    let cat_str = cat.to_string();
                    let cat_style = cat.to_string();
                    let cat_count = cat.to_string();
                    let cat_upper = if *cat == "all" { "ALL".to_string() } else { cat.to_uppercase() };
                    view! {
                        // design: left raw — stateful toggle (bg+border+color flip per active_category signal)
                        <button
                            on:click=move |_| active_category.set(cat_str.clone())
                            style=move || {
                                let is_active = active_category.get() == cat_style;
                                format!(
                                    "padding: 6px 14px; border-radius: 6px; font-family: 'Orbitron', monospace; font-size: 8px; \
                                    letter-spacing: 2px; cursor: pointer; border: 1px solid {}; background: {}; color: {}; transition: all 0.15s;",
                                    if is_active { "rgba(255,60,20,0.4)" } else { "rgba(255,245,240,0.08)" },
                                    if is_active { "rgba(255,60,20,0.15)" } else { "transparent" },
                                    if is_active { "rgba(255,245,240,0.9)" } else { "rgba(255,245,240,0.35)" },
                                )
                            }
                        >
                            {cat_upper}
                            {move || {
                                if cat_count == "all" { return String::new(); }
                                let all = listings.get();
                                let count = all.iter()
                                    .filter(|l| l.tags.iter().any(|t| t.to_lowercase() == cat_count))
                                    .count();
                                if count > 0 { format!(" ({})", count) } else { String::new() }
                            }}
                        </button>
                    }
                }).collect::<Vec<_>>()}
            </div>

            // ── Search ──
            <SearchBar placeholder="Search skills by name, description, or tags..." value=search />

            // ── Listings Grid ──
            {move || {
                if loading.get() {
                    return view! {
                        <div style="display: flex; justify-content: center; padding: 60px 0; color: rgba(255,245,240,0.3); font-size: 13px;">
                            "Loading marketplace..."
                        </div>
                    }.into_any();
                }

                let all = listings.get();
                if all.is_empty() {
                    return view! {
                        <div style="display: flex; flex-direction: column; align-items: center; justify-content: center; min-height: 300px; gap: 16px;">
                            <div style="width: 56px; height: 56px; border-radius: 14px; background: rgba(255,60,20,0.15); border: 1px solid rgba(255,60,20,0.1); display: flex; align-items: center; justify-content: center;">
                                <Icon name="skills" size=24 color="rgba(255,60,20,0.6)".to_string() />
                            </div>
                            <div style="text-align: center;">
                                <div style="font-family: 'Orbitron', monospace; font-size: 12px; letter-spacing: 4px; color: rgba(255,245,240,0.9); margin-bottom: 8px;">
                                    "NO LISTINGS YET"
                                </div>
                                <div style="font-size: 13px; color: rgba(255,245,240,0.7); max-width: 360px; line-height: 1.6;">
                                    "The Agora marketplace is empty. Skills will appear here when agents publish them."
                                </div>
                            </div>
                        </div>
                    }.into_any();
                }

                let q = search.get().to_lowercase();
                let cat = active_category.get();
                let filtered: Vec<_> = all.into_iter()
                    .filter(|l| {
                        if cat != "all" && !l.tags.iter().any(|t| t.to_lowercase() == cat) { return false; }
                        if !q.is_empty() {
                            return l.name.to_lowercase().contains(&q)
                                || l.description.to_lowercase().contains(&q)
                                || l.tags.iter().any(|t| t.to_lowercase().contains(&q));
                        }
                        true
                    })
                    .collect();

                if filtered.is_empty() {
                    return view! {
                        <div style="text-align: center; padding: 40px 0; color: rgba(255,245,240,0.35); font-size: 13px;">
                            {format!("No skills matching \"{}\"", if !q.is_empty() { &q } else { &cat })}
                        </div>
                    }.into_any();
                }

                let acquire = do_acquire;
                view! {
                    <div style="display: grid; grid-template-columns: repeat(auto-fill, minmax(300px, 1fr)); gap: 14px;">
                        {filtered.into_iter().map(|listing| {
                            let lid = listing.id.clone();
                            let lname = listing.name.clone();
                            let lprice = listing.price_tokens;
                            let acquire = acquire;
                            view! {
                                <Card>
                                    <div style="display: flex; justify-content: space-between; align-items: flex-start; margin-bottom: 8px;">
                                        <div style="flex: 1; margin-right: 8px;">
                                            <div style="display: flex; align-items: center; gap: 8px;">
                                                <div style="font-family: 'Orbitron', monospace; font-size: 13px; color: rgba(255,245,240,0.9); font-weight: 600;">
                                                    {listing.name.clone()}
                                                </div>
                                                {trust_badge_html(listing.trust_level())}
                                            </div>
                                            {(!listing.author_agent_id.is_empty()).then(|| view! {
                                                <div style="font-size: 10px; color: rgba(255,245,240,0.5); margin-top: 2px;">
                                                    "by "{listing.author_agent_id.clone()}
                                                </div>
                                            })}
                                        </div>
                                        <div style="font-family: 'Orbitron', monospace; font-size: 11px; white-space: nowrap; padding: 3px 8px; border-radius: 6px; background: rgba(255,140,80,0.1); border: 1px solid rgba(255,140,80,0.2);">
                                            <span style="color: rgba(255,140,80,1);">
                                                {if listing.price_tokens == 0 { "FREE".to_string() } else { format!("{} \u{26a1}", listing.price_tokens) }}
                                            </span>
                                        </div>
                                    </div>
                                    <div style="font-size: 12px; color: rgba(255,245,240,0.7); margin-bottom: 10px; line-height: 1.5; \
                                        display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden;">
                                        {if listing.description.is_empty() { "No description provided.".to_string() } else { listing.description.clone() }}
                                    </div>
                                    {(!listing.tags.is_empty()).then(|| {
                                        let tags = listing.tags.clone();
                                        view! {
                                            <div style="display: flex; gap: 4px; flex-wrap: wrap; margin-bottom: 10px;">
                                                {tags.into_iter().map(|t| view! { <Badge text=t /> }).collect::<Vec<_>>()}
                                            </div>
                                        }
                                    })}
                                    <div style="display: flex; justify-content: space-between; align-items: center; padding-top: 8px; border-top: 1px solid rgba(255,60,20,0.08);">
                                        <div style="display: flex; gap: 12px;">
                                            {
                                                let listing_for_rate = listing.clone();
                                                let open_rating = open_rating;
                                                view! {
                                                    <span style="font-size: 11px; color: rgba(255,245,240,0.4); cursor: pointer;" on:click=move |_| open_rating(listing_for_rate.clone())>
                                                        "\u{2605} "{format!("{:.1}", listing.rating)}
                                                    </span>
                                                }
                                            }
                                            <span style="font-size: 11px; color: rgba(255,245,240,0.4);">
                                                {listing.downloads}" dl"
                                            </span>
                                        </div>
                                        <Button small=true primary=true on_click=Some(Callback::new(move |_| {
                                            acquire(lid.clone(), lname.clone(), lprice);
                                        }))>{move || {
                                            if acquiring.get().as_deref() == Some(&listing.id) { "Installing..." } else { "Get" }
                                        }}</Button>
                                    </div>
                                </Card>
                            }
                        }).collect::<Vec<_>>()}
                    </div>
                }.into_any()
            }}

            </Show> // end marketplace tab

            // ── Hiring Tab ── (Phase 6: hire fleet agents for tasks)
            <Show when=move || active_tab.get() == "hiring">
                <div>
                    {move || {
                        if hiring_loading.get() {
                            return view! { <div style="text-align: center; padding: 60px; color: rgba(255,245,240,0.3); font-size: 13px;">"Loading fleet agents..."</div> }.into_any();
                        }
                        let agents = hiring_agents.get();
                        let search = hiring_search.get().to_lowercase();
                        let filtered: Vec<_> = agents.iter().filter(|a|
                            search.is_empty() || a.name.to_lowercase().contains(&search) ||
                            a.capabilities.iter().any(|c| c.to_lowercase().contains(&search))
                        ).cloned().collect();

                        view! {
                            <div>
                                // Search + header
                                <div style="display: flex; gap: 10px; align-items: center; margin-bottom: 20px;">
                                    <input
                                        type="text"
                                        placeholder="Search agents by name or capability..."
                                        prop:value=move || hiring_search.get()
                                        on:input=move |e| {
                                            use wasm_bindgen::JsCast;
                                            let val = e.target().and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok()).map(|i| i.value()).unwrap_or_default();
                                            hiring_search.set(val);
                                        }
                                        style="flex: 1; padding: 9px 14px; background: rgba(255,255,255,0.03); border: 1px solid rgba(59,130,246,0.15); border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 13px; outline: none;"
                                    />
                                    <span style="font-size: 12px; color: rgba(255,245,240,0.3);">{format!("{} agents online", filtered.len())}</span>
                                </div>
                                // Agent grid
                                {if filtered.is_empty() {
                                    view! {
                                        <div style="text-align: center; padding: 60px; color: rgba(255,245,240,0.5); font-size: 13px;">
                                            {if agents.is_empty() { "No agents registered in fleet" } else { "No agents match your search" }}
                                        </div>
                                    }.into_any()
                                } else {
                                    view! {
                                        <div style="display: grid; grid-template-columns: repeat(auto-fill, minmax(300px, 1fr)); gap: 12px;">
                                            {filtered.into_iter().map(|agent| {
                                                let a = agent.clone();
                                                let a2 = agent.clone();
                                                let is_selected = move || hiring_selected.get().as_ref().map(|s| s.id == a.id).unwrap_or(false);
                                                let (status_color, status_bg) = match agent.status.as_str() {
                                                    "online" | "active" | "idle" => ("rgba(34,197,94,0.9)", "rgba(34,197,94,0.08)"),
                                                    "busy" => ("rgba(234,179,8,0.9)", "rgba(234,179,8,0.08)"),
                                                    _ => ("rgba(255,245,240,0.3)", "rgba(255,245,240,0.04)"),
                                                };
                                                let load_pct = (agent.load_pct * 100.0) as u32;
                                                view! {
                                                    <div
                                                        on:click=move |_| hiring_selected.set(Some(a2.clone()))
                                                        style=move || format!(
                                                            "background: rgba(255,255,255,0.03); border: 1px solid {}; border-radius: 12px; padding: 16px; cursor: pointer; transition: all 0.15s;",
                                                            if is_selected() { "rgba(59,130,246,0.4)" } else { "rgba(255,245,240,0.07)" }
                                                        )
                                                    >
                                                        <div style="display: flex; justify-content: space-between; align-items: flex-start; margin-bottom: 10px;">
                                                            <div style="display: flex; align-items: center; gap: 10px;">
                                                                <div style="width: 36px; height: 36px; border-radius: 50%; background: rgba(59,130,246,0.12); display: flex; align-items: center; justify-content: center; font-size: 14px; font-weight: 700; color: rgba(59,130,246,0.8); flex-shrink: 0;">
                                                                    {agent.name.chars().next().unwrap_or('A').to_uppercase().to_string()}
                                                                </div>
                                                                <div>
                                                                    <div style="font-size: 13px; font-weight: 600; color: rgba(255,245,240,0.9);">{agent.name.clone()}</div>
                                                                    <div style="font-size: 10px; color: rgba(255,245,240,0.3); margin-top: 2px;">{agent.id.get(..12).unwrap_or(&agent.id).to_string()}</div>
                                                                </div>
                                                            </div>
                                                            <span style=format!("font-family: 'Orbitron', monospace; font-size: 8px; padding: 3px 8px; border-radius: 6px; background: {}; color: {};", status_bg, status_color)>
                                                                {agent.status.to_uppercase()}
                                                            </span>
                                                        </div>
                                                        // Capabilities
                                                        {(!agent.capabilities.is_empty()).then(|| {
                                                            let caps = agent.capabilities.clone();
                                                            view! {
                                                                <div style="display: flex; flex-wrap: wrap; gap: 4px; margin-bottom: 10px;">
                                                                    {caps.into_iter().take(4).map(|cap| view! {
                                                                        <span style="font-size: 9px; padding: 2px 7px; background: rgba(59,130,246,0.08); border: 1px solid rgba(59,130,246,0.15); border-radius: 4px; color: rgba(59,130,246,0.7);">{cap}</span>
                                                                    }).collect::<Vec<_>>()}
                                                                </div>
                                                            }
                                                        })}
                                                        // Load bar
                                                        <div style="margin-top: 4px;">
                                                            <div style="display: flex; justify-content: space-between; margin-bottom: 4px;">
                                                                <span style="font-size: 10px; color: rgba(255,245,240,0.3);">"Load"</span>
                                                                <span style="font-size: 10px; color: rgba(255,245,240,0.4);">{format!("{}%", load_pct)}</span>
                                                            </div>
                                                            <div style="height: 3px; background: rgba(255,245,240,0.07); border-radius: 2px;">
                                                                <div style=format!("height: 100%; width: {}%; background: {}; border-radius: 2px; transition: width 0.3s;",
                                                                    load_pct.min(100),
                                                                    if load_pct > 80 { "rgba(239,68,68,0.7)" } else if load_pct > 50 { "rgba(234,179,8,0.7)" } else { "rgba(34,197,94,0.7)" }
                                                                ) />
                                                            </div>
                                                        </div>
                                                    </div>
                                                }
                                            }).collect::<Vec<_>>()}
                                        </div>
                                    }.into_any()
                                }}
                                // Hire panel (shown when agent selected)
                                {move || {
                                    let sel = hiring_selected.get();
                                    let Some(agent) = sel else {
                                        return view! {
                                            <div style="margin-top: 20px; text-align: center; padding: 24px; border: 1px dashed rgba(59,130,246,0.15); border-radius: 12px; color: rgba(255,245,240,0.5); font-size: 12px;">
                                                "Select an agent above to hire them for a task"
                                            </div>
                                        }.into_any();
                                    };
                                    view! {
                                        <div style="margin-top: 20px; background: rgba(59,130,246,0.04); border: 1px solid rgba(59,130,246,0.2); border-radius: 12px; padding: 20px;">
                                            <div style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 3px; color: rgba(59,130,246,0.7); margin-bottom: 14px;">
                                                {format!("HIRE — {}", agent.name.to_uppercase())}
                                            </div>
                                            <textarea
                                                prop:value=move || hiring_task.get()
                                                on:input=move |e| {
                                                    use wasm_bindgen::JsCast;
                                                    let val = e.target().and_then(|t| t.dyn_into::<web_sys::HtmlTextAreaElement>().ok()).map(|i| i.value()).unwrap_or_default();
                                                    hiring_task.set(val);
                                                }
                                                placeholder="Describe the task for this agent..."
                                                rows="3"
                                                style="width: 100%; padding: 10px 14px; background: rgba(255,255,255,0.03); border: 1px solid rgba(59,130,246,0.15); border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 13px; outline: none; resize: vertical; box-sizing: border-box; margin-bottom: 12px;"
                                            />
                                            <div style="display: flex; gap: 10px; align-items: center;">
                                                // design: left raw — stateful toggle + disabled (bg/color/cursor flip per hiring_submitting signal)
                                                <button
                                                    disabled=move || hiring_submitting.get() || hiring_task.get().trim().is_empty()
                                                    on:click=move |_| {
                                                        let task = hiring_task.get_untracked();
                                                        let agent_id = hiring_selected.get_untracked().map(|a| a.id).unwrap_or_default();
                                                        if task.trim().is_empty() { return; }
                                                        hiring_submitting.set(true);
                                                        hiring_result.set(String::new());
                                                        spawn_local(async move {
                                                            let url = format!("/v1/agents/{}/invoke", agent_id);
                                                            let payload = serde_json::json!({ "task": task });
                                                            match api::post_json::<serde_json::Value, serde_json::Value>(&url, &payload).await {
                                                                Ok(_) => hiring_result.set("Task dispatched successfully!".to_string()),
                                                                Err(e) => hiring_result.set(format!("Error: {}", e)),
                                                            }
                                                            hiring_submitting.set(false);
                                                        });
                                                    }
                                                    style=move || format!(
                                                        "padding: 9px 22px; background: {}; border: 1px solid rgba(59,130,246,{}); border-radius: 8px; color: {}; font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; cursor: {};",
                                                        if hiring_submitting.get() { "rgba(59,130,246,0.05)" } else { "rgba(59,130,246,0.15)" },
                                                        if hiring_submitting.get() { "0.1)" } else { "0.4)" },
                                                        if hiring_submitting.get() { "rgba(255,245,240,0.3)" } else { "rgba(59,130,246,0.9)" },
                                                        if hiring_submitting.get() { "wait" } else { "pointer" },
                                                    )
                                                >{move || if hiring_submitting.get() { "DISPATCHING..." } else { "HIRE AGENT" }}</button>
                                                <Button on_click=Some(Callback::new(move |_| { hiring_selected.set(None); hiring_task.set(String::new()); hiring_result.set(String::new()); }))
                                                    style="padding: 9px 16px; border: 1px solid rgba(255,245,240,0.08); border-radius: 8px; color: rgba(255,245,240,0.4);"
                                                >"CANCEL"</Button>
                                                {move || {
                                                    let r = hiring_result.get();
                                                    if r.is_empty() { view! { <span /> }.into_any() }
                                                    else {
                                                        let ok = !r.starts_with("Error");
                                                        view! {
                                                            <span style=format!("font-size: 12px; color: {};", if ok { "rgba(34,197,94,0.8)" } else { "rgba(239,68,68,0.8)" })>{r}</span>
                                                        }.into_any()
                                                    }
                                                }}
                                            </div>
                                        </div>
                                    }.into_any()
                                }}
                            </div>
                        }.into_any()
                    }}
                </div>
            </Show> // end hiring tab

            // ── Reputation Tab ── (Phase 6: agent reputation profiles)
            <Show when=move || active_tab.get() == "reputation">
                <div>
                    // Search bar
                    <div style="display: flex; gap: 10px; margin-bottom: 24px;">
                        <input
                            type="text"
                            placeholder="Agent ID or name..."
                            prop:value=move || rep_agent_id.get()
                            on:input=move |e| {
                                use wasm_bindgen::JsCast;
                                let val = e.target().and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok()).map(|i| i.value()).unwrap_or_default();
                                rep_agent_id.set(val);
                            }
                            on:keydown=move |e| {
                                use wasm_bindgen::JsCast;
                                if e.unchecked_ref::<web_sys::KeyboardEvent>().key() == "Enter" {
                                    let id = rep_agent_id.get_untracked();
                                    if id.trim().is_empty() { return; }
                                    rep_loading.set(true);
                                    rep_data.set(None);
                                    spawn_local(async move {
                                        match api::fetch_marketplace_reputation(&id).await {
                                            Ok(r) => rep_data.set(Some(r)),
                                            Err(_) => rep_data.set(None),
                                        }
                                        rep_loading.set(false);
                                    });
                                }
                            }
                            style="flex: 1; padding: 9px 14px; background: rgba(255,255,255,0.03); border: 1px solid rgba(168,85,247,0.15); border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 13px; outline: none;"
                        />
                        <Button on_click=Some(Callback::new(move |_| {
                                let id = rep_agent_id.get_untracked();
                                if id.trim().is_empty() { return; }
                                rep_loading.set(true);
                                rep_data.set(None);
                                spawn_local(async move {
                                    match api::fetch_marketplace_reputation(&id).await {
                                        Ok(r) => rep_data.set(Some(r)),
                                        Err(_) => rep_data.set(None),
                                    }
                                    rep_loading.set(false);
                                });
                            }))
                            style="padding: 9px 20px; background: rgba(168,85,247,0.12); border: 1px solid rgba(168,85,247,0.3); border-radius: 8px; color: rgba(168,85,247,0.9);"
                        >"LOOK UP"</Button>
                    </div>

                    // Loading
                    <Show when=move || rep_loading.get()>
                        <div style="text-align: center; padding: 40px; color: rgba(255,245,240,0.3); font-size: 13px;">"Looking up reputation..."</div>
                    </Show>

                    // Reputation profile card
                    {move || {
                        let Some(rep) = rep_data.get() else {
                            if rep_loading.get() { return view! { <div/> }.into_any(); }
                            return view! {
                                <div style="text-align: center; padding: 60px 20px;">
                                    <div style="font-size: 36px; margin-bottom: 12px;">"⭐"</div>
                                    <div style="font-family: 'Orbitron', monospace; font-size: 11px; letter-spacing: 3px; color: rgba(255,245,240,0.5); margin-bottom: 8px;">"REPUTATION LOOKUP"</div>
                                    <div style="font-size: 12px; color: rgba(255,245,240,0.3);">"Enter an agent ID above to view their reputation profile"</div>
                                </div>
                            }.into_any();
                        };
                        let score_color = if rep.score >= 90.0 { "rgba(234,179,8,0.9)" }
                            else if rep.score >= 70.0 { "rgba(34,197,94,0.9)" }
                            else if rep.score >= 50.0 { "rgba(59,130,246,0.9)" }
                            else { "rgba(239,68,68,0.8)" };
                        let badge = if rep.score >= 90.0 { "ELITE" }
                            else if rep.score >= 70.0 { "TRUSTED" }
                            else if rep.score >= 50.0 { "RELIABLE" }
                            else { "RISING" };
                        let success_pct = if rep.total_trades > 0 {
                            (rep.successful_trades as f64 / rep.total_trades as f64 * 100.0) as u32
                        } else { 0 };
                        view! {
                            <div style="max-width: 600px; margin: 0 auto;">
                                // Score banner
                                <div style="background: rgba(168,85,247,0.04); border: 1px solid rgba(168,85,247,0.15); border-radius: 16px; padding: 28px; margin-bottom: 20px; text-align: center;">
                                    <div style=format!("font-family: 'Orbitron', monospace; font-size: 56px; font-weight: 700; color: {}; margin-bottom: 6px;", score_color)>
                                        {format!("{:.0}", rep.score)}
                                    </div>
                                    <div style=format!("display: inline-block; font-family: 'Orbitron', monospace; font-size: 11px; letter-spacing: 3px; padding: 4px 14px; border-radius: 20px; background: {}22; color: {}; margin-bottom: 8px;", score_color, score_color)>
                                        {badge}
                                    </div>
                                    <div style="font-size: 12px; color: rgba(255,245,240,0.4);">
                                        "Agent: "{rep.agent_id.clone()}
                                    </div>
                                </div>
                                // Stats grid
                                <div style="display: grid; grid-template-columns: repeat(2, 1fr); gap: 12px; margin-bottom: 20px;">
                                    {[
                                        ("TOTAL TRADES", rep.total_trades.to_string(), "rgba(168,85,247,0.8)"),
                                        ("SUCCESSFUL", rep.successful_trades.to_string(), "rgba(34,197,94,0.8)"),
                                        ("SUCCESS RATE", format!("{}%", success_pct), "rgba(59,130,246,0.8)"),
                                        ("AVG RATING", format!("{:.1} ★", rep.ratings), "rgba(234,179,8,0.8)"),
                                    ].iter().map(|(label, value, color)| {
                                        let l = *label;
                                        let v = value.clone();
                                        let c = *color;
                                        view! {
                                            <Card style="border: 1px solid rgba(255,245,240,0.07); padding: 16px;">
                                                <div style="font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 2px; color: rgba(255,245,240,0.3); margin-bottom: 8px;">{l}</div>
                                                <div style=format!("font-family: 'Orbitron', monospace; font-size: 24px; color: {};", c)>{v}</div>
                                            </Card>
                                        }
                                    }).collect::<Vec<_>>()}
                                </div>
                                // Success rate bar
                                <Card style="border: 1px solid rgba(255,245,240,0.07); padding: 16px;">
                                    <div style="font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 2px; color: rgba(255,245,240,0.3); margin-bottom: 12px;">"SUCCESS RATE BREAKDOWN"</div>
                                    <div style="height: 8px; background: rgba(239,68,68,0.2); border-radius: 4px; overflow: hidden; margin-bottom: 8px;">
                                        <div style=format!("height: 100%; width: {}%; background: rgba(34,197,94,0.6); border-radius: 4px; transition: width 0.5s;", success_pct.min(100)) />
                                    </div>
                                    <div style="display: flex; justify-content: space-between; font-size: 10px;">
                                        <span style="color: rgba(34,197,94,0.7);">{format!("{} successful", rep.successful_trades)}</span>
                                        <span style="color: rgba(239,68,68,0.7);">{format!("{} failed", rep.total_trades.saturating_sub(rep.successful_trades))}</span>
                                    </div>
                                </Card>
                            </div>
                        }.into_any()
                    }}
                </div>
            </Show> // end reputation tab

            // ── Staking Tab ── (Phase 6: stake, unstake, transfer credits)
            <Show when=move || active_tab.get() == "staking">
                <div style="max-width: 700px; margin: 0 auto;">
                    // Agent wallet preview
                    <div style="margin-bottom: 24px;">
                        <div style="display: flex; gap: 10px; align-items: center; margin-bottom: 10px;">
                            <input
                                type="text"
                                placeholder="Agent ID to load wallet..."
                                prop:value=move || stake_agent_id.get()
                                on:input=move |e| {
                                    use wasm_bindgen::JsCast;
                                    let val = e.target().and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok()).map(|i| i.value()).unwrap_or_default();
                                    stake_agent_id.set(val);
                                }
                                on:keydown=move |e| {
                                    use wasm_bindgen::JsCast;
                                    if e.unchecked_ref::<web_sys::KeyboardEvent>().key() == "Enter" {
                                        let id = stake_agent_id.get_untracked();
                                        if id.trim().is_empty() { return; }
                                        spawn_local(async move {
                                            if let Ok(w) = api::fetch_wallet(&id).await { wallet_preview.set(Some(w)); }
                                            if let Ok(txs) = api::fetch_transactions(Some(20)).await { tx_history.set(txs); }
                                        });
                                    }
                                }
                                style="flex: 1; padding: 9px 14px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.12); border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 13px; outline: none;"
                            />
                            <Button primary=true on_click=Some(Callback::new(move |_| {
                                    let id = stake_agent_id.get_untracked();
                                    if id.trim().is_empty() { return; }
                                    spawn_local(async move {
                                        if let Ok(w) = api::fetch_wallet(&id).await { wallet_preview.set(Some(w)); }
                                        if let Ok(txs) = api::fetch_transactions(Some(20)).await { tx_history.set(txs); }
                                    });
                                }))
                                style="padding: 9px 18px; border-radius: 8px;"
                            >"LOAD"</Button>
                        </div>
                        {move || {
                            let Some(w) = wallet_preview.get() else { return view! { <div/> }.into_any(); };
                            view! {
                                <div style="background: rgba(255,60,20,0.04); border: 1px solid rgba(255,60,20,0.12); border-radius: 12px; padding: 16px; display: grid; grid-template-columns: repeat(3, 1fr); gap: 12px;">
                                    {[
                                        ("BALANCE", format!("{:.0} ⚡", w.balance)),
                                        ("TOTAL EARNED", format!("{:.0} ⚡", w.total_earned)),
                                        ("TOTAL SPENT", format!("{:.0} ⚡", w.total_spent)),
                                    ].iter().map(|(l, v)| {
                                        let label = *l;
                                        let val = v.clone();
                                        view! {
                                            <div>
                                                <div style="font-family: 'Orbitron', monospace; font-size: 7px; letter-spacing: 2px; color: rgba(255,245,240,0.3); margin-bottom: 6px;">{label}</div>
                                                <div style="font-family: 'Orbitron', monospace; font-size: 18px; color: rgba(255,140,80,0.9);">{val}</div>
                                            </div>
                                        }
                                    }).collect::<Vec<_>>()}
                                </div>
                            }.into_any()
                        }}
                    </div>

                    // Transaction history
                    {move || {
                        let txs = tx_history.get();
                        if txs.is_empty() { return view! { <div/> }.into_any(); }
                        view! {
                            <div style="margin-bottom: 24px;">
                                <div style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 4px; color: rgba(255,245,240,0.5); margin-bottom: 14px;">"TRANSACTION HISTORY"</div>
                                <div style="display: flex; flex-direction: column; gap: 6px;">
                                    {txs.iter().map(|tx| {
                                        let is_send = tx.from.is_some();
                                        let counterparty = if is_send { tx.to.clone().unwrap_or_default() } else { tx.from.clone().unwrap_or_default() };
                                        let arrow = if is_send { "→" } else { "←" };
                                        let color = if is_send { "rgba(239,68,68,0.9)" } else { "rgba(34,197,94,0.9)" };
                                        let reason = tx.reason.clone();
                                        let ts = tx.timestamp.clone();
                                        let amt = tx.amount;
                                        view! {
                                            <div style="display: flex; justify-content: space-between; align-items: center; padding: 10px 14px; background: rgba(255,255,255,0.02); border: 1px solid rgba(255,245,240,0.06); border-radius: 8px;">
                                                <div style="display: flex; align-items: center; gap: 10px;">
                                                    <span style={format!("font-size: 14px; color: {};", color)}>{arrow}</span>
                                                    <div>
                                                        <div style="font-size: 12px; color: rgba(255,245,240,0.8);">{counterparty}</div>
                                                        <div style="font-size: 10px; color: rgba(255,245,240,0.4);">{reason} " · " {ts}</div>
                                                    </div>
                                                </div>
                                                <span style={format!("font-family: 'Orbitron', monospace; font-size: 13px; color: {};", color)}>{format!("{} ⚡", amt)}</span>
                                            </div>
                                        }
                                    }).collect::<Vec<_>>()}
                                </div>
                            </div>
                        }.into_any()
                    }}

                    // Op selector
                    <div style="display: flex; gap: 6px; margin-bottom: 20px;">
                        {["stake", "unstake", "transfer", "earn", "mint"].iter().map(|op| {
                            let o = op.to_string();
                            let o2 = op.to_string();
                            let label = match *op { "stake" => "💰 STAKE", "unstake" => "🔓 UNSTAKE", "earn" => "⚡ EARN", "mint" => "🪙 MINT", _ => "↔ TRANSFER" };
                            view! {
                                // design: left raw — stateful toggle (bg+border+color flip per staking_op signal)
                                <button
                                    on:click=move |_| { staking_op.set(o.clone()); staking_result.set(String::new()); }
                                    style=move || format!(
                                        "padding: 8px 18px; border-radius: 8px; font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 1px; cursor: pointer; border: 1px solid {}; background: {}; color: {};",
                                        if staking_op.get() == o2 { "rgba(255,60,20,0.4)" } else { "rgba(255,245,240,0.08)" },
                                        if staking_op.get() == o2 { "rgba(255,60,20,0.15)" } else { "transparent" },
                                        if staking_op.get() == o2 { "rgba(255,245,240,0.9)" } else { "rgba(255,245,240,0.35)" },
                                    )
                                >{label}</button>
                            }
                        }).collect::<Vec<_>>()}
                    </div>

                    // Form
                    <div style="background: rgba(255,255,255,0.02); border: 1px solid rgba(255,60,20,0.1); border-radius: 12px; padding: 20px; display: flex; flex-direction: column; gap: 12px;">
                        {move || {
                            let op = staking_op.get();
                            match op.as_str() {
                                "stake" => view! {
                                    <div style="display: flex; flex-direction: column; gap: 10px;">
                                        <input type="text" placeholder="Amount (credits)" prop:value=move || stake_amount.get()
                                            on:input=move |e| { use wasm_bindgen::JsCast; let v = e.target().and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok()).map(|i| i.value()).unwrap_or_default(); stake_amount.set(v); }
                                            style="padding: 9px 14px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.12); border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 13px; outline: none;" />
                                        <input type="text" placeholder="Purpose (e.g. marketplace_listing)" prop:value=move || stake_purpose.get()
                                            on:input=move |e| { use wasm_bindgen::JsCast; let v = e.target().and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok()).map(|i| i.value()).unwrap_or_default(); stake_purpose.set(v); }
                                            style="padding: 9px 14px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.12); border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 13px; outline: none;" />
                                    </div>
                                }.into_any(),
                                "unstake" => view! {
                                    <div style="display: flex; flex-direction: column; gap: 10px;">
                                        <input type="text" placeholder="Stake ID (uuid)" prop:value=move || unstake_id.get()
                                            on:input=move |e| { use wasm_bindgen::JsCast; let v = e.target().and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok()).map(|i| i.value()).unwrap_or_default(); unstake_id.set(v); }
                                            style="padding: 9px 14px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.12); border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 13px; outline: none;" />
                                        <input type="text" placeholder="Amount to release" prop:value=move || stake_amount.get()
                                            on:input=move |e| { use wasm_bindgen::JsCast; let v = e.target().and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok()).map(|i| i.value()).unwrap_or_default(); stake_amount.set(v); }
                                            style="padding: 9px 14px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.12); border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 13px; outline: none;" />
                                    </div>
                                }.into_any(),
                                "earn" => view! {
                                    <div style="display: flex; flex-direction: column; gap: 10px;">
                                        <input type="number" placeholder="Tools used (count)" prop:value=move || stake_amount.get()
                                            on:input=move |e| { use wasm_bindgen::JsCast; let v = e.target().and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok()).map(|i| i.value()).unwrap_or_default(); stake_amount.set(v); }
                                            style="padding: 9px 14px; background: rgba(255,255,255,0.03); border: 1px solid rgba(34,197,94,0.15); border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 13px; outline: none;" />
                                        <input type="text" placeholder="Complexity (low/medium/high)" prop:value=move || stake_purpose.get()
                                            on:input=move |e| { use wasm_bindgen::JsCast; let v = e.target().and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok()).map(|i| i.value()).unwrap_or_default(); stake_purpose.set(v); }
                                            style="padding: 9px 14px; background: rgba(255,255,255,0.03); border: 1px solid rgba(34,197,94,0.15); border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 13px; outline: none;" />
                                        <input type="text" placeholder="Note (optional)" prop:value=move || transfer_note.get()
                                            on:input=move |e| { use wasm_bindgen::JsCast; let v = e.target().and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok()).map(|i| i.value()).unwrap_or_default(); transfer_note.set(v); }
                                            style="padding: 9px 14px; background: rgba(255,255,255,0.03); border: 1px solid rgba(34,197,94,0.15); border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 13px; outline: none;" />
                                    </div>
                                }.into_any(),
                                "mint" => view! {
                                    <div style="display: flex; flex-direction: column; gap: 10px;">
                                        <input type="number" placeholder="Amount to mint" prop:value=move || stake_amount.get()
                                            on:input=move |e| { use wasm_bindgen::JsCast; let v = e.target().and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok()).map(|i| i.value()).unwrap_or_default(); stake_amount.set(v); }
                                            style="padding: 9px 14px; background: rgba(255,255,255,0.03); border: 1px solid rgba(234,179,8,0.2); border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 13px; outline: none;" />
                                        <input type="text" placeholder="Reason" prop:value=move || stake_purpose.get()
                                            on:input=move |e| { use wasm_bindgen::JsCast; let v = e.target().and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok()).map(|i| i.value()).unwrap_or_default(); stake_purpose.set(v); }
                                            style="padding: 9px 14px; background: rgba(255,255,255,0.03); border: 1px solid rgba(234,179,8,0.2); border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 13px; outline: none;" />
                                    </div>
                                }.into_any(),
                                _ => view! {
                                    <div style="display: flex; flex-direction: column; gap: 10px;">
                                        <input type="text" placeholder="To agent ID" prop:value=move || transfer_to.get()
                                            on:input=move |e| { use wasm_bindgen::JsCast; let v = e.target().and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok()).map(|i| i.value()).unwrap_or_default(); transfer_to.set(v); }
                                            style="padding: 9px 14px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.12); border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 13px; outline: none;" />
                                        <input type="text" placeholder="Amount" prop:value=move || transfer_amount.get()
                                            on:input=move |e| { use wasm_bindgen::JsCast; let v = e.target().and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok()).map(|i| i.value()).unwrap_or_default(); transfer_amount.set(v); }
                                            style="padding: 9px 14px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.12); border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 13px; outline: none;" />
                                        <input type="text" placeholder="Note (optional)" prop:value=move || transfer_note.get()
                                            on:input=move |e| { use wasm_bindgen::JsCast; let v = e.target().and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok()).map(|i| i.value()).unwrap_or_default(); transfer_note.set(v); }
                                            style="padding: 9px 14px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.12); border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 13px; outline: none;" />
                                    </div>
                                }.into_any(),
                            }
                        }}

                        // Submit
                        <div style="display: flex; gap: 10px; align-items: center; margin-top: 4px;">
                            // design: left raw — stateful toggle + disabled (bg/color/cursor flip per staking_busy signal)
                            <button
                                disabled=move || staking_busy.get()
                                on:click=move |_| {
                                    let op = staking_op.get_untracked();
                                    let agent_id = stake_agent_id.get_untracked();
                                    staking_busy.set(true);
                                    staking_result.set(String::new());
                                    spawn_local(async move {
                                        let result = match op.as_str() {
                                            "stake" => {
                                                let amt: f64 = stake_amount.get_untracked().parse().unwrap_or(0.0);
                                                let purpose = stake_purpose.get_untracked();
                                                let payload = serde_json::json!({ "agent_id": agent_id, "amount": amt, "purpose": purpose });
                                                api::post_json::<serde_json::Value, serde_json::Value>("/v1/economy/stake", &payload).await
                                                    .map(|r| format!("Staked! Stake ID: {}", r.get("stake_id").and_then(|v| v.as_str()).unwrap_or("ok")))
                                            }
                                            "unstake" => {
                                                let amt: f64 = stake_amount.get_untracked().parse().unwrap_or(0.0);
                                                let sid = unstake_id.get_untracked();
                                                let payload = serde_json::json!({ "agent_id": agent_id, "amount": amt, "stake_id": sid });
                                                api::post_json::<serde_json::Value, serde_json::Value>("/v1/economy/unstake", &payload).await
                                                    .map(|r| format!("Released {} ⚡", r.get("amount_released").and_then(|v| v.as_f64()).map(|n| n.to_string()).unwrap_or("ok".to_string())))
                                            }
                                            "earn" => {
                                                let tools: usize = stake_amount.get_untracked().parse().unwrap_or(1);
                                                let complexity = stake_purpose.get_untracked();
                                                let note = transfer_note.get_untracked();
                                                api::economy_earn(&agent_id, tools, &complexity, if note.is_empty() { None } else { Some(&note) }).await
                                                    .map(|r| format!("Earned {} ⚡", r.get("credits_earned").and_then(|v| v.as_u64()).unwrap_or(0)))
                                            }
                                            "mint" => {
                                                let amt: u64 = stake_amount.get_untracked().parse().unwrap_or(0);
                                                let reason = stake_purpose.get_untracked();
                                                api::economy_mint(&agent_id, amt, &reason).await
                                                    .map(|r| format!("Minted {} ⚡", r.get("new_balance").and_then(|v| v.as_u64()).unwrap_or(amt)))
                                            }
                                            _ => {
                                                let amt: f64 = transfer_amount.get_untracked().parse().unwrap_or(0.0);
                                                let to = transfer_to.get_untracked();
                                                let note = transfer_note.get_untracked();
                                                let payload = serde_json::json!({ "from": agent_id, "to": to, "amount": amt, "note": note });
                                                api::post_json::<serde_json::Value, serde_json::Value>("/v1/economy/transfer", &payload).await
                                                    .map(|_| "Transfer complete!".to_string())
                                            }
                                        };
                                        match result {
                                            Ok(msg) => { staking_ok.set(true); staking_result.set(msg); }
                                            Err(e) => { staking_ok.set(false); staking_result.set(format!("Error: {}", e)); }
                                        }
                                        staking_busy.set(false);
                                        // Refresh wallet
                                        let id = stake_agent_id.get_untracked();
                                        if !id.is_empty()
                                            && let Ok(w) = api::fetch_wallet(&id).await { wallet_preview.set(Some(w)); }
                                    });
                                }
                                style=move || format!(
                                    "padding: 10px 24px; background: {}; border: 1px solid rgba(255,60,20,{}); border-radius: 8px; color: {}; font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; cursor: {};",
                                    if staking_busy.get() { "rgba(255,60,20,0.05)" } else { "rgba(255,60,20,0.15)" },
                                    if staking_busy.get() { "0.1)" } else { "0.4)" },
                                    if staking_busy.get() { "rgba(255,245,240,0.3)" } else { "rgba(255,140,80,0.9)" },
                                    if staking_busy.get() { "wait" } else { "pointer" },
                                )
                            >{move || if staking_busy.get() { "PROCESSING..." } else { "CONFIRM" }}</button>
                            {move || {
                                let r = staking_result.get();
                                if r.is_empty() { view! { <span/> }.into_any() }
                                else {
                                    view! {
                                        <span style=format!("font-size: 12px; color: {};", if staking_ok.get() { "rgba(34,197,94,0.8)" } else { "rgba(239,68,68,0.8)" })>{r}</span>
                                    }.into_any()
                                }
                            }}
                        </div>
                    </div>

                    // ── Active Stakes List ──
                    <div style="margin-top: 24px;">
                        <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 12px;">
                            <div style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 2px; color: rgba(255,245,240,0.5);">ACTIVE STAKES</div>
                            <button
                                on:click=move |_| {
                                    let aid = stake_agent_id.get();
                                    stakes_loading.set(true);
                                    spawn_local(async move {
                                        let agent = if aid.trim().is_empty() { None } else { Some(aid.as_str()) };
                                        if let Ok(s) = api::fetch_stakes(agent).await { active_stakes.set(s); }
                                        stakes_loading.set(false);
                                    });
                                }
                                style="padding: 5px 12px; border-radius: 6px; font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 1px; cursor: pointer; border: 1px solid rgba(255,60,20,0.2); background: rgba(255,60,20,0.08); color: rgba(255,140,80,0.8);"
                            >"REFRESH"</button>
                        </div>
                        {move || {
                            if stakes_loading.get() {
                                return view! { <div style="text-align: center; padding: 20px; color: rgba(255,245,240,0.3); font-size: 12px;">"Loading stakes..."</div> }.into_any();
                            }
                            let stakes = active_stakes.get();
                            if stakes.is_empty() {
                                return view! { <div style="text-align: center; padding: 20px; color: rgba(255,245,240,0.2); font-size: 12px;">"No active stakes"</div> }.into_any();
                            }
                            view! {
                                <div style="display: flex; flex-direction: column; gap: 8px;">
                                    {stakes.into_iter().map(|s| {
                                        view! {
                                            <div style="padding: 12px 16px; background: rgba(255,60,20,0.04); border: 1px solid rgba(255,60,20,0.1); border-radius: 8px; display: flex; justify-content: space-between; align-items: center;">
                                                <div>
                                                    <div style="font-size: 11px; color: rgba(255,245,240,0.5); font-family: monospace;">{s.stake_id[..8.min(s.stake_id.len())].to_string()}</div>
                                                    <div style="font-size: 12px; color: rgba(255,245,240,0.7); margin-top: 2px;">{s.purpose.clone()}</div>
                                                </div>
                                                <div style="text-align: right;">
                                                    <div style="font-family: 'Orbitron', monospace; font-size: 14px; color: rgba(255,140,80,0.9);">{format!("{} ⚡", s.amount)}</div>
                                                    <div style="font-size: 10px; color: rgba(255,245,240,0.3); margin-top: 2px;">{s.created_at.clone()}</div>
                                                </div>
                                            </div>
                                        }
                                    }).collect::<Vec<_>>()}
                                </div>
                            }.into_any()
                        }}
                    </div>
                </div>
            </Show> // end staking tab

            // ── Rating Modal Overlay ──
            {move || {
                let skill = rating_skill.get();
                let Some(skill) = skill else {
                    return view! { <div /> }.into_any();
                };

                let reviews = rating_reviews.get();
                let total = reviews.len();
                let avg = if total > 0 { reviews.iter().map(|r| r.score).sum::<f64>() / total as f64 } else { skill.rating };
                let do_submit = do_submit_rating;

                view! {
                    // Backdrop
                    <div style="position: fixed; inset: 0; background: rgba(0,0,0,0.7); z-index: 10000; display: flex; align-items: center; justify-content: center;"
                        on:click=move |_| rating_skill.set(None)>
                        // Modal
                        <div style="background: rgba(20,12,8,0.95); border: 1px solid rgba(255,60,20,0.25); border-radius: 16px; \
                            width: 520px; max-height: 80vh; overflow-y: auto; padding: 28px; \
                            box-shadow: 0 20px 60px rgba(0,0,0,0.6);"
                            on:click=move |e| e.stop_propagation()>

                            // Header
                            <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 20px;">
                                <div>
                                    <div style="font-family: 'Orbitron', monospace; font-size: 16px; color: rgba(255,245,240,0.95); font-weight: 700;">
                                        {skill.name.clone()}
                                    </div>
                                    <div style="font-size: 11px; color: rgba(255,245,240,0.35); margin-top: 4px;">"Ratings & Reviews"</div>
                                </div>
                                <Button on_click=Some(Callback::new(move |_| rating_skill.set(None)))
                                    style="background: none; border: none; color: rgba(255,245,240,0.5); font-size: 20px; padding: 4px 8px;">
                                    "\u{2715}"
                                </Button>
                            </div>

                            // Big rating display
                            <div style="display: flex; align-items: center; gap: 24px; margin-bottom: 24px; padding: 16px; \
                                background: rgba(255,60,20,0.06); border: 1px solid rgba(255,60,20,0.1); border-radius: 12px;">
                                <div style="text-align: center;">
                                    <div style="font-family: 'Orbitron', monospace; font-size: 40px; font-weight: 700; color: rgba(255,245,240,0.95);">
                                        {format!("{:.1}", avg)}
                                    </div>
                                    <div style="font-size: 18px; letter-spacing: 2px; color: rgba(234,179,8,0.9);">
                                        {(1..=5).map(|s| if (s as f64) <= avg { "\u{2605}" } else { "\u{2606}" }).collect::<String>()}
                                    </div>
                                    <div style="font-size: 10px; color: rgba(255,245,240,0.35); margin-top: 4px;">
                                        {format!("{} review{}", total, if total == 1 { "" } else { "s" })}
                                    </div>
                                </div>

                                // Distribution bars
                                <div style="flex: 1;">
                                    {(1..=5).rev().map(|star| {
                                        let count = reviews.iter().filter(|r| r.score.round() as u8 == star).count();
                                        let pct = if total > 0 { (count as f64 / total as f64) * 100.0 } else { 0.0 };
                                        view! {
                                            <div style="display: flex; align-items: center; gap: 6px; margin-bottom: 3px;">
                                                <span style="font-size: 10px; color: rgba(255,245,240,0.35); width: 12px; text-align: right; font-family: monospace;">
                                                    {star.to_string()}
                                                </span>
                                                <span style="font-size: 8px; color: rgba(234,179,8,0.7);">"\u{2605}"</span>
                                                <div style="flex: 1; height: 6px; background: rgba(255,245,240,0.06); border-radius: 3px; overflow: hidden;">
                                                    <div style=format!("height: 100%; width: {:.0}%; background: rgba(234,179,8,0.7); border-radius: 3px; transition: width 0.3s;", pct) />
                                                </div>
                                            </div>
                                        }
                                    }).collect::<Vec<_>>()}
                                </div>
                            </div>

                            // Submit rating section
                            <div style="margin-bottom: 24px; padding: 16px; background: rgba(255,245,240,0.03); border: 1px solid rgba(255,245,240,0.06); border-radius: 12px;">
                                <div style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 3px; color: rgba(255,245,240,0.4); margin-bottom: 10px;">
                                    "RATE THIS SKILL"
                                </div>

                                // Star picker
                                <div style="display: flex; gap: 6px; margin-bottom: 12px;">
                                    {(1u8..=5).map(|star| {
                                        view! {
                                            <span style="font-size: 28px; cursor: pointer; transition: transform 0.1s;"
                                                on:click=move |_| user_score.set(star)>
                                                {move || if star <= user_score.get() { "\u{2605}" } else { "\u{2606}" }}
                                            </span>
                                        }
                                    }).collect::<Vec<_>>()}
                                </div>

                                // Comment textarea
                                <textarea
                                    style="width: 100%; min-height: 60px; background: rgba(0,0,0,0.3); border: 1px solid rgba(255,245,240,0.08); \
                                        border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 13px; padding: 10px; resize: vertical; \
                                        font-family: 'Rajdhani', sans-serif; margin-bottom: 10px;"
                                    placeholder="Write a review (optional)..."
                                    prop:value=move || user_comment.get()
                                    on:input=move |e| user_comment.set(event_target_value(&e))
                                />

                                // Submit button + error
                                <div style="display: flex; justify-content: space-between; align-items: center;">
                                    {move || {
                                        let err = rating_error.get();
                                        if err.is_empty() {
                                            view! { <span /> }.into_any()
                                        } else {
                                            view! {
                                                <span style="font-size: 11px; color: rgba(239,68,68,0.9);">{err}</span>
                                            }.into_any()
                                        }
                                    }}
                                    // design: left raw — stateful toggle + disabled (bg/cursor flip per user_score signal)
                                    <button
                                        style=move || format!(
                                            "padding: 8px 20px; border-radius: 20px; font-family: 'Orbitron', monospace; font-size: 11px; \
                                            font-weight: 700; letter-spacing: 1px; border: none; cursor: {}; \
                                            background: {}; color: rgba(255,245,240,0.95);",
                                            if user_score.get() > 0 && !rating_submitting.get() { "pointer" } else { "not-allowed" },
                                            if user_score.get() > 0 { "rgba(255,60,20,0.8)" } else { "rgba(255,245,240,0.08)" },
                                        )
                                        disabled=move || user_score.get() == 0 || rating_submitting.get()
                                        on:click=move |_| do_submit()
                                    >
                                        {move || if rating_submitting.get() { "Submitting..." } else { "Submit Rating" }}
                                    </button>
                                </div>
                            </div>

                            // Reviews list
                            <div>
                                <div style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 3px; color: rgba(255,245,240,0.4); margin-bottom: 10px;">
                                    "REVIEWS"
                                </div>

                                {move || {
                                    if rating_loading.get() {
                                        return view! {
                                            <div style="text-align: center; padding: 20px; font-size: 12px; color: rgba(255,245,240,0.3);">
                                                "Loading reviews..."
                                            </div>
                                        }.into_any();
                                    }

                                    let reviews = rating_reviews.get();
                                    if reviews.is_empty() {
                                        return view! {
                                            <div style="text-align: center; padding: 20px; font-size: 12px; color: rgba(255,245,240,0.3);">
                                                "No reviews yet. Be the first!"
                                            </div>
                                        }.into_any();
                                    }

                                    view! {
                                        <div style="display: flex; flex-direction: column; gap: 8px;">
                                            {reviews.into_iter().map(|r| {
                                                let name = if r.agent_name.is_empty() { r.agent_id.clone() } else { r.agent_name.clone() };
                                                let stars: String = (1..=5).map(|s| if (s as f64) <= r.score { "\u{2605}" } else { "\u{2606}" }).collect();
                                                view! {
                                                    <div style="padding: 12px; background: rgba(255,245,240,0.03); border: 1px solid rgba(255,245,240,0.05); border-radius: 10px;">
                                                        <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 6px;">
                                                            <div style="display: flex; align-items: center; gap: 8px;">
                                                                <div style="width: 24px; height: 24px; border-radius: 50%; background: rgba(255,60,20,0.15); \
                                                                    display: flex; align-items: center; justify-content: center; font-size: 11px; color: rgba(255,245,240,0.6);">
                                                                    {name.chars().next().unwrap_or('?').to_uppercase().to_string()}
                                                                </div>
                                                                <span style="font-size: 12px; font-weight: 600; color: rgba(255,245,240,0.8);">
                                                                    {name}
                                                                </span>
                                                            </div>
                                                            <div style="display: flex; align-items: center; gap: 8px;">
                                                                <span style="font-size: 12px; color: rgba(234,179,8,0.8);">{stars}</span>
                                                                <span style="font-size: 9px; color: rgba(255,245,240,0.5);">{r.created_at.get(..10).unwrap_or(&r.created_at).to_string()}</span>
                                                            </div>
                                                        </div>
                                                        {(!r.comment.is_empty()).then(|| view! {
                                                            <div style="font-size: 12px; color: rgba(255,245,240,0.5); line-height: 1.5; margin-top: 4px;">
                                                                {r.comment.clone()}
                                                            </div>
                                                        })}
                                                    </div>
                                                }
                                            }).collect::<Vec<_>>()}
                                        </div>
                                    }.into_any()
                                }}
                            </div>
                        </div>
                    </div>
                }.into_any()
            }}
        </div>
    }
}
