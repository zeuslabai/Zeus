// ═══════════════════════════════════════════════════════════
// ZEUS — AI Tools Page — S21 Phase 4 P1
// Web Search (Wikipedia + DDG) + Image Generation (Fooocus)
// ═══════════════════════════════════════════════════════════

use crate::api;
use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

#[component]
pub fn AiToolsPage() -> impl IntoView {
    let active_tab = RwSignal::new(0u8); // 0=search 1=imagegen

    view! {
        <div style="padding: 32px; max-width: 1100px; margin: 0 auto; font-family: 'Rajdhani', sans-serif; color: rgba(255,245,240,0.9);">
            <div style="margin-bottom: 24px;">
                <h1 style="font-family: 'Orbitron', monospace; font-size: 20px; font-weight: 700; letter-spacing: 4px; color: rgba(255,245,240,0.9); margin: 0 0 4px 0;">"AI TOOLS"</h1>
                <p style="font-size: 13px; color: rgba(255,245,240,0.5); margin: 0;">"Web search and image generation"</p>
            </div>

            // Tabs
            <div style="display: flex; gap: 4px; margin-bottom: 24px; background: rgba(255,255,255,0.02); border-radius: 10px; padding: 4px; width: fit-content;">
                {[("Web Search", 0u8), ("Image Generation", 1u8)].iter().map(|(label, idx)| {
                    let idx = *idx;
                    let label = *label;
                    view! {
                        <button
                            on:click=move |_| active_tab.set(idx)
                            style=move || format!(
                                "padding: 8px 18px; border-radius: 8px; border: none; cursor: pointer; font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; transition: all 0.15s; background: {}; color: {};",
                                if active_tab.get() == idx { "rgba(255,60,20,0.2)" } else { "transparent" },
                                if active_tab.get() == idx { "rgba(255,140,80,1)" } else { "rgba(255,245,240,0.5)" },
                            )
                        >{label}</button>
                    }
                }).collect_view()}
            </div>

            {move || (active_tab.get() == 0).then(|| view! { <WebSearchPane /> })}
            {move || (active_tab.get() == 1).then(|| view! { <ImageGenPane /> })}
        </div>
    }
}

// ─── WEB SEARCH ──────────────────────────────────────────

#[component]
fn WebSearchPane() -> impl IntoView {
    let query = RwSignal::new(String::new());
    let results = RwSignal::new(Vec::<api::SearchResult>::new());
    let searching = RwSignal::new(false);
    let error = RwSignal::new(String::new());
    let searched = RwSignal::new(false);

    let run_search = move || {
        let q = query.get_untracked();
        if q.is_empty() { return; }
        searching.set(true);
        error.set(String::new());
        results.set(vec![]);
        searched.set(false);
        spawn_local(async move {
            match api::web_search(&q).await {
                Ok(r) => { results.set(r); searched.set(true); }
                Err(e) => error.set(e),
            }
            searching.set(false);
        });
    };

    let do_search_click = move |_: leptos::ev::MouseEvent| run_search();
    let do_search_key = move |e: leptos::ev::KeyboardEvent| { if e.key() == "Enter" { run_search(); } };

    view! {
        <div>
            // Search bar
            <div style="display: flex; gap: 10px; margin-bottom: 24px;">
                <input
                    placeholder="Search Wikipedia + DuckDuckGo..."
                    prop:value=move || query.get()
                    on:input=move |e| query.set(event_target_value(&e))
                    on:keydown=do_search_key
                    style="flex: 1; padding: 12px 16px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.2); border-radius: 10px; color: rgba(255,245,240,0.9); font-size: 15px; outline: none; font-family: 'Rajdhani', sans-serif;"
                />
                <button
                    disabled=move || searching.get()
                    on:click=do_search_click
                    style="padding: 12px 24px; background: rgba(255,60,20,0.15); border: 1px solid rgba(255,60,20,0.4); border-radius: 10px; color: rgba(255,140,80,1); font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; cursor: pointer;"
                >{move || if searching.get() { "SEARCHING..." } else { "SEARCH" }}</button>
            </div>

            // Error
            {move || (!error.get().is_empty()).then(|| view! {
                <div style="padding: 12px 16px; background: rgba(239,68,68,0.08); border: 1px solid rgba(239,68,68,0.2); border-radius: 8px; color: rgba(239,68,68,0.8); font-size: 13px; margin-bottom: 16px;">{error.get()}</div>
            })}

            // Results
            {move || {
                let rs = results.get();
                if rs.is_empty() && searched.get() {
                    Some(view! {
                        <div style="text-align: center; padding: 48px; color: rgba(255,245,240,0.3);">
                            <div style="font-size: 32px; margin-bottom: 12px;">"🔍"</div>
                            <div>"No results found"</div>
                        </div>
                    }.into_any())
                } else if !rs.is_empty() {
                    Some(view! {
                        <div>
                            <div style="font-size: 12px; color: rgba(255,245,240,0.4); margin-bottom: 12px;">{format!("{} results", rs.len())}</div>
                            <div style="display: flex; flex-direction: column; gap: 12px;">
                            {rs.into_iter().map(|r| view! {
                                <div style="padding: 16px 18px; background: rgba(255,255,255,0.02); border: 1px solid rgba(255,60,20,0.08); border-radius: 10px;">
                                    <a href={r.url.clone()} target="_blank" rel="noopener"
                                        style="font-size: 15px; font-weight: 600; color: rgba(255,140,80,0.9); text-decoration: none; display: block; margin-bottom: 4px;">
                                        {r.title}
                                    </a>
                                    <div style="font-size: 12px; color: rgba(255,245,240,0.4); font-family: monospace; margin-bottom: 6px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">{r.url}</div>
                                    <div style="font-size: 13px; color: rgba(255,245,240,0.7); line-height: 1.5;">{r.snippet}</div>
                                </div>
                            }).collect_view()}
                            </div>
                        </div>
                    }.into_any())
                } else {
                    Some(view! {
                        <div style="text-align: center; padding: 64px; color: rgba(255,245,240,0.2);">
                            <div style="font-size: 40px; margin-bottom: 16px;">"🌐"</div>
                            <div style="font-size: 14px;">"Enter a query to search"</div>
                        </div>
                    }.into_any())
                }
            }}
        </div>
    }
}

// ─── IMAGE GENERATION ───────────────────────────────────

#[component]
fn ImageGenPane() -> impl IntoView {
    let prompt = RwSignal::new(String::new());
    let style = RwSignal::new(String::new());
    let size = RwSignal::new("1024×1024".to_string());
    let generating = RwSignal::new(false);
    let result_b64 = RwSignal::new(Option::<String>::None);
    let error = RwSignal::new(String::new());
    let history = RwSignal::new(Vec::<serde_json::Value>::new());

    // Load image history
    spawn_local(async move {
        if let Ok(r) = api::fetch_images().await {
            history.set(r.images);
        }
    });

    let do_generate = move |_: leptos::ev::MouseEvent| {
        let p = prompt.get_untracked();
        if p.is_empty() { error.set("Prompt required".into()); return; }
        let s = style.get_untracked();
        let sz = size.get_untracked();
        generating.set(true);
        error.set(String::new());
        result_b64.set(None);
        spawn_local(async move {
            let style_opt = if s.is_empty() { None } else { Some(s.as_str()) };
            match api::generate_image(&p, style_opt, Some(&sz)).await {
                Ok(data) => result_b64.set(Some(data)),
                Err(e) => error.set(e),
            }
            generating.set(false);
            // Refresh history
            if let Ok(r) = api::fetch_images().await {
                history.set(r.images);
            }
        });
    };

    let styles = ["Fooocus V2", "Cinematic", "Anime", "Photographic", "Digital Art", "Fantasy Art"];
    let sizes = ["1024×1024", "1152×896", "896×1152", "1280×768", "768×1280"];

    view! {
        <div style="display: flex; gap: 24px; align-items: flex-start; flex-wrap: wrap;">
            // Left: generation form
            <div style="flex: 1; min-width: 300px;">
                <div style="background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.15); border-radius: 12px; padding: 20px; margin-bottom: 16px;">
                    <div style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 2px; color: rgba(255,60,20,0.6); margin-bottom: 16px;">"GENERATE IMAGE"</div>

                    <div style="margin-bottom: 12px;">
                        <label style="font-size: 11px; color: rgba(255,245,240,0.5); display: block; margin-bottom: 6px; text-transform: uppercase; letter-spacing: 1px;">"Prompt"</label>
                        <textarea rows="4"
                            placeholder="A majestic mountain at sunset with aurora borealis..."
                            prop:value=move || prompt.get()
                            on:input=move |e| prompt.set(event_target_value(&e))
                            style="width: 100%; padding: 10px 12px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.2); border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 13px; outline: none; box-sizing: border-box; resize: vertical; font-family: 'Rajdhani', sans-serif; line-height: 1.5;"
                        />
                    </div>

                    <div style="display: flex; gap: 12px; margin-bottom: 16px; flex-wrap: wrap;">
                        <div style="flex: 1; min-width: 140px;">
                            <label style="font-size: 11px; color: rgba(255,245,240,0.5); display: block; margin-bottom: 6px; text-transform: uppercase; letter-spacing: 1px;">"Style"</label>
                            <select
                                prop:value=move || style.get()
                                on:change=move |e| style.set(event_target_value(&e))
                                style="width: 100%; padding: 9px 12px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.2); border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 13px; outline: none; cursor: pointer;"
                            >
                                {styles.iter().map(|s| view! { <option value={*s}>{*s}</option> }).collect_view()}
                            </select>
                        </div>
                        <div style="flex: 1; min-width: 120px;">
                            <label style="font-size: 11px; color: rgba(255,245,240,0.5); display: block; margin-bottom: 6px; text-transform: uppercase; letter-spacing: 1px;">"Size"</label>
                            <select
                                prop:value=move || size.get()
                                on:change=move |e| size.set(event_target_value(&e))
                                style="width: 100%; padding: 9px 12px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.2); border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 13px; outline: none; cursor: pointer;"
                            >
                                {sizes.iter().map(|s| view! { <option value={*s}>{*s}</option> }).collect_view()}
                            </select>
                        </div>
                    </div>

                    {move || (!error.get().is_empty()).then(|| view! {
                        <div style="padding: 10px 14px; background: rgba(239,68,68,0.08); border: 1px solid rgba(239,68,68,0.2); border-radius: 8px; color: rgba(239,68,68,0.8); font-size: 12px; margin-bottom: 12px;">{error.get()}</div>
                    })}

                    <button
                        disabled=move || generating.get()
                        on:click=do_generate
                        style="width: 100%; padding: 12px; background: rgba(255,60,20,0.2); border: 1px solid rgba(255,60,20,0.5); border-radius: 8px; color: rgba(255,140,80,1); font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 2px; cursor: pointer;"
                    >{move || if generating.get() { "GENERATING..." } else { "GENERATE" }}</button>

                    // Note about Fooocus
                    <div style="margin-top: 12px; font-size: 11px; color: rgba(255,245,240,0.3); line-height: 1.5;">"Requires Fooocus server running (ZEUS_FOOOCUS_URL). Set in gateway config."</div>
                </div>

                // Result image
                {move || result_b64.get().map(|data| {
                    let src = if data.starts_with("http") {
                        data.clone()
                    } else {
                        format!("data:image/png;base64,{}", data)
                    };
                    view! {
                        <div style="background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.15); border-radius: 12px; overflow: hidden;">
                            <img src={src} alt="Generated image" style="width: 100%; display: block; border-radius: 8px;" />
                            <div style="padding: 12px 16px; font-size: 11px; color: rgba(255,245,240,0.4);">"Generated · Click to view full size"</div>
                        </div>
                    }
                })}

                // Loading placeholder
                {move || generating.get().then(|| view! {
                    <div style="background: rgba(255,255,255,0.02); border: 1px solid rgba(255,60,20,0.1); border-radius: 12px; height: 300px; display: flex; align-items: center; justify-content: center; color: rgba(255,245,240,0.4); font-size: 13px;">
                        "Generating image... this may take 15–60s"
                    </div>
                })}
            </div>

            // Right: image history
            <div style="width: 260px; flex-shrink: 0;">
                <div style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 2px; color: rgba(255,60,20,0.6); margin-bottom: 12px;">"HISTORY"</div>
                {move || {
                    let imgs = history.get();
                    if imgs.is_empty() {
                        view! { <div style="color: rgba(255,245,240,0.3); font-size: 12px;">"No images yet"</div> }.into_any()
                    } else {
                        view! {
                            <div style="display: flex; flex-direction: column; gap: 8px;">
                            {imgs.into_iter().map(|img| {
                                let prompt_txt = img.get("prompt").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                let id = img.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                let url = img.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                view! {
                                    <div style="background: rgba(255,255,255,0.02); border: 1px solid rgba(255,60,20,0.08); border-radius: 8px; overflow: hidden;">
                                        {(!url.is_empty()).then(|| view! {
                                            <img src={url} alt={prompt_txt.clone()} style="width: 100%; display: block;" />
                                        })}
                                        <div style="padding: 8px 10px;">
                                            <div style="font-size: 10px; color: rgba(255,245,240,0.4); overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">{prompt_txt}</div>
                                            {(!id.is_empty()).then(|| view! {
                                                <div style="font-family: monospace; font-size: 9px; color: rgba(255,245,240,0.25); margin-top: 2px;">{id}</div>
                                            })}
                                        </div>
                                    </div>
                                }
                            }).collect_view()}
                            </div>
                        }.into_any()
                    }
                }}
            </div>
        </div>
    }
}
