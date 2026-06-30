// ═══════════════════════════════════════════════════════════
// ZEUS — MCP Servers Page — Phase 2: Wired to API
// ═══════════════════════════════════════════════════════════

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;
use crate::api;
use crate::components::design::*;

#[component]
pub fn McpPage() -> impl IntoView {
    let servers = RwSignal::new(Vec::<api::McpServer>::new());
    let show_connect = RwSignal::new(false);
    let new_name = RwSignal::new(String::new());
    let new_transport = RwSignal::new("stdio".to_string());
    let new_command = RwSignal::new(String::new());

    {
        let servers = servers;
        spawn_local(async move { if let Ok(s) = api::fetch_mcp_servers().await { servers.set(s.servers); } });
    }

    let reload_servers = move || {
        spawn_local(async move { if let Ok(s) = api::fetch_mcp_servers().await { servers.set(s.servers); } });
    };

    let show_tools = RwSignal::new(Option::<String>::None);
    let server_tools = RwSignal::new(Vec::<api::McpTool>::new());
    let tool_loading = RwSignal::new(false);

    let view_tools = move |server_id: String| {
        show_tools.set(Some(server_id.clone()));
        tool_loading.set(true);
        server_tools.set(vec![]);
        spawn_local(async move {
            if let Ok(t) = api::fetch_mcp_tools(&server_id).await {
                server_tools.set(t.tools);
            }
            tool_loading.set(false);
        });
    };

    view! {
        // Tools overlay
        <Show when=move || show_tools.get().is_some()>
            <div style="position: fixed; inset: 0; background: rgba(0,0,0,0.78); z-index: 1000; display: flex; align-items: center; justify-content: center;">
                <div style="background: #0d0704; border: 1px solid rgba(255,60,20,0.25); border-radius: 16px; padding: 32px; width: 600px; max-width: 94vw; max-height: 70vh; overflow-y: auto;">
                    <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 16px;">
                        <div style="font-family: 'Orbitron', monospace; font-size: 11px; letter-spacing: 5px; color: rgba(255,245,240,0.9);">"SERVER TOOLS"</div>
                        <button style="background: transparent; border: none; color: rgba(255,245,240,0.7); font-size: 18px; cursor: pointer;" on:click=move |_| show_tools.set(None)>"\u{00D7}"</button>
                    </div>
                    {move || {
                        if tool_loading.get() {
                            view! { <div style="color: rgba(255,245,240,0.7); font-size: 13px;">"Loading tools..."</div> }.into_any()
                        } else {
                            let tools = server_tools.get();
                            if tools.is_empty() {
                                view! { <div style="color: rgba(255,245,240,0.7); font-size: 13px;">"No tools found"</div> }.into_any()
                            } else {
                                view! {
                                    <div style="display: flex; flex-direction: column; gap: 6px;">
                                        {tools.into_iter().map(|t| {
                                            view! {
                                                <div style="padding: 10px 14px; background: rgba(255,255,255,0.03); border-radius: 8px; border: 1px solid rgba(255,60,20,0.06);">
                                                    <div style="display: flex; justify-content: space-between; align-items: center;">
                                                        <span style="font-family: 'Orbitron', monospace; font-size: 12px; color: rgba(255,245,240,0.9);">{t.name.clone()}</span>
                                                        <Button small=true on_click=Some(Callback::new({
                                                            let tn = t.name.clone();
                                                            move |_| {
                                                                let tn = tn.clone();
                                                                spawn_local(async move {
                                                                    match api::test_mcp_tool(&tn).await {
                                                                        Ok(r) => web_sys::console::log_1(&format!("Test {}: {}", tn, r.message).into()),
                                                                        Err(e) => web_sys::console::warn_1(&format!("Test failed: {}", e).into()),
                                                                    }
                                                                });
                                                            }
                                                        }))>"Test"</Button>
                                                    </div>
                                                    <div style="font-size: 11px; color: rgba(255,245,240,0.7); margin-top: 4px;">{t.description.clone()}</div>
                                                </div>
                                            }
                                        }).collect::<Vec<_>>()}
                                    </div>
                                }.into_any()
                            }
                        }
                    }}
                </div>
            </div>
        </Show>

        // Connect Server modal
        <Show when=move || show_connect.get()>
            <div style="position: fixed; inset: 0; background: rgba(0,0,0,0.78); z-index: 1000; display: flex; align-items: center; justify-content: center;">
                <div style="background: #0d0704; border: 1px solid rgba(255,60,20,0.25); border-radius: 16px; padding: 32px; width: 480px; max-width: 92vw;">
                    <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 20px;">
                        <div style="font-family: 'Orbitron', monospace; font-size: 11px; letter-spacing: 5px; color: rgba(255,245,240,0.9);">"CONNECT MCP SERVER"</div>
                        <button style="background: transparent; border: none; color: rgba(255,245,240,0.7); font-size: 18px; cursor: pointer;" on:click=move |_| show_connect.set(false)>"\u{00D7}"</button>
                    </div>
                    <div style="display: flex; flex-direction: column; gap: 12px;">
                        <div>
                            <div style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,245,240,0.7); margin-bottom: 6px;">"SERVER NAME"</div>
                            <input style="width: 100%; padding: 10px 14px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 14px; outline: none; box-sizing: border-box;"
                                prop:value=move || new_name.get()
                                on:input=move |ev| new_name.set(event_target_value(&ev))
                                placeholder="e.g. my-server"
                            />
                        </div>
                        <div>
                            <div style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,245,240,0.7); margin-bottom: 6px;">"TRANSPORT"</div>
                            <select style="width: 100%; background: #0d0704; border: 1px solid rgba(255,60,20,0.1); border-radius: 8px; padding: 10px 14px; color: rgba(255,245,240,0.9); font-size: 14px; box-sizing: border-box;"
                                on:change=move |ev| new_transport.set(event_target_value(&ev))
                            >
                                <option value="stdio">"stdio"</option>
                                <option value="http">"HTTP/SSE"</option>
                            </select>
                        </div>
                        <div>
                            <div style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,245,240,0.7); margin-bottom: 6px;">"COMMAND / URL"</div>
                            <input style="width: 100%; padding: 10px 14px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 14px; outline: none; box-sizing: border-box;"
                                prop:value=move || new_command.get()
                                on:input=move |ev| new_command.set(event_target_value(&ev))
                                placeholder="npx @modelcontextprotocol/..."
                            />
                        </div>
                    </div>
                    <div style="display: flex; gap: 10px; margin-top: 20px; justify-content: flex-end;">
                        <Button on_click=Some(Callback::new(move |_| show_connect.set(false)))>"Cancel"</Button>
                        <Button primary=true on_click=Some(Callback::new(move |_| {
                            let name = new_name.get_untracked();
                            if name.trim().is_empty() { return; }
                            let transport = new_transport.get_untracked();
                            let command = new_command.get_untracked();
                            show_connect.set(false);
                            spawn_local(async move {
                                let req = api::ConnectMcpReq { name, transport, command };
                                match api::connect_mcp(&req).await {
                                    Ok(_) => reload_servers(),
                                    Err(e) => web_sys::console::warn_1(&format!("MCP connect failed: {}", e).into()),
                                }
                                new_name.set(String::new());
                                new_command.set(String::new());
                            });
                        }))>"Connect"</Button>
                    </div>
                </div>
            </div>
        </Show>

        <div style="padding: 32px;">
            <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 24px;">
                <div style="margin-bottom: 0;">
                    <h1 style="font-family: 'Orbitron', monospace; font-size: 14px; letter-spacing: 6px; color: rgba(255,245,240,0.9); margin: 0;">"MCP SERVERS"</h1>
                    <p style="font-family: 'Rajdhani', sans-serif; font-size: 12px; color: rgba(255,245,240,0.7); margin: 4px 0 0;">{move || {
                        let s = servers.get();
                        let connected = s.iter().filter(|srv| srv.status == "connected").count();
                        if s.is_empty() { "Loading MCP servers...".to_string() }
                        else { format!("{} servers • {} connected", s.len(), connected) }
                    }}</p>
                </div>
                <Button primary=true on_click=Some(Callback::new(move |_| show_connect.set(true)))>
                    <Icon name="plus" size=12 /> " Connect Server"
                </Button>
            </div>
            <div style="display: grid; grid-template-columns: repeat(auto-fill, minmax(320px, 1fr)); gap: 16px;">
                {move || servers.get().into_iter().map(|s| {
                    let is_connected = s.status == "connected";
                    let status_str = s.status.clone();
                    view! {
                        <Card glow=is_connected>
                            <div style="display: flex; align-items: center; gap: 12px; margin-bottom: 12px;">
                                <div style={format!("width: 40px; height: 40px; border-radius: 10px; display: flex; align-items: center; justify-content: center; background: {};",
                                    if is_connected { "rgba(255,60,20,0.15)" } else { "rgba(255,255,255,0.03)" }
                                )}>
                                    <Icon name="tools" size=18 color={if is_connected { "rgba(255,60,20,0.6)" } else { "rgba(255,245,240,0.5)" }.to_string()} />
                                </div>
                                <div style="flex: 1;">
                                    <div style="font-family: 'Orbitron', monospace; font-size: 13px; color: rgba(255,245,240,0.9); font-weight: 600;">{s.name.clone()}</div>
                                    <div style="display: flex; align-items: center; gap: 6px; margin-top: 2px;">
                                        <StatusDot status=status_str />
                                        <span style="font-size: 11px; color: rgba(255,245,240,0.7);">{s.transport.clone()}</span>
                                    </div>
                                </div>
                            </div>
                            <div style="display: grid; grid-template-columns: 1fr 1fr 1fr; gap: 8px; margin-bottom: 12px;">
                                <div style="padding: 6px; background: rgba(255,255,255,0.03); border-radius: 6px; text-align: center;">
                                    <div style="font-family: 'Orbitron', monospace; font-size: 8px; color: rgba(255,245,240,0.5); letter-spacing: 1px;">"TOOLS"</div>
                                    <div style="font-size: 13px; color: rgba(255,245,240,0.9); font-weight: 600;">{s.tools_count}</div>
                                </div>
                                <div style="padding: 6px; background: rgba(255,255,255,0.03); border-radius: 6px; text-align: center;">
                                    <div style="font-family: 'Orbitron', monospace; font-size: 8px; color: rgba(255,245,240,0.5); letter-spacing: 1px;">"LATENCY"</div>
                                    <div style="font-size: 13px; color: rgba(255,245,240,0.9); font-weight: 600;">{format!("{}ms", s.latency_ms)}</div>
                                </div>
                                <div style="padding: 6px; background: rgba(255,255,255,0.03); border-radius: 6px; text-align: center;">
                                    <div style="font-family: 'Orbitron', monospace; font-size: 8px; color: rgba(255,245,240,0.5); letter-spacing: 1px;">"ERRORS"</div>
                                    <div style="font-size: 13px; color: rgba(255,245,240,0.9); font-weight: 600;">{format!("{:.1}%", s.error_rate)}</div>
                                </div>
                            </div>
                            <div style="display: flex; justify-content: flex-end; gap: 6px;">
                                <button
                                    style="font-family: 'Orbitron', monospace; font-size: 7px; letter-spacing: 1px; padding: 4px 8px; border-radius: 5px; cursor: pointer; background: rgba(239,68,68,0.08); border: 1px solid rgba(239,68,68,0.15); color: rgba(239,68,68,0.6);"
                                    on:click={
                                        let sid = s.id.clone();
                                        move |_| {
                                            let sid = sid.clone();
                                            spawn_local(async move {
                                                if let Err(e) = api::disconnect_mcp(&sid).await { web_sys::console::error_1(&format!("API error: {}", e).into()); }
                                                reload_servers();
                                            });
                                        }
                                    }
                                >"DEL"</button>
                                <Button small=true primary=is_connected on_click=Some(Callback::new({
                                    let sid2 = s.id.clone();
                                    let sname = s.name.clone();
                                    let stransport = s.transport.clone();
                                    move |_| {
                                        if is_connected {
                                            let sid2 = sid2.clone();
                                            spawn_local(async move {
                                                if let Err(e) = api::disconnect_mcp(&sid2).await { web_sys::console::error_1(&format!("API error: {}", e).into()); }
                                                reload_servers();
                                            });
                                        } else {
                                            let sname = sname.clone();
                                            let stransport = stransport.clone();
                                            spawn_local(async move {
                                                let req = api::ConnectMcpReq { name: sname, transport: stransport, command: String::new() };
                                                if let Err(e) = api::connect_mcp(&req).await { web_sys::console::error_1(&format!("API error: {}", e).into()); }
                                                reload_servers();
                                            });
                                        }
                                    }
                                }))>
                                    {if is_connected { "Disconnect" } else { "Connect" }}
                                </Button>
                                <Button small=true on_click=Some(Callback::new({
                                    let sid3 = s.id.clone();
                                    move |_| view_tools(sid3.clone())
                                }))>"Tools"</Button>
                            </div>
                        </Card>
                    }
                }).collect::<Vec<_>>()}
            </div>
        </div>
    }
}
