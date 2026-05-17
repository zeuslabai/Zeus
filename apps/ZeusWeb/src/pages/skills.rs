// ═══════════════════════════════════════════════════════════
// ZEUS — Skills Page — Phase 3: Enriched with OpenClaw metadata
// ═══════════════════════════════════════════════════════════

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;
use crate::api;
use crate::components::design::*;

const CATEGORIES: &[&str] = &[
    "all", "development", "messaging", "infrastructure",
    "security", "writing", "research", "data", "general",
];

#[component]
pub fn SkillsPage() -> impl IntoView {
    let skills = RwSignal::new(Vec::<api::Skill>::new());
    let categories = RwSignal::new(Vec::<api::SkillCategory>::new());
    let featured_mkt = RwSignal::new(Vec::<api::MarketplaceListing>::new());
    let active_category = RwSignal::new("all".to_string());
    let search = RwSignal::new(String::new());
    let show_install = RwSignal::new(false);
    let show_detail = RwSignal::new(false);
    let detail_skill = RwSignal::new(Option::<api::Skill>::None);
    let inst_name = RwSignal::new(String::new());
    let inst_content = RwSignal::new(String::new());
    let installing = RwSignal::new(false);
    let install_msg = RwSignal::new(String::new());

    let active_tab = RwSignal::new("installed".to_string());
    let mkt_listings = RwSignal::new(Vec::<api::MarketplaceListing>::new());
    let mkt_search = RwSignal::new(String::new());
    let mkt_category = RwSignal::new("all".to_string());
    let mkt_loading = RwSignal::new(false);
    let mkt_loaded = RwSignal::new(false);
    let mkt_total = RwSignal::new(0usize);
    let getting: RwSignal<Option<String>> = RwSignal::new(None);
    let skills_loaded = RwSignal::new(false);

    // Initial load
    {
        spawn_local(async move {
            if let Ok(s) = api::fetch_skills().await {
                skills.set(s.skills);
            }
            skills_loaded.set(true);
            if let Ok(c) = api::fetch_skill_categories().await {
                categories.set(c.categories);
            }
            if let Ok(r) = api::fetch_marketplace_featured(6).await {
                featured_mkt.set(r.listings);
            }
        });
    }

    let _reload = move || {
        spawn_local(async move {
            if let Ok(s) = api::fetch_skills().await {
                skills.set(s.skills);
            }
            if let Ok(c) = api::fetch_skill_categories().await {
                categories.set(c.categories);
            }
        });
    };

    let toggle = move |id: String, currently_enabled: bool| {
        spawn_local(async move {
            let _ = api::toggle_skill(&id, !currently_enabled).await;
            if let Ok(s) = api::fetch_skills().await { skills.set(s.skills); }
        });
    };

    let open_detail = move |skill_id: String| {
        spawn_local(async move {
            if let Ok(s) = api::fetch_skill(&skill_id).await {
                detail_skill.set(Some(s));
                show_detail.set(true);
            }
        });
    };

    let do_install = move |_| {
        let name = inst_name.get_untracked();
        if name.trim().is_empty() { install_msg.set("Name required".into()); return; }
        let content_str = inst_content.get_untracked();
        installing.set(true);
        install_msg.set(String::new());
        spawn_local(async move {
            let req = api::InstallSkillReq {
                name: name.clone(),
                content: if content_str.is_empty() { None } else { Some(content_str) },
            };
            match api::install_skill(&req).await {
                Ok(_) => {
                    show_install.set(false);
                    inst_name.set(String::new());
                    inst_content.set(String::new());
                    if let Ok(s) = api::fetch_skills().await { skills.set(s.skills); }
                    if let Ok(c) = api::fetch_skill_categories().await { categories.set(c.categories); }
                }
                Err(e) => install_msg.set(format!("Error: {}", e)),
            }
            installing.set(false);
        });
    };

    let load_marketplace = move || {
        if mkt_loading.get_untracked() { return; }
        mkt_loading.set(true);
        let q = mkt_search.get_untracked();
        let cat = mkt_category.get_untracked();
        spawn_local(async move {
            let q_opt = if q.is_empty() { None } else { Some(q.clone()) };
            let cat_opt = if cat == "all" { None } else { Some(cat.clone()) };
            if let Ok(r) = api::fetch_marketplace_listings(None, cat_opt.as_deref(), q_opt.as_deref(), None).await { mkt_total.set(r.total); mkt_listings.set(r.listings); mkt_loaded.set(true); }
            mkt_loading.set(false);
        });
    };

    let get_from_marketplace = move |listing_name: String| {
        getting.set(Some(listing_name.clone()));
        spawn_local(async move {
            let req = api::InstallSkillReq { name: listing_name.clone(), content: None };
            let _ = api::install_skill(&req).await;
            if let Ok(s) = api::fetch_skills().await { skills.set(s.skills); }
            getting.set(None);
        });
    };

    view! {
        // ── Install Modal ──
        <Show when=move || show_install.get()>
            <div style="position: fixed; inset: 0; background: rgba(0,0,0,0.78); z-index: 1000; display: flex; align-items: center; justify-content: center;">
                <div style="background: #0d0704; border: 1px solid rgba(255,60,20,0.25); border-radius: 16px; padding: 32px; width: 480px; max-width: 92vw; box-shadow: 0 0 60px rgba(255,60,20,0.15);">
                    <div style="font-family: 'Orbitron', monospace; font-size: 11px; letter-spacing: 5px; color: rgba(255,245,240,0.9); margin-bottom: 20px;">"INSTALL SKILL"</div>
                    <div style="display: flex; flex-direction: column; gap: 14px;">
                        <div>
                            <div style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,245,240,0.7); margin-bottom: 6px;">"NAME *"</div>
                            <input type="text" placeholder="Skill name or ClawHub slug" style="width: 100%; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 8px; padding: 10px 14px; color: rgba(255,245,240,0.9); font-family: 'Rajdhani', sans-serif; font-size: 14px; box-sizing: border-box; outline: none;"
                                prop:value=move || inst_name.get()
                                on:input=move |ev| inst_name.set(event_target_value(&ev))
                            />
                        </div>
                        <div>
                            <div style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,245,240,0.7); margin-bottom: 6px;">"SKILL.MD CONTENT (OPTIONAL)"</div>
                            <textarea rows=6 placeholder="Paste SKILL.md content or leave empty to fetch from ClawHub..." style="width: 100%; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 8px; padding: 10px 14px; color: rgba(255,245,240,0.9); font-family: monospace; font-size: 12px; box-sizing: border-box; outline: none; resize: vertical;"
                                prop:value=move || inst_content.get()
                                on:input=move |ev| inst_content.set(event_target_value(&ev))
                            />
                        </div>
                    </div>
                    <Show when=move || !install_msg.get().is_empty()>
                        <div style="margin-top: 10px; font-size: 13px; color: rgba(255,60,20,0.9);">{move || install_msg.get()}</div>
                    </Show>
                    <div style="display: flex; gap: 10px; margin-top: 20px; justify-content: flex-end;">
                        <Button on_click=Some(Callback::new(move |_| { show_install.set(false); install_msg.set(String::new()); }))>"Cancel"</Button>
                        <Button primary=true on_click=Some(Callback::new(do_install))>
                            {move || if installing.get() { "Installing..." } else { "Install" }}
                        </Button>
                    </div>
                </div>
            </div>
        </Show>

        // ── Detail Modal ──
        <Show when=move || show_detail.get()>
            <div style="position: fixed; inset: 0; background: rgba(0,0,0,0.78); z-index: 1000; display: flex; align-items: center; justify-content: center; overflow-y: auto; padding: 20px 0;"
                on:click=move |_| show_detail.set(false)>
                <div style="background: #0d0704; border: 1px solid rgba(255,60,20,0.25); border-radius: 16px; padding: 32px; width: 600px; max-width: 92vw; max-height: 85vh; overflow-y: auto; box-shadow: 0 0 60px rgba(255,60,20,0.15);"
                    on:click=move |ev| ev.stop_propagation()>
                    {move || {
                        let Some(sk) = detail_skill.get() else {
                            return view! { <div>"Loading..."</div> }.into_any();
                        };
                        let emoji = sk.emoji.clone().unwrap_or_default();
                        let author_display = sk.author.clone().unwrap_or_default();
                        let has_reqs = sk.requires.is_some();
                        let reqs_satisfied = sk.requires.as_ref().map(|r| r.satisfied).unwrap_or(true);
                        let reqs_summary = sk.requires.as_ref().map(|r| r.summary.clone()).unwrap_or_default();
                        let homepage = sk.homepage.clone();
                        let sys_prompt = sk.system_prompt.clone().unwrap_or_default();
                        let tools = sk.tools.clone().unwrap_or_default();
                        let install_specs = sk.install_specs.clone().unwrap_or_default();

                        view! {
                            <div>
                                // Header
                                <div style="display: flex; align-items: center; gap: 12px; margin-bottom: 16px;">
                                    {(!emoji.is_empty()).then(|| view! {
                                        <span style="font-size: 28px;">{emoji.clone()}</span>
                                    })}
                                    <div>
                                        <div style="font-family: 'Orbitron', monospace; font-size: 14px; letter-spacing: 3px; color: rgba(255,245,240,0.9);">{sk.name.clone()}</div>
                                        <div style="font-size: 12px; color: rgba(255,245,240,0.35); margin-top: 2px;">
                                            {format!("v{}", sk.version)}
                                            {(!author_display.is_empty()).then(|| format!(" by {}", author_display))}
                                        </div>
                                    </div>
                                </div>
                                // Category + tags
                                <div style="display: flex; gap: 6px; flex-wrap: wrap; margin-bottom: 12px;">
                                    <Badge text={sk.category.to_uppercase()} color="rgba(255,60,20,0.6)".to_string() />
                                    {sk.tags.iter().map(|t| view! {
                                        <Badge text={t.clone()} />
                                    }).collect::<Vec<_>>()}
                                </div>
                                // Description
                                <div style="font-size: 13px; color: rgba(255,245,240,0.65); line-height: 1.6; margin-bottom: 16px;">{sk.description.clone()}</div>
                                // Requirements
                                {has_reqs.then(|| view! {
                                    <div style="margin-bottom: 16px;">
                                        <div style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,245,240,0.7); margin-bottom: 8px;">"REQUIREMENTS"</div>
                                        <div style="display: flex; align-items: center; gap: 8px; margin-bottom: 6px;">
                                            <div style={format!("width: 8px; height: 8px; border-radius: 50%; background: {};",
                                                if reqs_satisfied { "#22c55e" } else { "#ef4444" })} />
                                            <span style="font-size: 12px; color: rgba(255,245,240,0.65);">{reqs_summary.clone()}</span>
                                        </div>
                                    </div>
                                })}
                                // Install specs
                                {(!install_specs.is_empty()).then(|| view! {
                                    <div style="margin-bottom: 16px;">
                                        <div style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,245,240,0.7); margin-bottom: 8px;">"INSTALL OPTIONS"</div>
                                        {install_specs.iter().map(|spec| {
                                            let label = spec.label.clone().unwrap_or_else(|| spec.kind.clone());
                                            view! {
                                                <div style="font-size: 12px; color: rgba(255,245,240,0.5); padding: 4px 0;">
                                                    {format!("{} ({})", label, spec.kind)}
                                                    {spec.formula.as_ref().map(|f| format!(" — {}", f))}
                                                </div>
                                            }
                                        }).collect::<Vec<_>>()}
                                    </div>
                                })}
                                // Tools
                                {(!tools.is_empty()).then(|| view! {
                                    <div style="margin-bottom: 16px;">
                                        <div style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,245,240,0.7); margin-bottom: 8px;">
                                            {format!("TOOLS ({})", tools.len())}
                                        </div>
                                        {tools.iter().map(|t| {
                                            let name = t.get("name").and_then(|v| v.as_str()).unwrap_or("?").to_string();
                                            let desc = t.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                            view! {
                                                <div style="font-size: 12px; padding: 4px 0; border-bottom: 1px solid rgba(255,245,240,0.04);">
                                                    <span style="color: rgba(255,60,20,0.7); font-family: monospace;">{name}</span>
                                                    {(!desc.is_empty()).then(|| view! {
                                                        <span style="color: rgba(255,245,240,0.35);"> " — " {desc}</span>
                                                    })}
                                                </div>
                                            }
                                        }).collect::<Vec<_>>()}
                                    </div>
                                })}
                                // Permissions
                                {(!sk.permissions.is_empty()).then(|| view! {
                                    <div style="margin-bottom: 16px;">
                                        <div style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,245,240,0.7); margin-bottom: 8px;">"PERMISSIONS"</div>
                                        <div style="display: flex; gap: 6px; flex-wrap: wrap;">
                                            {sk.permissions.iter().map(|p| view! {
                                                <Badge text={p.clone()} color="rgba(239,68,68,0.5)".to_string() />
                                            }).collect::<Vec<_>>()}
                                        </div>
                                    </div>
                                })}
                                // System prompt preview
                                {(!sys_prompt.is_empty()).then(|| {
                                    let preview = if sys_prompt.len() > 300 {
                                        format!("{}...", &sys_prompt[..{let mut i=300.min(sys_prompt.len()); while !sys_prompt.is_char_boundary(i){i-=1;} i}])
                                    } else {
                                        sys_prompt.clone()
                                    };
                                    view! {
                                        <div style="margin-bottom: 16px;">
                                            <div style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,245,240,0.7); margin-bottom: 8px;">"SYSTEM PROMPT"</div>
                                            <div style="font-size: 11px; color: rgba(255,245,240,0.4); font-family: monospace; background: rgba(255,255,255,0.02); border-radius: 8px; padding: 12px; white-space: pre-wrap; max-height: 200px; overflow-y: auto;">{preview}</div>
                                        </div>
                                    }
                                })}
                                // Homepage link
                                {homepage.map(|url| {
                                    let url_display = url.clone();
                                    view! {
                                        <div style="margin-bottom: 16px;">
                                            <a href={url} target="_blank" style="font-size: 12px; color: rgba(255,60,20,0.7); text-decoration: none;">
                                                {url_display} " ↗"
                                            </a>
                                        </div>
                                    }
                                })}
                                // Close button
                                <div style="display: flex; justify-content: flex-end; margin-top: 12px;">
                                    <Button on_click=Some(Callback::new(move |_| show_detail.set(false)))>"Close"</Button>
                                </div>
                            </div>
                        }.into_any()
                    }}
                </div>
            </div>
        </Show>

        // ── Main Page ──
        <div style="padding: 32px;">
            // Header row
            <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 20px;">
                <div>
                    <h1 style="font-family: 'Orbitron', monospace; font-size: 14px; letter-spacing: 6px; color: rgba(255,245,240,0.9); margin: 0;">"SKILLS"</h1>
                    <p style="font-size: 12px; color: rgba(255,245,240,0.7); margin: 4px 0 0;">
                        {move || {
                            let s = skills.get();
                            let enabled = s.iter().filter(|sk| sk.enabled).count();
                            if !skills_loaded.get() { "Loading skills...".to_string() }
                            else if s.is_empty() { "No skills installed".to_string() }
                            else { format!("{} skills \u{00b7} {} enabled", s.len(), enabled) }
                        }}
                    </p>
                </div>
                <div style="display: flex; gap: 10px; align-items: center;">
                    <a href="https://claw.computer/skills" target="_blank"
                        style="padding: 8px 16px; border-radius: 8px; border: 1px solid rgba(255,60,20,0.2); background: rgba(255,60,20,0.06); color: rgba(255,140,80,0.8); font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; text-decoration: none; display: flex; align-items: center; gap: 6px; transition: all 0.2s;">
                        "\u{1f517} 13K+ ON CLAWHUB \u{2197}"
                    </a>
                    <Button primary=true on_click=Some(Callback::new(move |_| show_install.set(true)))>
                        <Icon name="plus" size=12 /> " Install Skill"
                    </Button>
                </div>
            </div>

            // ── Tab bar ──
            <div style="display: flex; gap: 0; margin-bottom: 20px; border-bottom: 1px solid rgba(255,245,240,0.06);">
                {["installed", "marketplace"].iter().map(|tab| {
                    let t = tab.to_string();
                    let t_active = tab.to_string();
                    let t_count = tab.to_string();
                    let t_label = if *tab == "installed" { "INSTALLED" } else { "MARKETPLACE" };
                    view! {
                        <button
                            on:click={
                                let t = t.clone();
                                move |_| {
                                    active_tab.set(t.clone());
                                    if t == "marketplace" && !mkt_loaded.get_untracked() {
                                        load_marketplace();
                                    }
                                }
                            }
                            style=move || {
                                let active = active_tab.get() == t_active;
                                format!(
                                    "padding: 8px 20px; border: none; background: transparent; border-bottom: 2px solid {}; color: {}; font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; cursor: pointer; margin-bottom: -1px;",
                                    if active { "rgba(255,60,20,0.7)" } else { "transparent" },
                                    if active { "rgba(255,245,240,0.9)" } else { "rgba(255,245,240,0.3)" },
                                )
                            }
                        >
                            {t_label}
                            {move || {
                                if t_count == "installed" {
                                    let n = skills.get().len();
                                    if n > 0 { format!(" ({})", n) } else { String::new() }
                                } else {
                                    let n = mkt_total.get();
                                    if n > 0 { format!(" ({})", n) } else { String::new() }
                                }
                            }}
                        </button>
                    }
                }).collect::<Vec<_>>()}
            </div>

            // ── INSTALLED tab ──
            <Show when=move || active_tab.get() == "installed">
            // ── Featured from Agora ──
            {move || {
                let feat = featured_mkt.get();
                if feat.is_empty() { return view! { <div /> }.into_any(); }
                view! {
                    <div style="margin-bottom: 28px;">
                        <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 10px;">
                            <div style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 4px; color: rgba(255,245,240,0.5);">
                                "\u{2b50} FEATURED IN AGORA"
                            </div>
                            <a href="/agora" style="font-size: 11px; color: rgba(255,60,20,0.6); text-decoration: none;">"Browse marketplace \u{2192}"</a>
                        </div>
                        <div style="display: flex; gap: 12px; overflow-x: auto; padding-bottom: 8px;">
                            {feat.into_iter().map(|listing| {
                                view! {
                                    <div style="min-width: 220px; flex-shrink: 0; background: linear-gradient(135deg, rgba(255,60,20,0.06), rgba(255,140,80,0.03)); \
                                        border: 1px solid rgba(255,60,20,0.15); border-radius: 12px; padding: 14px;">
                                        <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 6px;">
                                            <div style="font-family: 'Orbitron', monospace; font-size: 11px; color: rgba(255,245,240,0.9); font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; flex: 1; margin-right: 8px;">
                                                {listing.name.clone()}
                                            </div>
                                            <div style="font-size: 10px; color: rgba(255,140,80,0.9); white-space: nowrap;">
                                                {if listing.price_tokens == 0 { "FREE".to_string() } else { format!("{} \u{26a1}", listing.price_tokens) }}
                                            </div>
                                        </div>
                                        <div style="font-size: 11px; color: rgba(255,245,240,0.35); line-height: 1.4; display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden;">
                                            {if listing.description.is_empty() { "No description.".to_string() } else { listing.description.clone() }}
                                        </div>
                                        <div style="display: flex; justify-content: space-between; align-items: center; margin-top: 10px;">
                                            <span style="font-size: 10px; color: rgba(234,179,8,0.7);">"\u{2605} "{format!("{:.1}", listing.rating)}</span>
                                            <a href="/agora" style="font-size: 9px; color: rgba(255,60,20,0.6); text-decoration: none; font-family: 'Orbitron', monospace; letter-spacing: 1px;">"GET \u{2192}"</a>
                                        </div>
                                    </div>
                                }
                            }).collect::<Vec<_>>()}
                        </div>
                    </div>
                }.into_any()
            }}

            // Category tabs
            <div style="display: flex; gap: 4px; margin-bottom: 16px; flex-wrap: wrap;">
                {CATEGORIES.iter().map(|cat| {
                    let cat_str = cat.to_string();
                    let cat_str_style = cat.to_string();
                    let cat_str_count = cat.to_string();
                    let cat_upper = if *cat == "all" { "ALL".to_string() } else { cat.to_uppercase() };
                    view! {
                        <button
                            on:click=move |_| active_category.set(cat_str.clone())
                            style=move || {
                                let is_active = active_category.get() == cat_str_style;
                                format!(
                                    "padding: 6px 14px; border-radius: 6px; font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 2px; cursor: pointer; border: 1px solid {}; background: {}; color: {};",
                                    if is_active { "rgba(255,60,20,0.4)" } else { "rgba(255,245,240,0.08)" },
                                    if is_active { "rgba(255,60,20,0.15)" } else { "transparent" },
                                    if is_active { "rgba(255,245,240,0.9)" } else { "rgba(255,245,240,0.35)" },
                                )
                            }
                        >
                            {cat_upper}
                            // Show count badge from categories data
                            {move || {
                                if cat_str_count == "all" { return String::new(); }
                                categories.get().iter()
                                    .find(|c| c.name == cat_str_count)
                                    .map(|c| format!(" ({})", c.count))
                                    .unwrap_or_default()
                            }}
                        </button>
                    }
                }).collect::<Vec<_>>()}
            </div>

            // Search bar
            <SearchBar placeholder="Search skills by name, description, or tags..." value=search />

            // Empty state — installed tab
            <Show when=move || skills_loaded.get() && skills.get().is_empty()>
                <div style="display: flex; flex-direction: column; align-items: center; justify-content: center; padding: 80px 32px; gap: 16px; text-align: center;">
                    <div style="font-size: 40px; opacity: 0.3;">"🧩"</div>
                    <div style="font-family: 'Orbitron', monospace; font-size: 11px; letter-spacing: 4px; color: rgba(255,245,240,0.5);">"NO SKILLS INSTALLED"</div>
                    <div style="font-size: 13px; color: rgba(255,245,240,0.4); max-width: 360px; line-height: 1.6;">"Skills extend Zeus with new capabilities. Browse the Marketplace tab or install a skill from ClawHub."</div>
                    <button on:click=move |_| active_tab.set("marketplace".to_string())
                        style="margin-top: 4px; padding: 10px 24px; border-radius: 8px; border: 1px solid rgba(255,60,20,0.3); background: rgba(255,60,20,0.1); color: #ff3c14; font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 3px; cursor: pointer;">
                        "BROWSE MARKETPLACE"
                    </button>
                </div>
            </Show>
            // Skills grid
            <div style="display: grid; grid-template-columns: repeat(auto-fill, minmax(320px, 1fr)); gap: 12px;">
                {move || {
                    let q = search.get().to_lowercase();
                    let cat = active_category.get();
                    skills.get().into_iter()
                        .filter(|s| {
                            // Category filter
                            if cat != "all" && s.category.to_lowercase() != cat.to_lowercase() {
                                return false;
                            }
                            // Text search
                            if !q.is_empty() {
                                let matches = s.name.to_lowercase().contains(&q)
                                    || s.description.to_lowercase().contains(&q)
                                    || s.tags.iter().any(|t| t.to_lowercase().contains(&q));
                                if !matches { return false; }
                            }
                            true
                        })
                        .map(|sk| {
                            let id_toggle = sk.id.clone();
                            let id_detail = sk.id.clone();
                            let id_del = sk.id.clone();
                            let currently_enabled = sk.enabled;
                            let emoji = sk.emoji.clone().unwrap_or_default();
                            let has_reqs = sk.requires.is_some();
                            let reqs_ok = sk.requires.as_ref().map(|r| r.satisfied).unwrap_or(true);
                            let author_display = sk.author.clone().unwrap_or_default();

                            view! {
                                <Card glow=sk.enabled>
                                    <div style="display: flex; align-items: flex-start; gap: 12px;">
                                        // Emoji or icon
                                        <div style={format!("width: 40px; height: 40px; border-radius: 10px; background: {}; display: flex; align-items: center; justify-content: center; flex-shrink: 0; font-size: 20px;",
                                            if sk.enabled { "rgba(34,197,94,0.1)" } else { "rgba(255,245,240,0.03)" }
                                        )}>
                                            {if !emoji.is_empty() {
                                                view! { <span>{emoji}</span> }.into_any()
                                            } else {
                                                view! { <Icon name="skills" size=18 color={if sk.enabled { "rgba(34,197,94,0.7)".to_string() } else { "rgba(255,245,240,0.5)".to_string() }} /> }.into_any()
                                            }}
                                        </div>

                                        <div style="flex: 1; min-width: 0;">
                                            // Name + badges
                                            <div style="display: flex; align-items: center; gap: 6px; margin-bottom: 4px; flex-wrap: wrap;">
                                                <span style="font-size: 14px; color: rgba(255,245,240,0.9); font-weight: 600; cursor: pointer;"
                                                    on:click={
                                                        let id_d = id_detail.clone();
                                                        move |_| open_detail(id_d.clone())
                                                    }>{sk.name.clone()}</span>
                                                <Badge text={sk.category.to_uppercase()} color="rgba(255,60,20,0.5)".to_string() />
                                                {(!sk.version.is_empty()).then(|| view! {
                                                    <Badge text={format!("v{}", sk.version)} />
                                                })}
                                                // Requirements status dot
                                                {has_reqs.then(|| view! {
                                                    <div style={format!("width: 8px; height: 8px; border-radius: 50%; background: {};",
                                                        if reqs_ok { "#22c55e" } else { "#ef4444" })}
                                                        title={if reqs_ok { "Requirements satisfied" } else { "Missing requirements" }} />
                                                })}
                                            </div>
                                            // Description
                                            <div style="font-size: 12px; color: rgba(255,245,240,0.7); margin-bottom: 6px; line-height: 1.4; display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden;">
                                                {if sk.description.is_empty() { "No description".to_string() } else { sk.description.clone() }}
                                            </div>
                                            // Meta row: author, tools count
                                            <div style="display: flex; gap: 8px; align-items: center; flex-wrap: wrap;">
                                                {(!author_display.is_empty()).then(|| view! {
                                                    <span style="font-size: 10px; color: rgba(255,245,240,0.5);">"by "{author_display}</span>
                                                })}
                                                {(sk.tools_count > 0).then(|| view! {
                                                    <span style="font-size: 10px; color: rgba(255,245,240,0.5);">{format!("{} tool{}", sk.tools_count, if sk.tools_count == 1 { "" } else { "s" })}</span>
                                                })}
                                            </div>
                                        </div>

                                        // Action buttons
                                        <div style="display: flex; flex-direction: column; gap: 4px; flex-shrink: 0;">
                                            <button
                                                on:click=move |_| toggle(id_toggle.clone(), currently_enabled)
                                                style={format!("padding: 5px 10px; border-radius: 6px; border: 1px solid {}; background: {}; color: {}; font-family: 'Orbitron', monospace; font-size: 7px; letter-spacing: 1px; cursor: pointer;",
                                                    if sk.enabled { "rgba(239,68,68,0.3)" } else { "rgba(34,197,94,0.3)" },
                                                    if sk.enabled { "rgba(239,68,68,0.1)" } else { "rgba(34,197,94,0.1)" },
                                                    if sk.enabled { "rgba(239,68,68,0.8)" } else { "rgba(34,197,94,0.8)" },
                                                )}
                                            >
                                                {if sk.enabled { "DISABLE" } else { "ENABLE" }}
                                            </button>
                                            <button
                                                on:click={
                                                    let did = id_del.clone();
                                                    move |_| {
                                                        let did = did.clone();
                                                        spawn_local(async move {
                                                            let _ = api::delete_skill(&did).await;
                                                            if let Ok(s) = api::fetch_skills().await { skills.set(s.skills); }
                                                        });
                                                    }
                                                }
                                                style="padding: 5px 10px; border-radius: 6px; border: 1px solid rgba(239,68,68,0.15); background: rgba(239,68,68,0.06); color: rgba(239,68,68,0.5); font-family: 'Orbitron', monospace; font-size: 7px; letter-spacing: 1px; cursor: pointer;"
                                            >"DEL"</button>
                                        </div>
                                    </div>
                                </Card>
                            }
                        })
                        .collect::<Vec<_>>()
                }}
            </div>
            </Show> // end installed tab

            // ── MARKETPLACE tab ──
            <Show when=move || active_tab.get() == "marketplace">
                <div style="display: flex; gap: 10px; margin-bottom: 16px; flex-wrap: wrap;">
                    <input
                        type="text"
                        placeholder="Search marketplace..."
                        prop:value=move || mkt_search.get()
                        on:input=move |e| {
                            use wasm_bindgen::JsCast;
                            let val = e.target().and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok()).map(|i| i.value()).unwrap_or_default();
                            mkt_search.set(val);
                        }
                        on:keydown=move |e| {
                            use wasm_bindgen::JsCast;
                            if e.unchecked_ref::<web_sys::KeyboardEvent>().key() == "Enter" {
                                mkt_loaded.set(false); load_marketplace();
                            }
                        }
                        style="flex: 1; min-width: 200px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.12); border-radius: 8px; padding: 9px 14px; color: rgba(255,245,240,0.9); font-family: 'Rajdhani', sans-serif; font-size: 14px; outline: none;"
                    />
                    <button on:click=move |_| { mkt_loaded.set(false); load_marketplace(); }
                        style="padding: 9px 18px; background: rgba(255,60,20,0.12); border: 1px solid rgba(255,60,20,0.25); border-radius: 8px; color: rgba(255,140,80,0.9); font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 2px; cursor: pointer;"
                    >"SEARCH"</button>
                </div>
                <div style="display: flex; gap: 6px; margin-bottom: 16px; flex-wrap: wrap;">
                    {CATEGORIES.iter().map(|cat| {
                        let c = cat.to_string();
                        let c_style = cat.to_string();
                        view! {
                            <button
                                on:click=move |_| { mkt_category.set(c.clone()); mkt_loaded.set(false); load_marketplace(); }
                                style=move || {
                                    let active = mkt_category.get() == c_style;
                                    format!("padding: 5px 12px; border-radius: 6px; border: 1px solid {}; background: {}; color: {}; font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 1px; cursor: pointer;",
                                        if active { "rgba(255,60,20,0.4)" } else { "rgba(255,245,240,0.08)" },
                                        if active { "rgba(255,60,20,0.15)" } else { "transparent" },
                                        if active { "rgba(255,245,240,0.9)" } else { "rgba(255,245,240,0.35)" },
                                    )
                                }
                            >{cat.to_uppercase()}</button>
                        }
                    }).collect::<Vec<_>>()}
                </div>
                <Show when=move || mkt_loading.get()>
                    <div style="text-align: center; padding: 40px; font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 3px; color: rgba(255,60,20,0.5);">"LOADING..."</div>
                </Show>
                <Show when=move || !mkt_loading.get() && mkt_loaded.get() && mkt_listings.get().is_empty()>
                    <div style="text-align: center; padding: 40px; font-size: 13px; color: rgba(255,245,240,0.5);">"No skills found."</div>
                </Show>
                <Show when=move || !mkt_listings.get().is_empty()>
                    <div style="display: grid; grid-template-columns: repeat(auto-fill, minmax(300px, 1fr)); gap: 12px;">
                        {move || mkt_listings.get().into_iter().map(|listing| {
                            let lname_get = listing.name.clone();
                            let lname_check = listing.name.clone();
                            let source_color = match listing.source.as_str() {
                                "builtin" => "#22c55e", "local" => "#3b82f6", "clawhub" => "#a855f7", _ => "rgba(255,245,240,0.3)",
                            };
                            let source_label = match listing.source.as_str() {
                                "builtin" => "BUILTIN", "local" => "LOCAL", "clawhub" => "CLAWHUB", _ => "UNKNOWN",
                            };
                            let trust = listing.trust_level();
                            view! {
                                <div style="background: rgba(255,255,255,0.02); border: 1px solid rgba(255,245,240,0.07); border-radius: 10px; padding: 14px; display: flex; flex-direction: column; gap: 8px;">
                                    <div style="display: flex; align-items: flex-start; justify-content: space-between; gap: 8px;">
                                        <div style="flex: 1; min-width: 0;">
                                            <div style="font-size: 14px; font-weight: 600; color: rgba(255,245,240,0.9); margin-bottom: 3px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">{listing.name.clone()}</div>
                                            <div style="font-size: 11px; color: rgba(255,245,240,0.4); line-height: 1.4; display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden;">
                                                {if listing.description.is_empty() { "No description.".to_string() } else { listing.description.clone() }}
                                            </div>
                                        </div>
                                        <div style="text-align: right; flex-shrink: 0;">
                                            <div style="font-family: 'Orbitron', monospace; font-size: 11px; color: rgba(255,140,80,0.9); font-weight: 700; margin-bottom: 3px;">
                                                {if listing.price_tokens == 0 { "FREE".to_string() } else { format!("{} \u{26a1}", listing.price_tokens) }}
                                            </div>
                                            <div style=format!("font-family: 'Orbitron', monospace; font-size: 7px; letter-spacing: 1px; padding: 2px 6px; border-radius: 3px; background: {}18; color: {};", source_color, source_color)>{source_label}</div>
                                        </div>
                                    </div>
                                    {if !listing.tags.is_empty() {
                                        view! {
                                            <div style="display: flex; gap: 4px; flex-wrap: wrap;">
                                                {listing.tags.clone().into_iter().take(4).map(|tag| view! {
                                                    <span style="font-size: 9px; padding: 1px 6px; background: rgba(255,60,20,0.08); border: 1px solid rgba(255,60,20,0.15); border-radius: 4px; color: rgba(255,140,80,0.7);">{tag}</span>
                                                }).collect::<Vec<_>>()}
                                            </div>
                                        }.into_any()
                                    } else { view! { <div/> }.into_any() }}
                                    <div style="display: flex; align-items: center; justify-content: space-between; margin-top: auto;">
                                        <div style="display: flex; gap: 10px; align-items: center;">
                                            {(listing.rating > 0.0).then(|| view! { <span style="font-size: 11px; color: rgba(234,179,8,0.7);">{format!("\u{2605} {:.1}", listing.rating)}</span> })}
                                            {(listing.downloads > 0).then(|| view! { <span style="font-size: 11px; color: rgba(255,245,240,0.2);">{format!("\u{2193} {}", listing.downloads)}</span> })}
                                            {(trust == 2).then(|| view! { <span style="font-size: 9px; color: #22c55e; font-family: 'Orbitron', monospace; letter-spacing: 1px;">"TRUSTED"</span> })}
                                        </div>
                                        <button
                                            on:click={ let n = lname_get.clone(); move |_| get_from_marketplace(n.clone()) }
                                            style={
                                                let lc = lname_check.clone();
                                                move || {
                                                    let busy = getting.get().as_deref() == Some(lc.as_str());
                                                    format!(
                                                        "padding: 5px 16px; background: {}; border: 1px solid {}; border-radius: 6px; color: {}; font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 2px; cursor: {};",
                                                        if busy { "rgba(255,60,20,0.05)" } else { "rgba(255,60,20,0.12)" },
                                                        if busy { "rgba(255,60,20,0.1)" } else { "rgba(255,60,20,0.3)" },
                                                        if busy { "rgba(255,245,240,0.2)" } else { "rgba(255,140,80,0.9)" },
                                                        if busy { "wait" } else { "pointer" },
                                                    )
                                                }
                                            }
                                        >{
                                            let lc2 = lname_check.clone();
                                            move || if getting.get().as_deref() == Some(lc2.as_str()) { "INSTALLING..." } else { "GET" }
                                        }</button>
                                    </div>
                                </div>
                            }
                        }).collect::<Vec<_>>()}
                    </div>
                </Show>
                <Show when=move || !mkt_loaded.get() && !mkt_loading.get()>
                    <div style="text-align: center; padding: 48px 20px;">
                        <div style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 4px; color: rgba(255,245,240,0.2); margin-bottom: 12px;">"SKILL MARKETPLACE"</div>
                        <div style="font-size: 13px; color: rgba(255,245,240,0.3); margin-bottom: 20px;">"Browse and install skills from Agora + ClawHub"</div>
                        <button on:click=move |_| load_marketplace()
                            style="padding: 10px 28px; background: rgba(255,60,20,0.12); border: 1px solid rgba(255,60,20,0.3); border-radius: 8px; color: rgba(255,140,80,0.9); font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 3px; cursor: pointer;"
                        >"BROWSE MARKETPLACE"</button>
                    </div>
                </Show>
            </Show> // end marketplace tab
        </div>
    }
}

