// ═══════════════════════════════════════════════════════════
// ZEUS — Tools Page — Phase 3: Schema-aware execution
// ═══════════════════════════════════════════════════════════

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;
use crate::api;
use crate::components::design::*;

/// Extract parameter names from a tool's JSON Schema `parameters` field.
fn schema_param_names(params: &serde_json::Value) -> Vec<(String, bool)> {
    let props = params
        .get("properties")
        .or_else(|| params.get("items").and_then(|i| i.get("properties")))
        .and_then(|p| p.as_object());
    let required: Vec<String> = params
        .get("required")
        .and_then(|r| r.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    match props {
        Some(map) => map.keys().map(|k| (k.clone(), required.contains(k))).collect(),
        None => Vec::new(),
    }
}

#[component]
pub fn ToolsPage() -> impl IntoView {
    let search = RwSignal::new(String::new());
    let filter = RwSignal::new("all".to_string());
    let tools = RwSignal::new(Vec::<api::ToolDef>::new());

    // Detail panel state
    let selected_tool = RwSignal::new(Option::<api::ToolDef>::None);
    let param_values = RwSignal::new(Vec::<(String, RwSignal<String>)>::new());
    let exec_result = RwSignal::new(Option::<Result<api::ToolExecResponse, String>>::None);
    let executing = RwSignal::new(false);

    {
        spawn_local(async move {
            if let Ok(t) = api::get_tools().await {
                tools.set(t.tools);
            }
        });
    }

    let open_tool = move |tool: api::ToolDef| {
        let params = schema_param_names(&tool.parameters);
        let signals: Vec<(String, RwSignal<String>)> = params
            .into_iter()
            .map(|(name, _)| (name, RwSignal::new(String::new())))
            .collect();
        param_values.set(signals);
        exec_result.set(None);
        executing.set(false);
        selected_tool.set(Some(tool));
    };

    let close_detail = move |_| {
        selected_tool.set(None);
    };

    let run_tool = move |_| {
        let tool = match selected_tool.get() {
            Some(t) => t,
            None => return,
        };
        executing.set(true);
        exec_result.set(None);
        let mut args = serde_json::Map::new();
        for (name, sig) in param_values.get().iter() {
            let val = sig.get();
            if !val.is_empty() {
                // Try to parse as JSON value (number, bool, object), fall back to string
                let json_val = serde_json::from_str::<serde_json::Value>(&val)
                    .unwrap_or(serde_json::Value::String(val));
                args.insert(name.clone(), json_val);
            }
        }
        let args = serde_json::Value::Object(args);
        let tool_name = tool.name.clone();
        spawn_local(async move {
            let result = api::execute_tool(&tool_name, &args).await;
            exec_result.set(Some(result));
            executing.set(false);
        });
    };

    // Derive category counts reactively
    let category_counts = move || {
        let all = tools.get();
        let total = all.len();
        let core = all.iter().filter(|t| t.category == "filesystem" || t.category == "shell" || t.category == "web" || t.category == "agent").count();
        let browser = all.iter().filter(|t| t.category == "browser" || t.category == "safari").count();
        let talos = total.saturating_sub(core + browser);
        (total, core, talos, browser)
    };

    view! {
        <div style="padding: 32px;">
            <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 24px;">
                <div>
                    <h1 style="font-family: 'Orbitron', monospace; font-size: 14px; letter-spacing: 6px; color: rgba(255,245,240,0.9); margin: 0;">"TOOL CATALOG"</h1>
                    <p style="font-size: 12px; color: rgba(255,245,240,0.7); margin: 4px 0 0;">{move || {
                        let t = tools.get();
                        if t.is_empty() { "Loading tools...".to_string() }
                        else {
                            let mut cats = std::collections::HashSet::new();
                            for tool in t.iter() { cats.insert(tool.category.clone()); }
                            format!("{} tools across {} categories", t.len(), cats.len())
                        }
                    }}</p>
                </div>
            </div>

            // ── Detail panel overlay ──
            {move || {
                let tool = selected_tool.get()?;
                let params = schema_param_names(&tool.parameters);
                let has_params = !params.is_empty();
                let required_params: Vec<String> = params.iter().filter(|(_, r)| *r).map(|(n, _)| n.clone()).collect();
                Some(view! {
                    <Card style="margin-bottom: 24px; border: 1px solid rgba(255,60,20,0.3);">
                        <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 16px;">
                            <div>
                                <div style="font-family: 'Orbitron', monospace; font-size: 13px; letter-spacing: 2px; color: rgba(255,245,240,0.9);">{tool.name.clone()}</div>
                                <div style="font-size: 12px; color: rgba(255,245,240,0.5); margin-top: 2px;">{tool.description.clone()}</div>
                            </div>
                            <div style="display: flex; gap: 8px; align-items: center;">
                                <Badge text={tool.category.clone()} />
                                <button
                                    on:click=close_detail
                                    style="background: none; border: 1px solid rgba(255,60,20,0.2); color: rgba(255,245,240,0.5); cursor: pointer; padding: 4px 12px; border-radius: 4px; font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px;"
                                >"CLOSE"</button>
                            </div>
                        </div>

                        // Parameter inputs
                        {has_params.then(|| view! {
                            <div style="margin-bottom: 16px;">
                                <div style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,245,240,0.5); margin-bottom: 8px;">"PARAMETERS"</div>
                                <div style="display: flex; flex-direction: column; gap: 8px;">
                                    {param_values.get().into_iter().map(|(name, sig)| {
                                        let is_required = required_params.contains(&name);
                                        let label = if is_required { format!("{} *", name) } else { name.clone() };
                                        view! {
                                            <div style="display: flex; align-items: center; gap: 10px;">
                                                <label style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 1px; color: rgba(255,245,240,0.7); min-width: 120px;">{label}</label>
                                                <input
                                                    type="text"
                                                    prop:value=move || sig.get()
                                                    on:input=move |ev| sig.set(event_target_value(&ev))
                                                    placeholder="value"
                                                    style="flex: 1; padding: 6px 10px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.15); border-radius: 4px; color: rgba(255,245,240,0.9); font-size: 12px; font-family: 'Rajdhani', sans-serif; outline: none;"
                                                />
                                            </div>
                                        }
                                    }).collect::<Vec<_>>()}
                                </div>
                            </div>
                        })}

                        // Execute button
                        <div style="display: flex; align-items: center; gap: 12px;">
                            <Button primary=true on_click=Some(Callback::new(run_tool))>
                                {move || if executing.get() { "EXECUTING..." } else { "EXECUTE" }}
                            </Button>
                            {move || {
                                let tool = selected_tool.get();
                                tool.map(|t| view! {
                                    <span style="font-size: 10px; color: rgba(255,245,240,0.4);">{format!("{} previous executions", t.calls)}</span>
                                })
                            }}
                        </div>

                        // Result display
                        {move || {
                            let result = exec_result.get()?;
                            Some(view! {
                                <div style="margin-top: 16px; padding: 12px; background: rgba(255,255,255,0.02); border-radius: 6px; border: 1px solid rgba(255,60,20,0.1);">
                                    <div style="display: flex; align-items: center; gap: 8px; margin-bottom: 8px;">
                                        <div style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,245,240,0.5);">"RESULT"</div>
                                        {match &result {
                                            Ok(r) => view! {
                                                <Badge text={if r.success { "SUCCESS" } else { "ERROR" }.to_string()}
                                                    color={if r.success { "#22c55e" } else { "#ef4444" }.to_string()} />
                                            }.into_any(),
                                            Err(_) => view! {
                                                <Badge text="FAILED".to_string() color="#ef4444".to_string() />
                                            }.into_any(),
                                        }}
                                    </div>
                                    <pre style="font-size: 11px; color: rgba(255,245,240,0.7); white-space: pre-wrap; word-break: break-word; margin: 0; max-height: 300px; overflow-y: auto; font-family: monospace;">
                                        {match &result {
                                            Ok(r) => r.output.clone(),
                                            Err(e) => e.clone(),
                                        }}
                                    </pre>
                                </div>
                            })
                        }}
                    </Card>
                })
            }}

            <SearchBar placeholder="Search tools..." value=search />
            <div style="display: flex; gap: 6px; margin-bottom: 20px;">
                {move || {
                    let (total, core, talos, browser) = category_counts();
                    let cats: Vec<(&str, String)> = vec![
                        ("all", format!("All ({})", total)),
                        ("Core", format!("Core ({})", core)),
                        ("Talos", format!("Talos ({})", talos)),
                        ("Browser", format!("Browser ({})", browser)),
                    ];
                    cats.into_iter().map(|(key, label)| {
                        let k = key.to_string();
                        let k2 = k.clone();
                        let k3 = k2.clone();
                        view! {
                            <button
                                style=move || format!(
                                    "font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 2px; text-transform: uppercase; padding: 4px 10px; border-radius: 6px; cursor: pointer; display: flex; align-items: center; gap: 6px; transition: all 0.3s; background: {}; border: 1px solid {}; color: {};",
                                    if filter.get() == k3 { "rgba(255,60,20,0.15)" } else { "transparent" },
                                    if filter.get() == k3 { "rgba(255,60,20,0.5)" } else { "rgba(255,60,20,0.1)" },
                                    if filter.get() == k3 { "rgba(255,140,80,1)" } else { "rgba(255,245,240,0.7)" }
                                )
                                on:click={ let k = k.clone(); move |_| filter.set(k.clone()) }
                            >
                                {label}
                            </button>
                        }
                    }).collect::<Vec<_>>()
                }}
            </div>
            <div style="display: grid; grid-template-columns: repeat(auto-fill, minmax(300px, 1fr)); gap: 12px;">
                {move || {
                    let f = filter.get();
                    let s = search.get().to_lowercase();
                    tools.get().into_iter()
                        .filter(|t| {
                            let in_filter = match f.as_str() {
                                "all" => true,
                                "Core" => t.category == "filesystem" || t.category == "shell" || t.category == "web" || t.category == "agent",
                                "Browser" => t.category == "browser" || t.category == "safari",
                                "Talos" => {
                                    let cat = t.category.as_str();
                                    cat != "filesystem" && cat != "shell" && cat != "web" && cat != "agent" && cat != "browser" && cat != "safari"
                                },
                                _ => true,
                            };
                            in_filter && (s.is_empty() || t.name.to_lowercase().contains(&s) || t.description.to_lowercase().contains(&s))
                        })
                        .map(|t| {
                            let cat = t.category.clone();
                            let tool_for_click = t.clone();
                            view! {
                                <Card style="cursor: pointer;">
                                    <div
                                        on:click=move |_| open_tool(tool_for_click.clone())
                                        style="display: flex; flex-direction: column; gap: 8px;"
                                    >
                                        <div style="display: flex; justify-content: space-between; align-items: flex-start;">
                                            <div style="font-family: 'Orbitron', monospace; font-size: 13px; color: rgba(255,245,240,0.9); font-weight: 600;">{t.name.clone()}</div>
                                            <Badge text=cat />
                                        </div>
                                        <div style="font-size: 12px; color: rgba(255,245,240,0.7);">
                                            {if t.description.is_empty() { "No description".to_string() } else { t.description.clone() }}
                                        </div>
                                        <div style="display: flex; justify-content: space-between; align-items: center;">
                                            <span style="font-size: 10px; color: rgba(255,245,240,0.5);">{t.calls.to_string()}" executions"</span>
                                            {
                                                let param_count = schema_param_names(&t.parameters).len();
                                                (param_count > 0).then(|| view! {
                                                    <span style="font-size: 10px; color: rgba(255,245,240,0.4);">{format!("{} params", param_count)}</span>
                                                })
                                            }
                                        </div>
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
