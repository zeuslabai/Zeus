// ═══════════════════════════════════════════════════════════
// ZEUS — Extensions (Skills) Page — Wired to API
// ═══════════════════════════════════════════════════════════

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;
use crate::api;
use crate::components::design::*;

#[component]
pub fn ExtensionsPage() -> impl IntoView {
    let skills = RwSignal::new(Vec::<api::Skill>::new());
    let search = RwSignal::new(String::new());
    let show_install = RwSignal::new(false);
    let inst_name = RwSignal::new(String::new());
    let inst_content = RwSignal::new(String::new());
    let toast = RwSignal::new(Option::<(bool, String)>::None);
    let installing = RwSignal::new(false);
    let install_msg = RwSignal::new(String::new());
    let loaded = RwSignal::new(false);

    {
        let skills = skills;
        spawn_local(async move {
            if let Ok(s) = api::fetch_skills().await { skills.set(s.skills); }
            loaded.set(true);
        });
    }

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
                }
                Err(e) => install_msg.set(format!("Error: {}", e)),
            }
            installing.set(false);
        });
    };

    view! {
        // Install modal
        <Show when=move || show_install.get()>
            <div style="position: fixed; inset: 0; background: rgba(0,0,0,0.78); z-index: 1000; display: flex; align-items: center; justify-content: center;">
                <div style="background: #0d0704; border: 1px solid rgba(255,60,20,0.25); border-radius: 16px; padding: 32px; width: 480px; max-width: 92vw; box-shadow: 0 0 60px rgba(255,60,20,0.15);">
                    <div style="font-family: 'Orbitron', monospace; font-size: 11px; letter-spacing: 5px; color: rgba(255,245,240,0.9); margin-bottom: 20px;">"INSTALL EXTENSION"</div>
                    <div style="display: flex; flex-direction: column; gap: 14px;">
                        <div>
                            <div style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,245,240,0.7); margin-bottom: 6px;">"NAME *"</div>
                            <input type="text" placeholder="Extension name" style="width: 100%; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 8px; padding: 10px 14px; color: rgba(255,245,240,0.9); font-family: 'Rajdhani', sans-serif; font-size: 14px; box-sizing: border-box; outline: none;"
                                prop:value=move || inst_name.get()
                                on:input=move |ev| inst_name.set(event_target_value(&ev))
                            />
                        </div>
                        <div>
                            <div style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,245,240,0.7); margin-bottom: 6px;">"CONTENT (OPTIONAL)"</div>
                            <textarea rows=4 placeholder="Extension script or configuration..." style="width: 100%; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 8px; padding: 10px 14px; color: rgba(255,245,240,0.9); font-family: monospace; font-size: 12px; box-sizing: border-box; outline: none; resize: vertical;"
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

        <div style="padding: 32px;">
            // Error/success toast
            {move || toast.get().map(|(ok, msg)| view! {
                <div style=format!(
                    "background: {}; border: 1px solid {}; border-radius: 8px; padding: 10px 16px; margin-bottom: 16px; font-family: 'Rajdhani', sans-serif; font-size: 13px; color: {};",
                    if ok { "rgba(34,197,94,0.1)" } else { "rgba(255,60,20,0.1)" },
                    if ok { "rgba(34,197,94,0.3)" } else { "rgba(255,60,20,0.3)" },
                    if ok { "rgba(34,197,94,0.9)" } else { "rgba(255,180,160,0.9)" },
                )
                    on:click=move |_| toast.set(None)
                >
                    {msg}
                </div>
            })}
            <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 24px;">
                <div>
                    <h1 style="font-family: 'Orbitron', monospace; font-size: 14px; letter-spacing: 6px; color: rgba(255,245,240,0.9); margin: 0;">"EXTENSIONS"</h1>
                    <p style="font-size: 12px; color: rgba(255,245,240,0.7); margin: 4px 0 0;">
                        {move || {
                            let s = skills.get();
                            let enabled = s.iter().filter(|sk| sk.enabled).count();
                            if !loaded.get() { "Loading extensions...".to_string() }
                            else if s.is_empty() { "No extensions installed".to_string() }
                            else { format!("{} installed • {} enabled", s.len(), enabled) }
                        }}
                    </p>
                    {move || (!loaded.get()).then(|| view! {
                        <p style="font-size: 11px; color: rgba(255,245,240,0.4); margin: 2px 0 0; font-family: 'Orbitron', monospace; letter-spacing: 1px;">"LOADING..."</p>
                    })}
                </div>
                <Button primary=true on_click=Some(Callback::new(move |_| show_install.set(true)))>
                    <Icon name="plus" size=12 /> " Install"
                </Button>
            </div>
            <SearchBar placeholder="Search extensions..." value=search />
            // Empty state
            <Show when=move || loaded.get() && skills.get().is_empty()>
                <div style="display: flex; flex-direction: column; align-items: center; justify-content: center; padding: 80px 32px; gap: 16px; text-align: center;">
                    <div style="font-size: 40px; opacity: 0.3;">"⚙"</div>
                    <div style="font-family: 'Orbitron', monospace; font-size: 11px; letter-spacing: 4px; color: rgba(255,245,240,0.5);">"NO EXTENSIONS INSTALLED"</div>
                    <div style="font-size: 13px; color: rgba(255,245,240,0.4); max-width: 320px; line-height: 1.6;">"Extensions add new capabilities to Zeus. Install your first extension to get started."</div>
                </div>
            </Show>
            // Extensions grid
            <div style="display: grid; grid-template-columns: repeat(auto-fill, minmax(300px, 1fr)); gap: 12px;">
                {move || {
                    let q = search.get().to_lowercase();
                    skills.get().into_iter()
                        .filter(|s| q.is_empty() || s.name.to_lowercase().contains(&q) || s.description.to_lowercase().contains(&q))
                        .map(|sk| {
                            let id = sk.id.clone();
                            let currently_enabled = sk.enabled;
                            let skills_sig = skills;
                            view! {
                                <Card>
                                    <div style="display: flex; align-items: flex-start; gap: 12px;">
                                        <div style={format!("width: 40px; height: 40px; border-radius: 10px; background: {}; display: flex; align-items: center; justify-content: center; flex-shrink: 0;",
                                            if sk.enabled { "rgba(34,197,94,0.15)" } else { "rgba(255,245,240,0.03)" }
                                        )}>
                                            <Icon name="skills" size=18 color={if sk.enabled { "rgba(34,197,94,0.7)".to_string() } else { "rgba(255,245,240,0.5)".to_string() }} />
                                        </div>
                                        <div style="flex: 1; min-width: 0;">
                                            <div style="display: flex; align-items: center; gap: 8px; margin-bottom: 4px;">
                                                <span style="font-size: 14px; color: rgba(255,245,240,0.9); font-weight: 600;">{sk.name.clone()}</span>
                                                <Badge text={if sk.enabled { "ENABLED" } else { "DISABLED" }}
                                                    color={if sk.enabled { "rgba(34,197,94,0.7)".to_string() } else { "rgba(255,245,240,0.5)".to_string() }} />
                                            </div>
                                            <div style="font-size: 12px; color: rgba(255,245,240,0.7); margin-bottom: 6px; line-height: 1.4;">{sk.description.clone()}</div>
                                            <div style="display: flex; gap: 6px; flex-wrap: wrap;">
                                                {(!sk.version.is_empty()).then(|| view! {
                                                    <Badge text={format!("v{}", sk.version)} />
                                                })}
                                                {sk.author.as_ref().filter(|a| !a.is_empty()).map(|author| view! {
                                                    <span style="font-size: 10px; color: rgba(255,245,240,0.5);">"by "{author.clone()}</span>
                                                })}
                                            </div>
                                        </div>
                                        <button
                                            on:click=move |_| {
                                                let id = id.clone();
                                                let skills_sig = skills_sig;
                                                let new_enabled = !currently_enabled;
                                                spawn_local(async move {
                                                    if let Err(e) = api::toggle_skill(&id, new_enabled).await {
                                                        toast.set(Some((false, format!("Toggle failed: {}", e))));
                                                    }
                                                    if let Ok(s) = api::fetch_skills().await { skills_sig.set(s.skills); }
                                                });
                                            }
                                            style={format!("padding: 6px 12px; border-radius: 6px; border: 1px solid {}; background: {}; color: {}; font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 2px; cursor: pointer;",
                                                if sk.enabled { "rgba(239,68,68,0.3)" } else { "rgba(34,197,94,0.3)" },
                                                if sk.enabled { "rgba(239,68,68,0.1)" } else { "rgba(34,197,94,0.1)" },
                                                if sk.enabled { "rgba(239,68,68,0.8)" } else { "rgba(34,197,94,0.8)" },
                                            )}
                                        >
                                            {if sk.enabled { "DISABLE" } else { "ENABLE" }}
                                        </button>
                                        <button
                                            on:click={
                                                let start_id = sk.id.clone();
                                                move |_| {
                                                    let start_id = start_id.clone();
                                                    let skills_sig = skills_sig;
                                                    spawn_local(async move {
                                                        if let Err(e) = api::start_extension(&start_id).await {
                                                            toast.set(Some((false, format!("Start failed: {}", e))));
                                                        }
                                                        if let Ok(s) = api::fetch_skills().await { skills_sig.set(s.skills); }
                                                    });
                                                }
                                            }
                                            style="padding: 6px 10px; border-radius: 6px; border: 1px solid rgba(59,130,246,0.25); background: rgba(59,130,246,0.08); color: rgba(59,130,246,0.7); font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 1px; cursor: pointer;"
                                        >"▶"</button>
                                        <button
                                            on:click={
                                                let stop_id = sk.id.clone();
                                                move |_| {
                                                    let stop_id = stop_id.clone();
                                                    let skills_sig = skills_sig;
                                                    spawn_local(async move {
                                                        if let Err(e) = api::stop_extension(&stop_id).await {
                                                            toast.set(Some((false, format!("Stop failed: {}", e))));
                                                        }
                                                        if let Ok(s) = api::fetch_skills().await { skills_sig.set(s.skills); }
                                                    });
                                                }
                                            }
                                            style="padding: 6px 10px; border-radius: 6px; border: 1px solid rgba(234,179,8,0.25); background: rgba(234,179,8,0.08); color: rgba(234,179,8,0.7); font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 1px; cursor: pointer;"
                                        >"■"</button>
                                        <button
                                            on:click={
                                                let del_id = sk.id.clone();
                                                move |_| {
                                                    let del_id = del_id.clone();
                                                    let skills_sig = skills;
                                                    spawn_local(async move {
                                                        if let Err(e) = api::delete_skill(&del_id).await {
                                                            toast.set(Some((false, format!("Delete failed: {}", e))));
                                                        }
                                                        if let Ok(s) = api::fetch_skills().await { skills_sig.set(s.skills); }
                                                    });
                                                }
                                            }
                                            style="padding: 6px 10px; border-radius: 6px; border: 1px solid rgba(239,68,68,0.15); background: rgba(239,68,68,0.06); color: rgba(239,68,68,0.5); font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 1px; cursor: pointer;"
                                        >"DEL"</button>
                                    </div>
                                </Card>
                            }
                        })
                        .collect::<Vec<_>>()
                }}
            </div>
        </div>
    }
}
