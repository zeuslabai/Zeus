// ═══════════════════════════════════════════════════════════
// ZEUS — Sandbox Page — Policies, Executions, Resources
// ═══════════════════════════════════════════════════════════

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;
use crate::api;
use crate::components::design::*;

#[component]
pub fn SandboxPage() -> impl IntoView {
    let policies = RwSignal::new(Vec::<api::SandboxPolicy>::new());
    let executions = RwSignal::new(Vec::<api::SandboxExecution>::new());
    let resources = RwSignal::new(api::SandboxResourceUsage::default());
    let exec_total = RwSignal::new(0u32);
    let loading = RwSignal::new(true);
    let perms = RwSignal::new(api::GlobalPerms::default());
    let allowlist = RwSignal::new(Vec::<String>::new());
    let new_command = RwSignal::new(String::new());
    let message = RwSignal::new(String::new());

    // Fetch all sandbox data on mount
    {
        spawn_local(async move {
            let (p, perm, al) = (
                api::fetch_sandbox_policies().await,
                api::fetch_permissions().await,
                api::fetch_allowlist().await,
            );
            if let Ok(p) = p { policies.set(p.policies); }
            if let Ok(p) = perm { perms.set(p.global); }
            if let Ok(a) = al { allowlist.set(a.allowlist); }
            loading.set(false);
        });
    }

    let toggle_perm = move |field: &'static str| {
        let mut p = perms.get();
        match field {
            "shell" => p.shell_access = !p.shell_access,
            "file" => p.file_write = !p.file_write,
            "web" => p.web_access = !p.web_access,
            _ => {}
        }
        perms.set(p.clone());
        spawn_local(async move {
            match api::update_permissions(&p).await {
                Ok(_) => message.set("Permissions updated".to_string()),
                Err(e) => message.set(format!("Error: {}", e)),
            }
        });
    };

    let add_command = move |_| {
        let cmd = new_command.get();
        if cmd.trim().is_empty() { return; }
        let mut list = allowlist.get();
        list.push(cmd.trim().to_string());
        allowlist.set(list.clone());
        new_command.set(String::new());
        spawn_local(async move {
            match api::update_allowlist(&list).await {
                Ok(_) => message.set("Allowlist updated".to_string()),
                Err(e) => message.set(format!("Error: {}", e)),
            }
        });
    };

    let remove_command = move |idx: usize| {
        let mut list = allowlist.get();
        if idx < list.len() {
            list.remove(idx);
            allowlist.set(list.clone());
            spawn_local(async move {
                let _ = api::update_allowlist(&list).await;
            });
        }
    };

    view! {
        <div style="padding: 32px;">
            <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 24px;">
                <div>
                    <h1 style="font-family: 'Orbitron', monospace; font-size: 14px; letter-spacing: 6px; color: rgba(255,245,240,0.9); margin: 0;">"SANDBOX"</h1>
                    <p style="color: rgba(255,245,240,0.7); font-size: 12px;">
                        {move || {
                            if loading.get() { "Loading sandbox...".to_string() }
                            else { format!("{} policies • {} executions", policies.get().len(), exec_total.get()) }
                        }}
                    </p>
                </div>
                {move || {
                    let msg = message.get();
                    (!msg.is_empty()).then(|| view! {
                        <Badge text=msg color="#22c55e".to_string() />
                    })
                }}
            </div>

            <Show when=move || !loading.get()>
                // Resource usage metrics
                <SectionTitle>"RESOURCE USAGE"</SectionTitle>
                <div style="display: flex; gap: 12px; margin-bottom: 24px; flex-wrap: wrap;">
                    {move || {
                        let r = resources.get();
                        let cpu = format!("{:.0}%", (r.cpu_current / r.cpu_limit.max(1.0)) * 100.0);
                        let mem = format!("{:.0} MB", r.memory_current_mb);
                        let disk = format!("{:.0} MB", r.disk_current_mb);
                        let kb = (r.network_bytes_in + r.network_bytes_out) / 1024;
                        let net = if kb > 1024 { format!("{:.1} MB", kb as f64 / 1024.0) } else { format!("{} KB", kb) };
                        view! {
                            <MetricCard label="CPU" value=cpu icon="cpu" />
                            <MetricCard label="MEMORY" value=mem icon="memory" />
                            <MetricCard label="DISK" value=disk icon="folder" />
                            <MetricCard label="NETWORK" value=net icon="globe" />
                        }
                    }}
                </div>

                // Permission toggles
                <SectionTitle>"PERMISSIONS"</SectionTitle>
                <div style="display: flex; gap: 12px; margin-bottom: 24px; flex-wrap: wrap;">
                    <Card>
                        <div style="display: flex; align-items: center; justify-content: space-between; gap: 16px; min-width: 200px;">
                            <div style="display: flex; align-items: center; gap: 10px;">
                                <Icon name="terminal" size=16 />
                                <span style="font-size: 12px; font-weight: 500; color: rgba(255,245,240,0.9);">"Shell Access"</span>
                            </div>
                            <button
                                on:click=move |_| toggle_perm("shell")
                                style={move || format!(
                                    "width: 40px; height: 22px; border-radius: 11px; border: none; cursor: pointer; position: relative; transition: background 0.2s; background: {};",
                                    if perms.get().shell_access { "#ff3c14" } else { "rgba(255,255,255,0.1)" }
                                )}
                            >
                                <div style={move || format!(
                                    "width: 16px; height: 16px; border-radius: 50%; background: white; position: absolute; top: 3px; transition: left 0.2s; left: {};",
                                    if perms.get().shell_access { "21px" } else { "3px" }
                                )} />
                            </button>
                        </div>
                    </Card>
                    <Card>
                        <div style="display: flex; align-items: center; justify-content: space-between; gap: 16px; min-width: 200px;">
                            <div style="display: flex; align-items: center; gap: 10px;">
                                <Icon name="file" size=16 />
                                <span style="font-size: 12px; font-weight: 500; color: rgba(255,245,240,0.9);">"File Write"</span>
                            </div>
                            <button
                                on:click=move |_| toggle_perm("file")
                                style={move || format!(
                                    "width: 40px; height: 22px; border-radius: 11px; border: none; cursor: pointer; position: relative; transition: background 0.2s; background: {};",
                                    if perms.get().file_write { "#ff3c14" } else { "rgba(255,255,255,0.1)" }
                                )}
                            >
                                <div style={move || format!(
                                    "width: 16px; height: 16px; border-radius: 50%; background: white; position: absolute; top: 3px; transition: left 0.2s; left: {};",
                                    if perms.get().file_write { "21px" } else { "3px" }
                                )} />
                            </button>
                        </div>
                    </Card>
                    <Card>
                        <div style="display: flex; align-items: center; justify-content: space-between; gap: 16px; min-width: 200px;">
                            <div style="display: flex; align-items: center; gap: 10px;">
                                <Icon name="globe" size=16 />
                                <span style="font-size: 12px; font-weight: 500; color: rgba(255,245,240,0.9);">"Web Access"</span>
                            </div>
                            <button
                                on:click=move |_| toggle_perm("web")
                                style={move || format!(
                                    "width: 40px; height: 22px; border-radius: 11px; border: none; cursor: pointer; position: relative; transition: background 0.2s; background: {};",
                                    if perms.get().web_access { "#ff3c14" } else { "rgba(255,255,255,0.1)" }
                                )}
                            >
                                <div style={move || format!(
                                    "width: 16px; height: 16px; border-radius: 50%; background: white; position: absolute; top: 3px; transition: left 0.2s; left: {};",
                                    if perms.get().web_access { "21px" } else { "3px" }
                                )} />
                            </button>
                        </div>
                    </Card>
                    <Card>
                        <div style="display: flex; align-items: center; gap: 10px; min-width: 200px;">
                            <Icon name="shield" size=16 />
                            <div>
                                <span style="font-size: 10px; color: rgba(255,245,240,0.5);">"Level"</span>
                                <div style="font-family: 'Orbitron', monospace; font-size: 12px; letter-spacing: 2px; color: rgba(255,245,240,0.9);">
                                    {move || { let l = perms.get().level; if l.is_empty() { "STANDARD".to_string() } else { l.to_uppercase() } }}
                                </div>
                            </div>
                        </div>
                    </Card>
                </div>

                // Sandbox policies
                <SectionTitle>"SANDBOX POLICIES"</SectionTitle>
                <div style="display: flex; flex-direction: column; gap: 8px; margin-bottom: 24px;">
                    {move || {
                        let pols = policies.get();
                        if pols.is_empty() {
                            vec![view! {
                                <Card>
                                    <div style="text-align: center; padding: 16px; color: rgba(255,245,240,0.5); font-size: 12px;">"No sandbox policies configured"</div>
                                </Card>
                            }.into_any()]
                        } else {
                            pols.into_iter().map(|p| {
                                let level_color = match p.sandbox_level.as_str() {
                                    "strict" => "#ef4444",
                                    "standard" => "#22c55e",
                                    _ => "#eab308",
                                };
                                view! {
                                    <Card>
                                        <div style="display: flex; align-items: flex-start; gap: 12px;">
                                            <div style={format!("width: 4px; height: 36px; border-radius: 2px; background: {}; flex-shrink: 0;", level_color)} />
                                            <div style="flex: 1;">
                                                <div style="display: flex; align-items: center; gap: 8px; margin-bottom: 4px;">
                                                    <span style="font-size: 13px; font-weight: 500; color: rgba(255,245,240,0.9);">{p.name.clone()}</span>
                                                    <Badge text={p.sandbox_level.clone()} color=level_color.to_string() />
                                                    <Badge text={format!("net: {}", p.network_access)} color="#3b82f6".to_string() />
                                                </div>
                                                <div style="display: flex; gap: 16px; font-size: 10px; color: rgba(255,245,240,0.5);">
                                                    <span>{format!("{} shell cmds", p.shell_allowlist.len())}</span>
                                                    <span>{format!("{} fs boundaries", p.filesystem_boundaries.len())}</span>
                                                    {(!p.network_allowlist.is_empty()).then(|| view! {
                                                        <span>{format!("{} net rules", p.network_allowlist.len())}</span>
                                                    })}
                                                </div>
                                            </div>
                                        </div>
                                    </Card>
                                }.into_any()
                            }).collect::<Vec<_>>()
                        }
                    }}
                </div>

                // Recent executions
                <SectionTitle>"RECENT EXECUTIONS"</SectionTitle>
                <div style="display: flex; flex-direction: column; gap: 4px; margin-bottom: 24px;">
                    {move || {
                        let execs = executions.get();
                        if execs.is_empty() {
                            vec![view! {
                                <div style="text-align: center; padding: 16px; color: rgba(255,245,240,0.5); font-size: 12px;">"No sandbox executions recorded"</div>
                            }.into_any()]
                        } else {
                            execs.into_iter().take(20).map(|e| {
                                let status_color = match e.status.as_str() {
                                    "success" | "completed" => "#22c55e",
                                    "blocked" | "denied" => "#ef4444",
                                    "running" => "#eab308",
                                    _ => "rgba(255,245,240,0.5)",
                                };
                                view! {
                                    <div style="display: flex; align-items: center; gap: 10px; padding: 8px 12px; background: rgba(255,255,255,0.03); border-radius: 4px;">
                                        <StatusDot status=e.status.clone() />
                                        <code style="flex: 1; font-family: 'Orbitron', monospace; font-size: 11px; color: rgba(255,245,240,0.5); overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">
                                            {e.command.clone()}
                                        </code>
                                        <Badge text={e.status.clone()} color=status_color.to_string() />
                                        {(e.duration_ms > 0).then(|| view! {
                                            <span style="font-size: 9px; color: rgba(255,245,240,0.5);">{format!("{}ms", e.duration_ms)}</span>
                                        })}
                                        {(e.memory_used_mb > 0).then(|| view! {
                                            <span style="font-size: 9px; color: rgba(255,245,240,0.5);">{format!("{}MB", e.memory_used_mb)}</span>
                                        })}
                                        <span style="font-size: 9px; color: rgba(255,245,240,0.5); white-space: nowrap;">{e.timestamp.clone()}</span>
                                    </div>
                                }.into_any()
                            }).collect::<Vec<_>>()
                        }
                    }}
                </div>

                // Command allowlist
                <SectionTitle>"COMMAND ALLOWLIST"</SectionTitle>
                <Card>
                    <div style="margin-bottom: 12px; display: flex; gap: 8px;">
                        <input
                            type="text"
                            placeholder="Add command pattern (e.g. git *, cargo build)"
                            prop:value=move || new_command.get()
                            on:input=move |ev| new_command.set(event_target_value(&ev))
                            on:keydown=move |ev| {
                                if ev.key() == "Enter" {
                                    let cmd = new_command.get();
                                    if !cmd.trim().is_empty() {
                                        let mut list = allowlist.get();
                                        list.push(cmd.trim().to_string());
                                        allowlist.set(list.clone());
                                        new_command.set(String::new());
                                        spawn_local(async move {
                                            let _ = api::update_allowlist(&list).await;
                                        });
                                    }
                                }
                            }
                            style="flex: 1; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 6px; padding: 8px 12px; color: rgba(255,245,240,0.9); font-family: 'Orbitron', monospace; font-size: 11px;"
                        />
                        <Button primary=true on_click=Some(Callback::new(add_command))>
                            <Icon name="plus" size=12 /> " Add"
                        </Button>
                    </div>
                    {move || {
                        let list = allowlist.get();
                        if list.is_empty() {
                            view! {
                                <div style="padding: 12px; text-align: center; color: rgba(255,245,240,0.5); font-size: 11px;">"No commands allowlisted — all require approval"</div>
                            }.into_any()
                        } else {
                            view! {
                                <div style="display: flex; flex-direction: column; gap: 3px;">
                                    {list.into_iter().enumerate().map(|(i, cmd)| {
                                        let remove = move |_| remove_command(i);
                                        view! {
                                            <div style="display: flex; align-items: center; justify-content: space-between; padding: 6px 10px; background: rgba(255,255,255,0.03); border-radius: 3px;">
                                                <code style="font-family: 'Orbitron', monospace; font-size: 10px; color: rgba(255,245,240,0.6);">{cmd}</code>
                                                <button on:click=remove style="background: none; border: none; color: #ef4444; cursor: pointer; font-size: 14px; padding: 0 4px; opacity: 0.5;">"×"</button>
                                            </div>
                                        }
                                    }).collect::<Vec<_>>()}
                                </div>
                            }.into_any()
                        }
                    }}
                    <div style="margin-top: 8px; font-size: 9px; color: rgba(255,245,240,0.5);">
                        {move || format!("{} commands allowed", allowlist.get().len())}
                    </div>
                </Card>
            </Show>
        </div>
    }
}
