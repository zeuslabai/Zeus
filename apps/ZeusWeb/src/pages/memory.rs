// ═══════════════════════════════════════════════════════════
// ZEUS — Memory Page — Phase 7: + Knowledge Graph
// ═══════════════════════════════════════════════════════════

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;
use crate::api;
use crate::components::design::*;

#[component]
pub fn MemoryPage() -> impl IntoView {
    let search = RwSignal::new(String::new());
    let show_remember = RwSignal::new(false);
    let remember_text = RwSignal::new(String::new());
    let search_results = RwSignal::new(Vec::<api::MemorySearchResult>::new());
    let search_mode = RwSignal::new(false);
    let mem = RwSignal::new(api::MemoryResponse::default());
    let files = RwSignal::new(Vec::<api::MemoryFile>::new());
    let timeline = RwSignal::new(Vec::<serde_json::Value>::new());
    let timeline_total = RwSignal::new(0u32);
    let communities = RwSignal::new(Vec::<serde_json::Value>::new());
    let communities_total = RwSignal::new(0u32);
    // Phase 7: Knowledge Graph
    let graph_query = RwSignal::new(String::new());
    let graph_entity = RwSignal::new(String::new());
    let graph_results = RwSignal::new(Option::<serde_json::Value>::None);
    let graph_connections = RwSignal::new(Vec::<serde_json::Value>::new());
    let graph_loading = RwSignal::new(false);

    {
        let mem = mem;
        spawn_local(async move { if let Ok(m) = api::fetch_memory().await { mem.set(m); } });
    }
    {
        let files = files;
        spawn_local(async move { if let Ok(f) = api::fetch_memory_files().await { files.set(f.files); } });
    }
    {
        let timeline = timeline;
        let timeline_total = timeline_total;
        spawn_local(async move {
            if let Ok(t) = api::fetch_memory_timeline().await {
                timeline_total.set(t.total);
                timeline.set(t.entries);
            }
        });
    }
    {
        let communities = communities;
        let communities_total = communities_total;
        spawn_local(async move {
            if let Ok(c) = api::fetch_memory_communities().await {
                communities_total.set(c.total);
                communities.set(c.communities);
            }
        });
    }

    view! {
        <div style="padding: 32px;">
            <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 24px;">
                <div>
                    <h1 style="font-family: 'Orbitron', monospace; font-size: 14px; letter-spacing: 6px; color: rgba(255,245,240,0.9); margin: 0;">"MEMORY"</h1>
                    <p style="font-size: 12px; color: rgba(255,245,240,0.7); margin: 4px 0 0;">{move || {
                        let m = mem.get();
                        if m.total_chunks == 0 { "Loading memory stats...".to_string() }
                        else { format!("Mnemosyne • {} chunks • {} files indexed • {}", m.total_chunks, m.files_indexed, m.embedding_model) }
                    }}</p>
                </div>
                <div style="display: flex; gap: 8px;">
                    <Button small=true on_click=Some(Callback::new(move |_| {
                        let q = search.get_untracked();
                        if q.trim().is_empty() { return; }
                        search_mode.set(true);
                        spawn_local(async move {
                            match api::search_memory(&q).await {
                                Ok(r) => search_results.set(r.results),
                                Err(e) => web_sys::console::warn_1(&format!("Search failed: {}", e).into()),
                            }
                        });
                    }))>"Semantic Search"</Button>
                    <Button on_click=Some(Callback::new(move |_| {
                        spawn_local(async move {
                            match api::fetch_reindex().await {
                                Ok(r) => web_sys::console::log_1(&format!("Reindexed: {} scanned, {} changed", r.files_scanned, r.files_changed).into()),
                                Err(e) => web_sys::console::warn_1(&format!("Reindex failed: {}", e).into()),
                            }
                        });
                    }))>"Reindex"</Button>
                    <Button primary=true on_click=Some(Callback::new(move |_| show_remember.set(true)))>
                        <Icon name="plus" size=12 /> " Remember"
                    </Button>
                </div>
            </div>

            // Metric row
            {move || {
                let m = mem.get();
                view! {
                    <div style="display: flex; gap: 12px; margin-bottom: 20px; flex-wrap: wrap;">
                        <MetricCard label="Chunks" value={m.total_chunks.to_string()} icon="memory" />
                        <MetricCard label="Files Indexed" value={m.files_indexed.to_string()} icon="tools" />
                        <MetricCard label="Timeline" value={timeline_total.get().to_string()} icon="sessions" />
                        <MetricCard label="Communities" value={communities_total.get().to_string()} icon="agents" />
                    </div>
                }
            }}

            <SearchBar placeholder="Search memory (BM25 + semantic)..." value=search />

            // Remember modal
            <Show when=move || show_remember.get()>
                <div style="position: fixed; inset: 0; background: rgba(0,0,0,0.78); z-index: 1000; display: flex; align-items: center; justify-content: center;">
                    <div style="background: #0d0704; border: 1px solid rgba(255,60,20,0.25); border-radius: 16px; padding: 32px; width: 480px; max-width: 92vw;">
                        <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 20px;">
                            <div style="font-family: 'Orbitron', monospace; font-size: 11px; letter-spacing: 5px; color: rgba(255,245,240,0.9);">"REMEMBER"</div>
                            <button style="background: transparent; border: none; color: rgba(255,245,240,0.7); font-size: 18px; cursor: pointer;" on:click=move |_| show_remember.set(false)>"\u{00D7}"</button>
                        </div>
                        <textarea style="width: 100%; min-height: 100px; padding: 10px 14px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 14px; outline: none; box-sizing: border-box; resize: vertical; font-family: 'Rajdhani', sans-serif;"
                            prop:value=move || remember_text.get()
                            on:input=move |ev| remember_text.set(event_target_value(&ev))
                            placeholder="Enter a fact, note, or context to remember..."
                        />
                        <div style="margin-top: 12px;">
                            <Button primary=true on_click=Some(Callback::new(move |_| {
                                let fact = remember_text.get_untracked();
                                if fact.trim().is_empty() { return; }
                                show_remember.set(false);
                                spawn_local(async move {
                                    match api::remember(&fact).await {
                                        Ok(_) => { remember_text.set(String::new()); web_sys::console::log_1(&"Remembered!".into()); }
                                        Err(e) => web_sys::console::warn_1(&format!("Remember failed: {}", e).into()),
                                    }
                                });
                            }))>"Save to Memory"</Button>
                        </div>
                    </div>
                </div>
            </Show>

            // Search results panel
            <Show when=move || search_mode.get()>
                <Card>
                    <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 12px;">
                        <SectionTitle>"Search Results"</SectionTitle>
                        <button style="font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 1px; padding: 4px 10px; border-radius: 5px; cursor: pointer; background: transparent; border: 1px solid rgba(255,60,20,0.15); color: rgba(255,245,240,0.7);"
                            on:click=move |_| { search_mode.set(false); search_results.set(vec![]); }
                        >"CLOSE"</button>
                    </div>
                    {move || {
                        let sr = search_results.get();
                        if sr.is_empty() {
                            view! { <div style="padding: 12px; color: rgba(255,245,240,0.7); font-size: 13px;">"No results — try different keywords or add more memories."</div> }.into_any()
                        } else {
                            view! {
                                <div style="display: flex; flex-direction: column; gap: 6px; max-height: 300px; overflow-y: auto;">
                                    {sr.into_iter().map(|r| {
                                        let score = format!("{:.2}", r.score);
                                        view! {
                                            <div style="padding: 8px 10px; background: rgba(255,255,255,0.02); border-radius: 6px; border: 1px solid rgba(255,60,20,0.06);">
                                                <div style="display: flex; justify-content: space-between; margin-bottom: 4px;">
                                                    <span style="font-size: 12px; color: rgba(255,245,240,0.9);">{r.content.clone()}</span>
                                                    <span style="font-family: 'Orbitron', monospace; font-size: 9px; color: rgba(255,60,20,0.5);">{score}</span>
                                                </div>
                                                <div style="font-size: 10px; color: rgba(255,245,240,0.5);">{r.path.clone()}</div>
                                            </div>
                                        }
                                    }).collect::<Vec<_>>()}
                                </div>
                            }.into_any()
                        }
                    }}
                </Card>
            </Show>


            <div style="display: grid; grid-template-columns: 1fr 1fr; gap: 16px; margin-bottom: 16px;">
                // Workspace Files
                <Card>
                    <SectionTitle>"Workspace Files"</SectionTitle>
                    <div style="font-family: 'Orbitron', monospace; font-size: 11px; color: rgba(255,245,240,0.5); margin-bottom: 12px; letter-spacing: 1px;">
                        "~/.zeus/workspace/"
                    </div>
                    <div style="max-height: 360px; overflow-y: auto;">
                        {move || {
                            let s = search.get().to_lowercase();
                            let file_list = files.get();
                            if file_list.is_empty() {
                                view! {
                                    <div style="padding: 16px; text-align: center; color: rgba(255,245,240,0.7); font-size: 13px;">
                                        "Loading files..."
                                    </div>
                                }.into_any()
                            } else {
                                view! {
                                    <div>
                                        {file_list.into_iter()
                                            .filter(|f| s.is_empty() || f.path.to_lowercase().contains(&s))
                                            .map(|f| {
                                                let size_str = if f.size > 1024 {
                                                    format!("{:.1} KB", f.size as f64 / 1024.0)
                                                } else {
                                                    format!("{} B", f.size)
                                                };
                                                let file_ext = f.path.rsplit('.').next().unwrap_or("").to_string();
                                                view! {
                                                    <div style="display: flex; align-items: center; gap: 12px; padding: 10px 0; border-bottom: 1px solid rgba(255,60,20,0.1); cursor: pointer;">
                                                        <Icon name="memory" size=14 color="rgba(255,60,20,0.6)".to_string() />
                                                        <div style="flex: 1;">
                                                            <div style="font-size: 13px; color: rgba(255,245,240,0.9); font-weight: 500;">{f.path.clone()}</div>
                                                            <div style="font-size: 10px; color: rgba(255,245,240,0.7);">
                                                                {size_str}" • "{f.chunk_count}" chunks • Modified "{f.modified.clone()}
                                                            </div>
                                                        </div>
                                                        <Badge text=file_ext />
                                                    </div>
                                                }
                                            }).collect::<Vec<_>>()}
                                    </div>
                                }.into_any()
                            }
                        }}
                    </div>
                </Card>

                // Communities
                <Card>
                    <SectionTitle>"Knowledge Communities"</SectionTitle>
                    {move || {
                        let c = communities.get();
                        if c.is_empty() {
                            view! {
                                <div style="padding: 16px; text-align: center; color: rgba(255,245,240,0.7); font-size: 13px;">
                                    "No clusters found — add more memories to build the graph."
                                </div>
                            }.into_any()
                        } else {
                            view! {
                                <div style="display: flex; flex-direction: column; gap: 8px; max-height: 360px; overflow-y: auto;">
                                    {c.into_iter().map(|comm| {
                                        let name = comm.get("name").and_then(|v| v.as_str()).unwrap_or("Unknown").to_string();
                                        let members = comm.get("members").and_then(|v| v.as_u64()).unwrap_or(0);
                                        let desc = comm.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                        view! {
                                            <div style="padding: 10px 12px; background: rgba(255,255,255,0.02); border-radius: 8px; border: 1px solid rgba(255,60,20,0.06);">
                                                <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 4px;">
                                                    <span style="font-family: 'Orbitron', monospace; font-size: 11px; color: rgba(255,245,240,0.9); letter-spacing: 1px;">{name}</span>
                                                    <span style="font-size: 10px; color: rgba(255,245,240,0.7);">{members.to_string()}" entities"</span>
                                                </div>
                                                {if !desc.is_empty() {
                                                    view! { <div style="font-size: 11px; color: rgba(255,245,240,0.7); line-height: 1.5;">{desc}</div> }.into_any()
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

            // ── Phase 7: Knowledge Graph ──
            <Card>
                <SectionTitle>"\u{1f9e0} Knowledge Graph"</SectionTitle>
                <div style="display: flex; gap: 8px; margin-bottom: 14px;">
                    <input
                        type="text"
                        placeholder="Semantic search (e.g. 'rust deployment', 'discord integration')..."
                        prop:value=move || graph_query.get()
                        on:input=move |e| {
                            use wasm_bindgen::JsCast;
                            let val = e.target().and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok()).map(|i| i.value()).unwrap_or_default();
                            graph_query.set(val);
                        }
                        on:keydown=move |e| {
                            use wasm_bindgen::JsCast;
                            if e.unchecked_ref::<web_sys::KeyboardEvent>().key() == "Enter" {
                                let q = graph_query.get_untracked();
                                if q.trim().is_empty() { return; }
                                graph_loading.set(true);
                                graph_results.set(None);
                                spawn_local(async move {
                                    if let Ok(r) = api::search_memory_graph(&q).await { graph_results.set(Some(r)); }
                                    graph_loading.set(false);
                                });
                            }
                        }
                        style="flex: 1; padding: 8px 12px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 13px; outline: none;"
                    />
                    <input
                        type="text"
                        placeholder="Entity ID..."
                        prop:value=move || graph_entity.get()
                        on:input=move |e| {
                            use wasm_bindgen::JsCast;
                            let val = e.target().and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok()).map(|i| i.value()).unwrap_or_default();
                            graph_entity.set(val);
                        }
                        on:keydown=move |e| {
                            use wasm_bindgen::JsCast;
                            if e.unchecked_ref::<web_sys::KeyboardEvent>().key() == "Enter" {
                                let eid = graph_entity.get_untracked();
                                if eid.trim().is_empty() { return; }
                                graph_loading.set(true);
                                graph_connections.set(Vec::new());
                                spawn_local(async move {
                                    if let Ok(r) = api::fetch_memory_graph(&eid).await { graph_connections.set(r.connections); }
                                    graph_loading.set(false);
                                });
                            }
                        }
                        style="width: 160px; padding: 8px 12px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 13px; outline: none;"
                    />
                </div>
                <Show when=move || graph_loading.get()>
                    <div style="padding: 20px; text-align: center; color: rgba(255,245,240,0.3); font-size: 12px;">"Searching knowledge graph..."</div>
                </Show>
                // Semantic search results
                {move || {
                    let Some(results) = graph_results.get() else { return view! { <div/> }.into_any(); };
                    let nodes = results.get("nodes").and_then(|v| v.as_array()).cloned().unwrap_or_default();
                    let edges = results.get("edges").and_then(|v| v.as_array()).cloned().unwrap_or_default();
                    if nodes.is_empty() && edges.is_empty() {
                        return view! { <div style="padding: 12px; color: rgba(255,245,240,0.3); font-size: 12px;">"No graph results — try a broader search term."</div> }.into_any();
                    }
                    view! {
                        <div style="margin-bottom: 16px;">
                            <div style="font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 2px; color: rgba(255,245,240,0.3); margin-bottom: 8px;">
                                {format!("GRAPH SEARCH — {} nodes · {} edges", nodes.len(), edges.len())}
                            </div>
                            <div style="display: flex; flex-wrap: wrap; gap: 6px; max-height: 180px; overflow-y: auto;">
                                {nodes.into_iter().take(20).map(|node| {
                                    let id = node.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                    let label = node.get("label").or_else(|| node.get("content")).and_then(|v| v.as_str())
                                        .unwrap_or(&id).chars().take(60).collect::<String>();
                                    let ntype = node.get("type").and_then(|v| v.as_str()).unwrap_or("node").to_string();
                                    let (color, bg) = match ntype.as_str() {
                                        "entity" => ("rgba(59,130,246,0.8)", "rgba(59,130,246,0.08)"),
                                        "fact" => ("rgba(34,197,94,0.8)", "rgba(34,197,94,0.08)"),
                                        "event" => ("rgba(234,179,8,0.8)", "rgba(234,179,8,0.08)"),
                                        _ => ("rgba(255,60,20,0.7)", "rgba(255,60,20,0.07)"),
                                    };
                                    view! {
                                        <div
                                            style=format!("padding: 6px 10px; border-radius: 8px; border: 1px solid {}; background: {}; cursor: pointer;", color, bg)
                                            on:click=move |_| {
                                                let eid = id.clone();
                                                graph_entity.set(eid.clone());
                                                graph_loading.set(true);
                                                graph_connections.set(Vec::new());
                                                spawn_local(async move {
                                                    if let Ok(r) = api::fetch_memory_graph(&eid).await { graph_connections.set(r.connections); }
                                                    graph_loading.set(false);
                                                });
                                            }
                                        >
                                            <div style=format!("font-size: 8px; font-family: 'Orbitron', monospace; letter-spacing: 1px; color: {}; margin-bottom: 2px;", color)>{ntype.to_uppercase()}</div>
                                            <div style="font-size: 11px; color: rgba(255,245,240,0.8);">{label}</div>
                                        </div>
                                    }
                                }).collect::<Vec<_>>()}
                            </div>
                        </div>
                    }.into_any()
                }}
                // Entity connections
                {move || {
                    let conns = graph_connections.get();
                    if conns.is_empty() { return view! { <div/> }.into_any(); }
                    let entity = graph_entity.get();
                    view! {
                        <div>
                            <div style="font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 2px; color: rgba(255,245,240,0.3); margin-bottom: 8px;">
                                {format!("CONNECTIONS — {} → {} links", entity, conns.len())}
                            </div>
                            <div style="display: flex; flex-direction: column; gap: 4px; max-height: 220px; overflow-y: auto;">
                                {conns.into_iter().take(30).map(|conn| {
                                    let rel = conn.get("relation").or_else(|| conn.get("type")).and_then(|v| v.as_str()).unwrap_or("linked").to_string();
                                    let target = conn.get("target").or_else(|| conn.get("entity")).and_then(|v| v.as_str()).unwrap_or("").to_string();
                                    let weight = conn.get("weight").and_then(|v| v.as_f64()).unwrap_or(1.0);
                                    let bar_w = ((weight * 100.0) as u32).min(100);
                                    view! {
                                        <div style="display: flex; align-items: center; gap: 8px; padding: 6px 8px; background: rgba(255,255,255,0.02); border-radius: 6px;">
                                            <span style="font-family: 'Orbitron', monospace; font-size: 8px; color: rgba(255,60,20,0.6); min-width: 70px; text-align: right;">{rel.to_uppercase()}</span>
                                            <span style="color: rgba(255,245,240,0.5);">"→"</span>
                                            <span style="font-size: 12px; color: rgba(255,245,240,0.8); flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">{target}</span>
                                            <div style="width: 60px; height: 3px; background: rgba(255,245,240,0.07); border-radius: 2px; flex-shrink: 0;">
                                                <div style=format!("height: 100%; width: {}%; background: rgba(255,60,20,0.4); border-radius: 2px;", bar_w) />
                                            </div>
                                        </div>
                                    }
                                }).collect::<Vec<_>>()}
                            </div>
                        </div>
                    }.into_any()
                }}
                {move || {
                    if graph_results.get().is_none() && graph_connections.get().is_empty() && !graph_loading.get() {
                        view! {
                            <div style="padding: 16px; text-align: center; color: rgba(255,245,240,0.2); font-size: 12px; line-height: 1.6;">
                                "Search semantically (left box) to find knowledge nodes — click a node to explore its connections. Enter an entity ID (right box) to jump directly to its graph."
                            </div>
                        }.into_any()
                    } else { view! { <div/> }.into_any() }
                }}
            </Card>

            // Memory Timeline
            <Card>
                <SectionTitle>"Memory Timeline"</SectionTitle>
                {move || {
                    let t = timeline.get();
                    if t.is_empty() {
                        view! {
                            <div style="padding: 16px; text-align: center; color: rgba(255,245,240,0.7); font-size: 13px;">
                                "No timeline entries yet — activity will appear here as Zeus works."
                            </div>
                        }.into_any()
                    } else {
                        view! {
                            <div style="display: flex; flex-direction: column; gap: 6px; max-height: 300px; overflow-y: auto;">
                                {t.into_iter().take(25).map(|entry| {
                                    let timestamp = entry.get("timestamp").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                    let content = entry.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                    let source = entry.get("source").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                    let entry_type = entry.get("type").and_then(|v| v.as_str()).unwrap_or("memory").to_string();
                                    view! {
                                        <div style="display: flex; align-items: flex-start; gap: 10px; padding: 8px 0; border-bottom: 1px solid rgba(255,60,20,0.06);">
                                            <div style="width: 6px; height: 6px; border-radius: 50%; background: rgba(255,60,20,0.4); margin-top: 5px; flex-shrink: 0;" />
                                            <div style="flex: 1; min-width: 0;">
                                                <div style="font-size: 12px; color: rgba(255,245,240,0.9); line-height: 1.4; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">{content}</div>
                                                <div style="font-size: 10px; color: rgba(255,245,240,0.5); margin-top: 2px;">
                                                    {timestamp}
                                                    {if !source.is_empty() { format!(" • {}", source) } else { String::new() }}
                                                </div>
                                            </div>
                                            <Badge text=entry_type />
                                        </div>
                                    }
                                }).collect::<Vec<_>>()}
                            </div>
                        }.into_any()
                    }
                }}
            </Card>
        </div>
    }
}
