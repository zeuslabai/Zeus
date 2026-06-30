// ═══════════════════════════════════════════════════════════
// ZEUS — Vector Stores Page — S21 Phase 4 P0
// List, create, search, inspect, delete vector stores
// ═══════════════════════════════════════════════════════════

use crate::api;
use crate::components::design::*;
use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

fn status_color(s: &str) -> &'static str {
    match s {
        "completed" | "ready" => "rgba(34,197,94,0.8)",
        "in_progress" | "indexing" => "rgba(234,179,8,0.8)",
        _ => "rgba(255,245,240,0.4)",
    }
}

fn fmt_ts(ts: &str) -> String {
    if ts.len() >= 10 { ts[..10].to_string() } else { ts.to_string() }
}

#[component]
pub fn VectorStoresPage() -> impl IntoView {
    let stores = RwSignal::new(Vec::<api::VectorStore>::new());
    let loading = RwSignal::new(true);
    let selected = RwSignal::new(Option::<api::VectorStore>::None);
    let files = RwSignal::new(Vec::<serde_json::Value>::new());
    let search_q = RwSignal::new(String::new());
    let search_results = RwSignal::new(Vec::<api::VectorSearchResult>::new());
    let searching = RwSignal::new(false);
    let toast = RwSignal::new(String::new());
    let toast_ok = RwSignal::new(true);
    // Create form
    let show_create = RwSignal::new(false);
    let new_name = RwSignal::new(String::new());
    let new_desc = RwSignal::new(String::new());
    let creating = RwSignal::new(false);
    // Add file
    let new_file_path = RwSignal::new(String::new());
    let adding_file = RwSignal::new(false);

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
            match api::fetch_vector_stores().await {
                Ok(r) => stores.set(r.data),
                Err(e) => {
                    toast_ok.set(false);
                    toast.set(format!("Load error: {e}"));
                }
            }
            loading.set(false);
        });
    };
    reload();

    let select_store = move |store: api::VectorStore| {
        let id = store.id.clone();
        selected.set(Some(store));
        search_results.set(vec![]);
        search_q.set(String::new());
        spawn_local(async move {
            if let Ok(r) = api::fetch_vector_store_files(&id).await {
                files.set(r.data);
            }
        });
    };

    let do_create = move |_| {
        let name = new_name.get_untracked();
        if name.is_empty() { set_toast(false, "Name required".into()); return; }
        let desc = new_desc.get_untracked();
        creating.set(true);
        spawn_local(async move {
            let body = serde_json::json!({ "name": name, "description": desc });
            match api::create_vector_store(&body).await {
                Ok(vs) => {
                    set_toast(true, format!("Created: {}", vs.name));
                    new_name.set(String::new());
                    new_desc.set(String::new());
                    show_create.set(false);
                    reload();
                }
                Err(e) => set_toast(false, format!("Error: {e}")),
            }
            creating.set(false);
        });
    };

    let do_delete = move |id: String| {
        spawn_local(async move {
            if let Ok(_) = api::delete_vector_store(&id).await {
                set_toast(true, "Deleted".into());
                if selected.get_untracked().as_ref().map(|s| s.id.as_str()) == Some(&id) {
                    selected.set(None);
                }
                reload();
            } else {
                set_toast(false, "Delete failed".into());
            }
        });
    };

    let run_search = move || {
        let q = search_q.get_untracked();
        if q.is_empty() { return; }
        let id = match selected.get_untracked() { Some(s) => s.id, None => return };
        searching.set(true);
        search_results.set(vec![]);
        spawn_local(async move {
            match api::search_vector_store(&id, &serde_json::json!({ "query": q, "top_k": 10 })).await {
                Ok(r) => search_results.set(r.data),
                Err(e) => set_toast(false, format!("Search error: {e}")),
            }
            searching.set(false);
        });
    };
    let do_search = move |_: leptos::ev::MouseEvent| run_search();
    let do_search_keydown = move |e: leptos::ev::KeyboardEvent| { if e.key() == "Enter" { run_search(); } };

    let do_add_file = move |_| {
        let path = new_file_path.get_untracked();
        if path.is_empty() { set_toast(false, "File path required".into()); return; }
        let id = match selected.get_untracked() { Some(s) => s.id, None => return };
        adding_file.set(true);
        spawn_local(async move {
            match api::add_file_to_vector_store(&id, &serde_json::json!({ "file_id": path })).await {
                Ok(_) => {
                    set_toast(true, "File added".into());
                    new_file_path.set(String::new());
                    if let Ok(r) = api::fetch_vector_store_files(&id).await { files.set(r.data); }
                }
                Err(e) => set_toast(false, format!("Error: {e}")),
            }
            adding_file.set(false);
        });
    };

    view! {
        <div style="padding: 32px; max-width: 1200px; margin: 0 auto; font-family: 'Rajdhani', sans-serif; color: rgba(255,245,240,0.9);">
            // Header
            <div style="display: flex; align-items: center; justify-content: space-between; margin-bottom: 28px;">
                <div>
                    <h1 style="font-family: 'Orbitron', monospace; font-size: 20px; font-weight: 700; letter-spacing: 4px; color: rgba(255,245,240,0.9); margin: 0 0 4px 0;">"VECTOR STORES"</h1>
                    <p style="font-size: 13px; color: rgba(255,245,240,0.5); margin: 0;">"Semantic memory namespaces — FTS5 + embedding search"</p>
                </div>
                <Button primary=true on_click=Some(Callback::new(move |_| show_create.update(|v| *v = !*v)))>
                    <Icon name="plus" size=12 /> " NEW STORE"
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
                    <div style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 2px; color: rgba(255,60,20,0.6); margin-bottom: 16px;">"NEW VECTOR STORE"</div>
                    <div style="display: flex; gap: 12px; flex-wrap: wrap;">
                        <input
                            placeholder="Store name"
                            prop:value=move || new_name.get()
                            on:input=move |e| new_name.set(event_target_value(&e))
                            style="flex: 1; min-width: 200px; padding: 10px 14px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.2); border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 14px; outline: none;"
                        />
                        <input
                            placeholder="Description (optional)"
                            prop:value=move || new_desc.get()
                            on:input=move |e| new_desc.set(event_target_value(&e))
                            style="flex: 2; min-width: 200px; padding: 10px 14px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.2); border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 14px; outline: none;"
                        />
                        <Button primary=true on_click=Some(Callback::new(move |_| do_create(())))>
                            {move || if creating.get() { "Creating..." } else { "Create" }}
                        </Button>
                    </div>
                </div>
            })}

            // Main two-pane layout
            <div style="display: flex; gap: 20px; align-items: flex-start;">
                // Left: Store list
                <div style="width: 300px; flex-shrink: 0;">
                    {move || loading.get().then(|| view! {
                        <div style="color: rgba(255,245,240,0.4); font-size: 13px; padding: 20px 0;">"Loading..."</div>
                    })}
                    {move || {
                        let vs = stores.get();
                        if !loading.get() && vs.is_empty() {
                            Some(view! {
                                <div style="text-align: center; padding: 40px 20px; color: rgba(255,245,240,0.3);">
                                    <div style="font-size: 32px; margin-bottom: 12px;">"🗄️"</div>
                                    <div style="font-size: 13px;">"No vector stores yet"</div>
                                    <div style="font-size: 11px; margin-top: 4px;">"Create one to start indexing semantic memory"</div>
                                </div>
                            }.into_any())
                        } else {
                            Some(view! {
                                <div>
                                {vs.into_iter().map(|s| {
                                    let s_clone = s.clone();
                                    let id = s.id.clone();
                                    let id_del = id.clone();
                                    let is_sel = Memo::new(move |_| {
                                        selected.get().as_ref().map(|sel| sel.id.as_str()) == Some(&id)
                                    });
                                    view! {
                                        <div
                                            style=move || format!(
                                                "padding: 14px; border-radius: 10px; cursor: pointer; margin-bottom: 8px; border: 1px solid {}; background: {};",
                                                if is_sel.get() { "rgba(255,60,20,0.4)" } else { "rgba(255,60,20,0.1)" },
                                                if is_sel.get() { "rgba(255,60,20,0.08)" } else { "rgba(255,255,255,0.02)" },
                                            )
                                            on:click={
                                                let s_clone = s_clone.clone();
                                                move |_| select_store(s_clone.clone())
                                            }
                                        >
                                            <div style="display: flex; justify-content: space-between; align-items: flex-start;">
                                                <div style="font-weight: 600; font-size: 14px; color: rgba(255,245,240,0.9);">{s.name.clone()}</div>
                                                <button
                                                    on:click=move |e| {
                                                        e.stop_propagation();
                                                        do_delete(id_del.clone());
                                                    }
                                                    style="background: none; border: none; color: rgba(239,68,68,0.5); cursor: pointer; font-size: 12px; padding: 0 0 0 8px;"
                                                >"✕"</button>
                                            </div>
                                            {s.description.as_ref().map(|d| view! {
                                                <div style="font-size: 11px; color: rgba(255,245,240,0.4); margin-top: 2px;">{d.clone()}</div>
                                            })}
                                            <div style="display: flex; gap: 8px; margin-top: 8px; flex-wrap: wrap;">
                                                <span style=move || format!(
                                                    "font-size: 10px; padding: 2px 8px; border-radius: 20px; background: rgba(255,255,255,0.05); color: {};",
                                                    status_color(&s.status)
                                                )>{s.status.clone()}</span>
                                                <span style="font-size: 10px; padding: 2px 8px; border-radius: 20px; background: rgba(255,255,255,0.05); color: rgba(255,245,240,0.5);">
                                                    {format!("{} files", s.file_counts.total)}
                                                </span>
                                                <span style="font-size: 10px; color: rgba(255,245,240,0.3);">{fmt_ts(&s.created_at)}</span>
                                            </div>
                                        </div>
                                    }
                                }).collect_view()}
                                </div>
                            }.into_any())
                        }
                    }}
                </div>

                // Right: Detail pane
                <div style="flex: 1; min-width: 0;">
                    {move || match selected.get() {
                        None => Some(view! {
                            <div style="display: flex; align-items: center; justify-content: center; height: 300px; color: rgba(255,245,240,0.3); font-size: 14px; border: 1px dashed rgba(255,60,20,0.1); border-radius: 12px;">
                                "← Select a vector store to inspect"
                            </div>
                        }.into_any()),
                        Some(store) => Some(view! {
                            <div>
                                // Store header
                                <div style="background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.15); border-radius: 12px; padding: 20px; margin-bottom: 16px;">
                                    <div style="display: flex; justify-content: space-between; align-items: flex-start; margin-bottom: 12px;">
                                        <div>
                                            <div style="font-family: 'Orbitron', monospace; font-size: 14px; font-weight: 700; color: rgba(255,245,240,0.9);">{store.name.clone()}</div>
                                            {store.description.map(|d| view! { <div style="font-size: 12px; color: rgba(255,245,240,0.5); margin-top: 2px;">{d}</div> })}
                                        </div>
                                        <span style=move || format!(
                                            "font-size: 10px; padding: 3px 10px; border-radius: 20px; font-weight: 600; background: rgba(255,255,255,0.05); color: {};",
                                            status_color(&store.status)
                                        )>{store.status.clone()}</span>
                                    </div>
                                    <div style="display: flex; gap: 20px; flex-wrap: wrap;">
                                        <div>
                                            <div style="font-size: 10px; color: rgba(255,245,240,0.4); text-transform: uppercase; letter-spacing: 1px;">"Total Files"</div>
                                            <div style="font-size: 22px; font-weight: 700; color: rgba(255,140,80,1);">{store.file_counts.total}</div>
                                        </div>
                                        <div>
                                            <div style="font-size: 10px; color: rgba(255,245,240,0.4); text-transform: uppercase; letter-spacing: 1px;">"Completed"</div>
                                            <div style="font-size: 22px; font-weight: 700; color: rgba(34,197,94,0.8);">{store.file_counts.completed}</div>
                                        </div>
                                        <div>
                                            <div style="font-size: 10px; color: rgba(255,245,240,0.4); text-transform: uppercase; letter-spacing: 1px;">"In Progress"</div>
                                            <div style="font-size: 22px; font-weight: 700; color: rgba(234,179,8,0.8);">{store.file_counts.in_progress}</div>
                                        </div>
                                        <div>
                                            <div style="font-size: 10px; color: rgba(255,245,240,0.4); text-transform: uppercase; letter-spacing: 1px;">"Created"</div>
                                            <div style="font-size: 14px; font-weight: 600; color: rgba(255,245,240,0.7);">{fmt_ts(&store.created_at)}</div>
                                        </div>
                                    </div>
                                </div>

                                // Search
                                <div style="background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.15); border-radius: 12px; padding: 20px; margin-bottom: 16px;">
                                    <div style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 2px; color: rgba(255,60,20,0.6); margin-bottom: 12px;">"SEMANTIC SEARCH"</div>
                                    <div style="display: flex; gap: 10px;">
                                        <input
                                            placeholder="Search this store..."
                                            prop:value=move || search_q.get()
                                            on:input=move |e| search_q.set(event_target_value(&e))
                                            on:keydown=do_search_keydown
                                            style="flex: 1; padding: 10px 14px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.2); border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 14px; outline: none;"
                                        />
                                        <Button primary=true on_click=Some(Callback::new(move |_| do_search(leptos::ev::MouseEvent::new("click").unwrap())))>
                                            {move || if searching.get() { "Searching..." } else { "Search" }}
                                        </Button>
                                    </div>
                                    {move || {
                                        let results = search_results.get();
                                        if !results.is_empty() { Some(view! {
                                            <div style="margin-top: 16px; display: flex; flex-direction: column; gap: 8px;">
                                            {results.into_iter().enumerate().map(|(i, r)| view! {
                                                <div style="padding: 12px 14px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 8px;">
                                                    <div style="display: flex; justify-content: space-between; margin-bottom: 6px;">
                                                        <span style="font-size: 10px; color: rgba(255,60,20,0.6);">{format!("#{} · score {:.3}", i+1, r.score)}</span>
                                                        <span style="font-size: 10px; color: rgba(255,245,240,0.4);">{r.memory_type.clone()}</span>
                                                    </div>
                                                    <div style="font-size: 13px; color: rgba(255,245,240,0.8); white-space: pre-wrap; line-height: 1.5;">{r.content}</div>
                                                </div>
                                            }).collect_view()}
                                            </div>
                                        })} else { None }
                                    }}
                                </div>

                                // Add file
                                <div style="background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.15); border-radius: 12px; padding: 20px; margin-bottom: 16px;">
                                    <div style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 2px; color: rgba(255,60,20,0.6); margin-bottom: 12px;">"ADD FILE"</div>
                                    <div style="display: flex; gap: 10px;">
                                        <input
                                            placeholder="File ID or upload path"
                                            prop:value=move || new_file_path.get()
                                            on:input=move |e| new_file_path.set(event_target_value(&e))
                                            style="flex: 1; padding: 10px 14px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.2); border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 14px; outline: none;"
                                        />
                                        <Button primary=true on_click=Some(Callback::new(move |_| do_add_file(())))>
                                            {move || if adding_file.get() { "Adding..." } else { "Add File" }}
                                        </Button>
                                    </div>
                                </div>

                                // Files list
                                <div style="background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.15); border-radius: 12px; padding: 20px;">
                                    <div style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 2px; color: rgba(255,60,20,0.6); margin-bottom: 12px;">"FILES IN STORE"</div>
                                    {move || {
                                        let fs = files.get();
                                        if fs.is_empty() {
                                            Some(view! { <div style="color: rgba(255,245,240,0.3); font-size: 13px;">"No files indexed yet."</div> }.into_any())
                                        } else {
                                            Some(view! {
                                                <div style="display: flex; flex-direction: column; gap: 6px;">
                                                {fs.into_iter().map(|f| {
                                                    let id = f.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                                    let status = f.get("status").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                                    let status2 = status.clone();
                                                    let source = f.get("source").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                                    view! {
                                                        <div style="display: flex; align-items: center; gap: 12px; padding: 8px 12px; background: rgba(255,255,255,0.02); border-radius: 6px; font-size: 12px;">
                                                            <span style="color: rgba(255,245,240,0.5); font-family: monospace; flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">{id}</span>
                                                            {(!source.is_empty()).then(|| view! { <span style="color: rgba(255,245,240,0.4);">{source}</span> })}
                                                            <span style=move || format!("color: {};", status_color(&status2))>{status}</span>
                                                        </div>
                                                    }
                                                }).collect_view()}
                                                </div>
                                            }.into_any())
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
