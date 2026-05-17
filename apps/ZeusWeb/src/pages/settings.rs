// ═══════════════════════════════════════════════════════════
// ZEUS — Settings Page — Phase 3: Config history + reload
// ═══════════════════════════════════════════════════════════

use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;
use crate::api;
use crate::components::design::*;

#[component]
pub fn SettingsPage() -> impl IntoView {
    let config = RwSignal::new(api::ConfigResponse::default());
    let keys = RwSignal::new(Vec::<api::ApiKey>::new());
    let keys_loaded = RwSignal::new(false);
    let permissions = RwSignal::new(api::PermissionsResponse::default());
    let sec_level = RwSignal::new("standard".to_string());
    let history = RwSignal::new(Vec::<serde_json::Value>::new());
    let reloading = RwSignal::new(false);
    let reload_msg = RwSignal::new(String::new());
    let providers = RwSignal::new(Vec::<api::ProviderInfo>::new());
    let selected_provider = RwSignal::new(String::new());
    let selected_model = RwSignal::new(String::new());
    let ollama_url = RwSignal::new("http://localhost:11434".to_string());
    let ollama_models = RwSignal::new(Vec::<String>::new());
    let detecting_models = RwSignal::new(false);
    let model_save_msg = RwSignal::new(String::new());

    {
        let config = config;
        let selected_provider = selected_provider;
        let selected_model = selected_model;
        let ollama_url = ollama_url;
        spawn_local(async move {
            if let Ok(c) = api::fetch_config().await {
                // Parse "provider/model" from config
                let parts: Vec<&str> = c.model.splitn(2, '/').collect();
                if parts.len() == 2 {
                    selected_provider.set(parts[0].to_string());
                    selected_model.set(parts[1].to_string());
                } else if !c.model.is_empty() {
                    selected_model.set(c.model.clone());
                }
                // Load Ollama URL from config
                if !c.ollama.url.is_empty() {
                    ollama_url.set(c.ollama.url.clone());
                }
                config.set(c);
            }
        });
    }
    {
        let keys = keys;
        let keys_loaded = keys_loaded;
        spawn_local(async move {
            if let Ok(k) = api::fetch_keys().await { keys.set(k.keys); }
            keys_loaded.set(true);
        });
    }
    {
        let permissions = permissions;
        let sec_level_s = sec_level;
        spawn_local(async move {
            if let Ok(p) = api::fetch_permissions().await {
                if !p.global.level.is_empty() { sec_level_s.set(p.global.level.clone()); }
                permissions.set(p);
            }
        });
    }
    {
        let history = history;
        spawn_local(async move {
            if let Ok(h) = api::fetch_config_history().await { history.set(h.history); }
        });
    }
    {
        let providers = providers;
        spawn_local(async move {
            if let Ok(p) = api::fetch_providers_list().await { providers.set(p.providers); }
        });
    }

    let do_reload = move |_| {
        reloading.set(true);
        reload_msg.set(String::new());
        spawn_local(async move {
            match api::reload_config().await {
                Ok(r) => reload_msg.set(r.message),
                Err(e) => reload_msg.set(format!("Error: {}", e)),
            }
            reloading.set(false);
            if let Ok(c) = api::fetch_config().await { config.set(c); }
        });
    };

    view! {
        <div style="padding: 32px;">
            <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 24px;">
                <h1 style="font-family: 'Orbitron', monospace; font-size: 14px; letter-spacing: 6px; color: rgba(255,245,240,0.9); margin: 0;">"SETTINGS"</h1>
                <div style="display: flex; gap: 8px; align-items: center;">
                    {move || if !reload_msg.get().is_empty() {
                        view! { <span style="font-size: 11px; color: rgba(255,245,240,0.7);">{reload_msg.get()}</span> }.into_any()
                    } else {
                        view! { <span /> }.into_any()
                    }}
                    <Button primary=true on_click=Some(Callback::new(do_reload))>
                        {move || if reloading.get() { "Reloading..." } else { "Reload Config" }}
                    </Button>
                </div>
            </div>
            <div style="display: grid; grid-template-columns: 1fr 1fr; gap: 16px;">
                // Model Configuration
                <Card>
                    <SectionTitle>"Model Configuration"</SectionTitle>
                    <div style="margin-bottom: 16px;">
                        <label style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,245,240,0.7); display: block; margin-bottom: 6px;">"PROVIDER"</label>
                        {move || {
                            let provs = providers.get();
                            let sel = selected_provider.get();
                            view! {
                                <select
                                    style="width: 100%; padding: 10px 12px; background: rgba(255,255,255,0.06); border: 1px solid rgba(255,60,20,0.2); border-radius: 8px; color: rgba(255,245,240,0.9); font-family: 'Rajdhani', sans-serif; font-size: 13px; outline: none; box-sizing: border-box; cursor: pointer;"
                                    on:change=move |ev| {
                                        let val = leptos::prelude::event_target_value(&ev);
                                        selected_provider.set(val.clone());
                                        model_save_msg.set(String::new());
                                        ollama_models.set(Vec::new());
                                        // Auto-select first model for this provider
                                        let provs = providers.get();
                                        if let Some(p) = provs.iter().find(|p| p.id == val)
                                            && let Some(m) = p.models.first() {
                                                selected_model.set(m.id.clone());
                                            }
                                        // Auto-detect Ollama models when selected
                                        if val == "ollama" {
                                            let url = ollama_url.get();
                                            detecting_models.set(true);
                                            spawn_local(async move {
                                                if let Ok(result) = api::test_provider_connection("ollama", None, Some(&url)).await
                                                    && result.success && !result.models.is_empty() {
                                                        selected_model.set(result.models[0].clone());
                                                        ollama_models.set(result.models);
                                                    }
                                                detecting_models.set(false);
                                            });
                                        }
                                    }
                                >
                                    {if sel.is_empty() {
                                        view! { <option value="" selected disabled>"Select a provider..."</option> }.into_any()
                                    } else {
                                        view! { <option value="" disabled>"Select a provider..."</option> }.into_any()
                                    }}
                                    {provs.into_iter().map(|p| {
                                        let id = p.id.clone();
                                        let name = format!("{} {}", p.icon, p.name);
                                        let is_sel = id == sel;
                                        view! { <option value={id} selected=is_sel>{name}</option> }
                                    }).collect::<Vec<_>>()}
                                </select>
                            }
                        }}
                    </div>
                    // Ollama URL field — only shown when Ollama is selected
                    {move || {
                        if selected_provider.get() == "ollama" {
                            view! {
                                <div style="margin-bottom: 16px;">
                                    <label style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,245,240,0.7); display: block; margin-bottom: 6px;">"OLLAMA URL"</label>
                                    <div style="display: flex; gap: 8px;">
                                        <input
                                            type="text"
                                            id="ollama-url-input"
                                            prop:value=move || ollama_url.get()
                                            on:input=move |ev| {
                                                ollama_url.set(leptos::prelude::event_target_value(&ev));
                                            }
                                            placeholder="http://localhost:11434"
                                            style="flex: 1; padding: 8px 12px; background: rgba(255,255,255,0.06); border: 1px solid rgba(255,60,20,0.2); border-radius: 8px; color: rgba(255,245,240,0.9); font-family: 'Rajdhani', sans-serif; font-size: 12px; outline: none; box-sizing: border-box;"
                                        />
                                        <button
                                            style="padding: 8px 12px; background: rgba(255,60,20,0.15); border: 1px solid rgba(255,60,20,0.4); border-radius: 8px; color: rgba(255,140,80,1); font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 1px; cursor: pointer; white-space: nowrap;"
                                            on:click=move |_| {
                                                let url = ollama_url.get();
                                                detecting_models.set(true);
                                                model_save_msg.set(String::new());
                                                spawn_local(async move {
                                                    match api::test_provider_connection("ollama", None, Some(&url)).await {
                                                        Ok(result) => {
                                                            if result.success && !result.models.is_empty() {
                                                                selected_model.set(result.models[0].clone());
                                                                ollama_models.set(result.models);
                                                                model_save_msg.set("Connected!".to_string());
                                                            } else if result.success {
                                                                model_save_msg.set("Connected but no models found".to_string());
                                                                ollama_models.set(Vec::new());
                                                            } else {
                                                                model_save_msg.set(format!("Failed: {}", result.error));
                                                                ollama_models.set(Vec::new());
                                                            }
                                                        }
                                                        Err(e) => {
                                                            model_save_msg.set(format!("Error: {}", e));
                                                            ollama_models.set(Vec::new());
                                                        }
                                                    }
                                                    detecting_models.set(false);
                                                });
                                            }
                                        >
                                            {move || if detecting_models.get() { "DETECTING..." } else { "DETECT MODELS" }}
                                        </button>
                                    </div>
                                </div>
                            }.into_any()
                        } else {
                            view! { <div /> }.into_any()
                        }
                    }}
                    <div style="margin-bottom: 16px;">
                        <label style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,245,240,0.7); display: block; margin-bottom: 6px;">"MODEL"</label>
                        {move || {
                            let sel_prov = selected_provider.get();
                            let sel_model = selected_model.get();
                            let live_ollama = ollama_models.get();

                            // Use live Ollama models if available, otherwise fall back to static list
                            if sel_prov == "ollama" && !live_ollama.is_empty() {
                                view! {
                                    <select
                                        style="width: 100%; padding: 10px 12px; background: rgba(255,255,255,0.06); border: 1px solid rgba(255,60,20,0.2); border-radius: 8px; color: rgba(255,245,240,0.9); font-family: 'Rajdhani', sans-serif; font-size: 13px; outline: none; box-sizing: border-box; cursor: pointer;"
                                        on:change=move |ev| {
                                            selected_model.set(leptos::prelude::event_target_value(&ev));
                                        }
                                    >
                                        {live_ollama.into_iter().map(|m| {
                                            let is_sel = m == sel_model;
                                            let label = m.clone();
                                            view! { <option value={m} selected=is_sel>{label}</option> }
                                        }).collect::<Vec<_>>()}
                                    </select>
                                }.into_any()
                            } else {
                                let provs = providers.get();
                                let models: Vec<api::ProviderModel> = provs.iter()
                                    .find(|p| p.id == sel_prov)
                                    .map(|p| p.models.clone())
                                    .unwrap_or_default();
                                view! {
                                    <select
                                        style="width: 100%; padding: 10px 12px; background: rgba(255,255,255,0.06); border: 1px solid rgba(255,60,20,0.2); border-radius: 8px; color: rgba(255,245,240,0.9); font-family: 'Rajdhani', sans-serif; font-size: 13px; outline: none; box-sizing: border-box; cursor: pointer;"
                                        on:change=move |ev| {
                                            selected_model.set(leptos::prelude::event_target_value(&ev));
                                        }
                                    >
                                        {models.into_iter().map(|m| {
                                            let id = m.id.clone();
                                            let label = format!("{} ({})", m.name, m.tier);
                                            let is_sel = id == sel_model;
                                            view! { <option value={id} selected=is_sel>{label}</option> }
                                        }).collect::<Vec<_>>()}
                                    </select>
                                }.into_any()
                            }
                        }}
                    </div>
                    <div style="margin-bottom: 16px; display: flex; align-items: center; gap: 12px;">
                        <Button primary=true on_click=Some(Callback::new(move |_| {
                            let prov = selected_provider.get();
                            let model = selected_model.get();
                            if prov.is_empty() || model.is_empty() { return; }
                            let model_str = format!("{}/{}", prov, model);
                            let url = ollama_url.get();
                            spawn_local(async move {
                                // Save model to config
                                let mut cfg = serde_json::json!({ "model": model_str });
                                // Also save Ollama URL if using Ollama
                                if model_str.starts_with("ollama/") {
                                    cfg["ollama"] = serde_json::json!({ "url": url });
                                }
                                let _ = api::save_config(&cfg).await;
                                if let Ok(c) = api::fetch_config().await { config.set(c); }
                                model_save_msg.set("Saved!".to_string());
                            });
                        }))>
                            "Save Model"
                        </Button>
                        {move || if !model_save_msg.get().is_empty() {
                            view! { <span style="font-size: 11px; color: #22c55e;">{model_save_msg.get()}</span> }.into_any()
                        } else {
                            view! { <span /> }.into_any()
                        }}
                    </div>
                    <div>
                        <label style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,245,240,0.7); display: block; margin-bottom: 6px;">"MAX ITERATIONS"</label>
                        {
                            let max_iter_val = RwSignal::new(String::new());
                            let max_iter_msg = RwSignal::new(String::new());
                            // Init from config once loaded
                            {
                                let max_iter_val = max_iter_val;
                                create_effect(move |_| {
                                    let c = config.get();
                                    if max_iter_val.get_untracked().is_empty() {
                                        max_iter_val.set(c.max_iterations.to_string());
                                    }
                                });
                            }
                            view! {
                                <div style="display: flex; gap: 8px; align-items: center;">
                                    <input
                                        type="number"
                                        min="1" max="100"
                                        prop:value=move || max_iter_val.get()
                                        on:input=move |ev| {
                                            max_iter_val.set(leptos::prelude::event_target_value(&ev));
                                            max_iter_msg.set(String::new());
                                        }
                                        style="flex: 1; padding: 10px 12px; background: rgba(255,255,255,0.06); border: 1px solid rgba(255,60,20,0.2); border-radius: 8px; color: rgba(255,245,240,0.9); font-family: 'Rajdhani', sans-serif; font-size: 13px; outline: none; box-sizing: border-box;"
                                    />
                                    <button
                                        style="padding: 8px 14px; background: rgba(255,60,20,0.15); border: 1px solid rgba(255,60,20,0.4); border-radius: 8px; color: rgba(255,140,80,1); font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 1px; cursor: pointer; white-space: nowrap;"
                                        on:click=move |_| {
                                            let val = max_iter_val.get();
                                            if let Ok(n) = val.parse::<u32>() {
                                                spawn_local(async move {
                                                    match api::save_config(&serde_json::json!({ "max_iterations": n })).await {
                                                        Ok(_) => {
                                                            max_iter_msg.set("Saved!".to_string());
                                                            if let Ok(c) = api::fetch_config().await { config.set(c); }
                                                        }
                                                        Err(e) => max_iter_msg.set(format!("Error: {}", e)),
                                                    }
                                                });
                                            } else {
                                                max_iter_msg.set("Invalid number".to_string());
                                            }
                                        }
                                    >"SAVE"</button>
                                    {move || if !max_iter_msg.get().is_empty() {
                                        view! { <span style="font-size: 10px; color: #22c55e;">{max_iter_msg.get()}</span> }.into_any()
                                    } else {
                                        view! { <span /> }.into_any()
                                    }}
                                </div>
                            }
                        }
                    </div>
                </Card>

                // Security
                <Card>
                    <SectionTitle>"Security"</SectionTitle>
                    <div style="margin-bottom: 16px;">
                        <label style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,245,240,0.7); display: block; margin-bottom: 6px;">"SECURITY LEVEL"</label>
                        <div style="display: flex; gap: 8px;">
                            {["minimal", "standard", "strict"].iter().map(|l| {
                                let level = l.to_string();
                                let level_c = level.clone();
                                let level_c2 = level.clone();
                                view! {
                                    <button
                                        style=move || {
                                            let base = "flex: 1; justify-content: center; font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 2px; text-transform: uppercase; padding: 4px 10px; border-radius: 6px; cursor: pointer; display: flex; align-items: center; gap: 6px; transition: all 0.3s;";
                                            if sec_level.get() == level_c {
                                                format!("{} background: rgba(255,60,20,0.15); border: 1px solid rgba(255,60,20,0.5); color: rgba(255,140,80,1);", base)
                                            } else {
                                                format!("{} background: transparent; border: 1px solid rgba(255,60,20,0.1); color: rgba(255,245,240,0.7);", base)
                                            }
                                        }
                                        on:click={ let level = level_c2.clone(); move |_| { sec_level.set(level.clone()); let lvl = level.clone(); wasm_bindgen_futures::spawn_local(async move { let _ = crate::api::save_config(&serde_json::json!({"aegis": {"sandbox_level": lvl}})).await; }); }}
                                    >
                                        {*l}
                                    </button>
                                }
                            }).collect::<Vec<_>>()}
                        </div>
                    </div>
                    <div>
                        <label style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,245,240,0.7); display: block; margin-bottom: 6px;">"GATEWAY"</label>
                        {move || {
                            let c = config.get();
                            let gw = c.gateway();
                            view! {
                                <input value=gw readonly style="width: 100%; padding: 10px 12px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 8px; color: rgba(255,245,240,0.9); font-family: 'Orbitron', monospace; font-size: 12px; outline: none; box-sizing: border-box;" />
                            }
                        }}
                    </div>
                </Card>

                // API Keys
                <Card>
                    <SectionTitle>"API Keys"</SectionTitle>
                    {move || {
                        let k = keys.get();
                        let loaded = keys_loaded.get();
                        if !loaded {
                            view! {
                                <div style="padding: 12px; color: rgba(255,245,240,0.7); font-size: 13px;">"Loading API keys..."</div>
                            }.into_any()
                        } else if k.is_empty() {
                            view! {
                                <div style="padding: 12px; color: rgba(255,245,240,0.7); font-size: 13px;">"No API keys configured. Use the fields below to set provider keys."</div>
                            }.into_any()
                        } else {
                            view! {
                                <div>
                                    {k.into_iter().map(|key| {
                                        let env_var = key.env_var.clone();
                                        let env_var_set = env_var.clone();
                                        let _env_var_clear = env_var.clone();
                                        let status_color = if key.configured { "#22c55e" } else { "#ef4444" };
                                        let status_text = if key.configured { "configured" } else { "not set" };
                                        let input_id = format!("key-{}", env_var);
                                        let input_id_set = input_id.clone();
                                        let key_saving = RwSignal::new(false);
                                        let key_msg = RwSignal::new(String::new());
                                        view! {
                                            <div style="margin-bottom: 12px;">
                                                <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 4px;">
                                                    <label style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,245,240,0.7);">{env_var.clone()}</label>
                                                    <div style="display: flex; align-items: center; gap: 6px;">
                                                        {move || if !key_msg.get().is_empty() {
                                                            view! { <span style="font-size: 10px; color: #22c55e;">{key_msg.get()}</span> }.into_any()
                                                        } else {
                                                            view! { <span style={format!("font-size: 10px; color: {};", status_color)}>{status_text}</span> }.into_any()
                                                        }}
                                                    </div>
                                                </div>
                                                <div style="display: flex; gap: 8px;">
                                                    <input
                                                        type="password"
                                                        id={input_id}
                                                        placeholder="Paste API key..."
                                                        style="flex: 1; padding: 8px 12px; background: rgba(255,255,255,0.06); border: 1px solid rgba(255,60,20,0.2); border-radius: 8px; color: rgba(255,245,240,0.9); font-family: 'Rajdhani', sans-serif; font-size: 12px; outline: none; box-sizing: border-box;"
                                                    />
                                                    <button
                                                        style="padding: 8px 16px; background: rgba(255,60,20,0.15); border: 1px solid rgba(255,60,20,0.4); border-radius: 8px; color: rgba(255,140,80,1); font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; cursor: pointer; white-space: nowrap;"
                                                        on:click={
                                                            let env_var = env_var_set.clone();
                                                            let input_id = input_id_set.clone();
                                                            move |_| {
                                                                let env_var = env_var.clone();
                                                                let input_id = input_id.clone();
                                                                if let Some(el) = leptos::prelude::document().get_element_by_id(&input_id)
                                                                    && let Ok(input) = el.dyn_into::<web_sys::HtmlInputElement>() {
                                                                        let val = input.value();
                                                                        if val.is_empty() { return; }
                                                                        key_saving.set(true);
                                                                        spawn_local(async move {
                                                                            match api::store_credential(&env_var, &val).await {
                                                                                Ok(_) => key_msg.set("saved!".to_string()),
                                                                                Err(e) => key_msg.set(format!("error: {}", e)),
                                                                            }
                                                                            key_saving.set(false);
                                                                            // Refresh keys list
                                                                            if let Ok(k) = api::fetch_keys().await { keys.set(k.keys); }
                                                                        });
                                                                    }
                                                            }
                                                        }
                                                    >
                                                        {move || if key_saving.get() { "..." } else { "SET" }}
                                                    </button>
                                                </div>
                                            </div>
                                        }
                                    }).collect::<Vec<_>>()}
                                </div>
                            }.into_any()
                        }
                    }}
                </Card>

                // Workspace Paths
                <Card>
                    <SectionTitle>"Workspace"</SectionTitle>
                    {move || {
                        let c = config.get();
                        let paths: Vec<(&'static str, String, &'static str)> = vec![
                            ("WORKSPACE PATH", if c.workspace.is_empty() { "~/.zeus/workspace".to_string() } else { c.workspace.clone() }, "workspace"),
                            ("OBSIDIAN VAULT", if c.obsidian_vault.is_empty() { "~/Obsidian/Zeus".to_string() } else { c.obsidian_vault.clone() }, "obsidian_vault"),
                            ("MNEMOSYNE DB", if c.mnemosyne_db.is_empty() { "~/.zeus/mnemosyne.db".to_string() } else { c.mnemosyne_db.clone() }, "mnemosyne_db"),
                        ];
                        paths.into_iter().map(|(label, value, config_key)| {
                            let field_val = RwSignal::new(value);
                            let field_msg = RwSignal::new(String::new());
                            view! {
                                <div style="margin-bottom: 12px;">
                                    <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 4px;">
                                        <label style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,245,240,0.7);">{label}</label>
                                        {move || if !field_msg.get().is_empty() {
                                            view! { <span style="font-size: 10px; color: #22c55e;">{field_msg.get()}</span> }.into_any()
                                        } else { view! { <span /> }.into_any() }}
                                    </div>
                                    <div style="display: flex; gap: 8px;">
                                        <input
                                            prop:value=move || field_val.get()
                                            on:input=move |ev| field_val.set(event_target_value(&ev))
                                            style="flex: 1; padding: 8px 12px; background: rgba(255,255,255,0.06); border: 1px solid rgba(255,60,20,0.2); border-radius: 8px; color: rgba(255,245,240,0.9); font-family: 'Orbitron', monospace; font-size: 11px; outline: none; box-sizing: border-box;"
                                        />
                                        <button
                                            style="padding: 8px 14px; background: rgba(255,60,20,0.15); border: 1px solid rgba(255,60,20,0.4); border-radius: 8px; color: rgba(255,140,80,1); font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; cursor: pointer; white-space: nowrap;"
                                            on:click=move |_| {
                                                let val = field_val.get();
                                                if val.is_empty() { return; }
                                                spawn_local(async move {
                                                    match api::save_config(&serde_json::json!({ config_key: val })).await {
                                                        Ok(_) => field_msg.set("saved".to_string()),
                                                        Err(e) => field_msg.set(format!("error: {}", e)),
                                                    }
                                                });
                                            }
                                        >"SET"</button>
                                    </div>
                                </div>
                            }
                        }).collect::<Vec<_>>()
                    }}
                </Card>
            </div>

            // Config History
            <Card style="margin-top: 16px;">
                <SectionTitle>"Configuration History"</SectionTitle>
                {move || {
                    let h = history.get();
                    if h.is_empty() {
                        view! {
                            <div style="padding: 16px; text-align: center; color: rgba(255,245,240,0.7); font-size: 13px;">
                                "No configuration changes recorded"
                            </div>
                        }.into_any()
                    } else {
                        view! {
                            <div style="display: flex; flex-direction: column; gap: 4px; max-height: 250px; overflow-y: auto;">
                                {h.into_iter().take(15).map(|entry| {
                                    let ts = entry.get("timestamp").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                    let field = entry.get("field").and_then(|v| v.as_str()).unwrap_or("config").to_string();
                                    let old = entry.get("old_value").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                    let new_val = entry.get("new_value").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                    let user = entry.get("user").and_then(|v| v.as_str()).unwrap_or("system").to_string();
                                    view! {
                                        <div style="display: flex; align-items: center; gap: 10px; padding: 8px 0; border-bottom: 1px solid rgba(255,60,20,0.06);">
                                            <div style="width: 6px; height: 6px; border-radius: 50%; background: rgba(255,60,20,0.4); flex-shrink: 0;" />
                                            <div style="flex: 1; min-width: 0;">
                                                <div style="display: flex; gap: 6px; align-items: center;">
                                                    <span style="font-family: 'Orbitron', monospace; font-size: 10px; color: rgba(255,245,240,0.9); letter-spacing: 1px;">{field}</span>
                                                    {if !old.is_empty() && !new_val.is_empty() {
                                                        view! {
                                                            <span style="font-size: 10px; color: rgba(255,245,240,0.5);">
                                                                {old}" → "{new_val}
                                                            </span>
                                                        }.into_any()
                                                    } else {
                                                        view! { <span /> }.into_any()
                                                    }}
                                                </div>
                                                <div style="font-size: 10px; color: rgba(255,245,240,0.5); margin-top: 2px;">
                                                    {user}" • "{ts}
                                                </div>
                                            </div>
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
