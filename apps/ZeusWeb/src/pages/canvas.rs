// ═══════════════════════════════════════════════════════════
// ZEUS — Visual Canvas Builder — S21 Phase 4 P1
// Component palette + JSON spec editor + render preview
// ═══════════════════════════════════════════════════════════

use crate::api;
use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

fn render_component(comp: &api::CanvasComponent, depth: usize) -> AnyView {
    let indent = depth * 16;
    let label = comp.label.clone().unwrap_or_default();
    let content = comp.content.clone().unwrap_or_default();
    let comp_type = comp.component_type.clone();
    let type_color = match comp.component_type.as_str() {
        "button" => "rgba(255,60,20,0.7)",
        "text" | "heading" | "paragraph" => "rgba(147,197,253,0.8)",
        "input" | "textarea" | "select" => "rgba(167,243,208,0.8)",
        "card" | "container" | "box" => "rgba(234,179,8,0.7)",
        "image" => "rgba(192,132,252,0.7)",
        "list" | "table" => "rgba(251,146,60,0.7)",
        _ => "rgba(255,245,240,0.5)",
    };

    // Pre-collect children into owned AnyViews to avoid borrow issues in view!
    let child_views: Vec<AnyView> = comp.children.iter()
        .map(|c| render_component(c, depth + 1))
        .collect();
    let has_children = !child_views.is_empty();

    view! {
        <div style=format!("margin-left: {}px; margin-bottom: 6px;", indent)>
            <div style="padding: 8px 12px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.08); border-radius: 6px;">
                <div style="display: flex; align-items: center; gap: 8px; margin-bottom: 4px;">
                    <span style=format!("font-size: 10px; padding: 1px 8px; border-radius: 4px; font-weight: 700; font-family: 'Orbitron', monospace; letter-spacing: 1px; background: rgba(255,255,255,0.05); color: {};", type_color)>{comp_type.to_uppercase()}</span>
                    {(!label.is_empty()).then(|| view! { <span style="font-size: 13px; color: rgba(255,245,240,0.8); font-weight: 600;">{label}</span> })}
                </div>
                {(!content.is_empty()).then(|| view! {
                    <div style="font-size: 12px; color: rgba(255,245,240,0.6); line-height: 1.4;">{content}</div>
                })}
                {has_children.then(|| view! {
                    <div style="margin-top: 8px;">
                        {child_views}
                    </div>
                })}
            </div>
        </div>
    }.into_any()
}

const DEFAULT_SPEC: &str = r#"{
  "layout": "column",
  "components": [
    {
      "type": "heading",
      "label": "Zeus Canvas",
      "content": "Welcome to the visual canvas builder"
    },
    {
      "type": "card",
      "label": "Feature Card",
      "children": [
        {
          "type": "text",
          "content": "This card contains nested components."
        },
        {
          "type": "button",
          "label": "Get Started",
          "props": { "variant": "primary" }
        }
      ]
    },
    {
      "type": "input",
      "label": "Search",
      "props": { "placeholder": "Enter query..." }
    }
  ]
}"#;

#[component]
pub fn CanvasPage() -> impl IntoView {
    let component_types = RwSignal::new(Vec::<String>::new());
    let spec_json = RwSignal::new(DEFAULT_SPEC.to_string());
    let rendered = RwSignal::new(Option::<api::CanvasRenderResponse>::None);
    let rendering = RwSignal::new(false);
    let error = RwSignal::new(String::new());
    let active_tab = RwSignal::new(0u8); // 0=editor 1=preview

    // Load available component types
    spawn_local(async move {
        if let Ok(r) = api::fetch_canvas_components().await {
            component_types.set(r.component_types);
        }
    });

    let do_render = move |_: leptos::ev::MouseEvent| {
        let spec = spec_json.get_untracked();
        let body: serde_json::Value = match serde_json::from_str(&spec) {
            Ok(v) => v,
            Err(e) => { error.set(format!("Invalid JSON: {e}")); return; }
        };
        rendering.set(true);
        error.set(String::new());
        spawn_local(async move {
            match api::render_canvas(&body).await {
                Ok(r) => {
                    rendered.set(Some(r));
                    active_tab.set(1);
                }
                Err(e) => error.set(format!("Render error: {e}")),
            }
            rendering.set(false);
        });
    };

    view! {
        <div style="padding: 32px; max-width: 1200px; margin: 0 auto; font-family: 'Rajdhani', sans-serif; color: rgba(255,245,240,0.9);">
            // Header
            <div style="display: flex; align-items: center; justify-content: space-between; margin-bottom: 24px;">
                <div>
                    <h1 style="font-family: 'Orbitron', monospace; font-size: 20px; font-weight: 700; letter-spacing: 4px; color: rgba(255,245,240,0.9); margin: 0 0 4px 0;">"VISUAL CANVAS"</h1>
                    <p style="font-size: 13px; color: rgba(255,245,240,0.5); margin: 0;">"JSON-driven UI component builder with live preview"</p>
                </div>
                <button
                    disabled=move || rendering.get()
                    on:click=do_render
                    style="padding: 10px 24px; background: rgba(255,60,20,0.2); border: 1px solid rgba(255,60,20,0.5); border-radius: 8px; color: rgba(255,140,80,1); font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; cursor: pointer;"
                >{move || if rendering.get() { "RENDERING..." } else { "▶ RENDER" }}</button>
            </div>

            // Error
            {move || (!error.get().is_empty()).then(|| view! {
                <div style="padding: 10px 16px; background: rgba(239,68,68,0.08); border: 1px solid rgba(239,68,68,0.2); border-radius: 8px; color: rgba(239,68,68,0.8); font-size: 13px; margin-bottom: 16px; font-family: monospace;">{error.get()}</div>
            })}

            <div style="display: flex; gap: 20px; align-items: flex-start;">
                // Left: component palette + editor
                <div style="flex: 1; min-width: 0;">
                    // Tabs
                    <div style="display: flex; gap: 4px; margin-bottom: 16px; background: rgba(255,255,255,0.02); border-radius: 10px; padding: 4px; width: fit-content;">
                        {[("JSON Editor", 0u8), ("Preview", 1u8)].iter().map(|(label, idx)| {
                            let idx = *idx;
                            let label = *label;
                            view! {
                                <button
                                    on:click=move |_| active_tab.set(idx)
                                    style=move || format!(
                                        "padding: 7px 16px; border-radius: 8px; border: none; cursor: pointer; font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; background: {}; color: {};",
                                        if active_tab.get() == idx { "rgba(255,60,20,0.2)" } else { "transparent" },
                                        if active_tab.get() == idx { "rgba(255,140,80,1)" } else { "rgba(255,245,240,0.5)" },
                                    )
                                >{label}</button>
                            }
                        }).collect_view()}
                    </div>

                    // JSON Editor
                    {move || (active_tab.get() == 0).then(|| view! {
                        <div>
                            <textarea
                                rows="28"
                                prop:value=move || spec_json.get()
                                on:input=move |e| spec_json.set(event_target_value(&e))
                                spellcheck="false"
                                style="width: 100%; padding: 14px; background: rgba(0,0,0,0.4); border: 1px solid rgba(255,60,20,0.15); border-radius: 10px; color: rgba(255,245,240,0.85); font-family: 'Fira Code', 'Courier New', monospace; font-size: 13px; outline: none; box-sizing: border-box; resize: vertical; line-height: 1.6; tab-size: 2;"
                            />
                            <div style="margin-top: 8px; font-size: 11px; color: rgba(255,245,240,0.3);">"Edit the JSON spec above and click RENDER to see the preview."</div>
                        </div>
                    })}

                    // Preview
                    {move || (active_tab.get() == 1).then(|| {
                        match rendered.get() {
                            None => Some(view! {
                                <div style="display: flex; align-items: center; justify-content: center; height: 400px; color: rgba(255,245,240,0.3); font-size: 14px; border: 1px dashed rgba(255,60,20,0.1); border-radius: 12px;">
                                    "← Edit spec and click RENDER"
                                </div>
                            }.into_any()),
                            Some(resp) => Some(view! {
                                <div style="background: rgba(255,255,255,0.02); border: 1px solid rgba(255,60,20,0.12); border-radius: 12px; padding: 20px; min-height: 400px;">
                                    <div style="display: flex; align-items: center; gap: 10px; margin-bottom: 16px; padding-bottom: 12px; border-bottom: 1px solid rgba(255,60,20,0.08);">
                                        <span style="font-family: 'Orbitron', monospace; font-size: 9px; letter-spacing: 2px; color: rgba(255,60,20,0.6);">"LAYOUT:"</span>
                                        <span style="font-size: 12px; color: rgba(255,245,240,0.6);">{resp.layout.clone()}</span>
                                        <span style=format!("font-size: 10px; padding: 2px 8px; border-radius: 20px; font-weight: 600; background: rgba(255,255,255,0.05); color: {};",
                                            if resp.ok { "rgba(34,197,94,0.7)" } else { "rgba(239,68,68,0.7)" }
                                        )>{if resp.ok { "OK" } else { "ERROR" }}</span>
                                    </div>
                                    <div style=format!("display: flex; flex-direction: {};  gap: 12px;",
                                        if resp.layout == "row" { "row" } else { "column" }
                                    )>
                                    {resp.components.iter().map(|c| render_component(c, 0)).collect_view()}
                                    </div>
                                </div>
                            }.into_any()),
                        }
                    })}
                </div>

                // Right: component palette
                <div style="width: 220px; flex-shrink: 0;">
                    <div style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 2px; color: rgba(255,60,20,0.6); margin-bottom: 12px;">"COMPONENT TYPES"</div>
                    {move || {
                        let types = component_types.get();
                        if types.is_empty() {
                            view! { <div style="color: rgba(255,245,240,0.3); font-size: 12px;">"Loading..."</div> }.into_any()
                        } else {
                            view! {
                                <div style="display: flex; flex-direction: column; gap: 4px;">
                                {types.into_iter().map(|t| {
                                    let t2 = t.clone();
                                    view! {
                                        <div
                                            style="padding: 7px 12px; background: rgba(255,255,255,0.02); border: 1px solid rgba(255,60,20,0.08); border-radius: 6px; font-size: 12px; cursor: pointer; color: rgba(255,245,240,0.7); transition: all 0.15s;"
                                            on:click=move |_| {
                                                // Insert a snippet into the spec at cursor (simple append to components array)
                                                let snippet = format!(r#", {{"type": "{}", "label": "New {}", "content": ""}}"#, t2, t2);
                                                spec_json.update(|s| {
                                                    if let Some(pos) = s.rfind(']') {
                                                        s.insert_str(pos, &snippet);
                                                    }
                                                });
                                            }
                                        >
                                            <span style="font-family: monospace;">{t}</span>
                                        </div>
                                    }
                                }).collect_view()}
                                </div>
                            }.into_any()
                        }
                    }}

                    // Quick templates
                    <div style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 2px; color: rgba(255,60,20,0.6); margin-top: 20px; margin-bottom: 12px;">"TEMPLATES"</div>
                    <div style="display: flex; flex-direction: column; gap: 6px;">
                        <button
                            on:click=move |_| spec_json.set(DEFAULT_SPEC.to_string())
                            style="padding: 7px 12px; background: rgba(255,255,255,0.02); border: 1px solid rgba(255,60,20,0.08); border-radius: 6px; font-size: 11px; color: rgba(255,245,240,0.6); cursor: pointer; text-align: left;"
                        >"📋 Default layout"</button>
                        <button
                            on:click=move |_| spec_json.set(r#"{"layout":"row","components":[{"type":"card","label":"Metric 1","content":"42"},{"type":"card","label":"Metric 2","content":"128"},{"type":"card","label":"Metric 3","content":"7"}]}"#.to_string())
                            style="padding: 7px 12px; background: rgba(255,255,255,0.02); border: 1px solid rgba(255,60,20,0.08); border-radius: 6px; font-size: 11px; color: rgba(255,245,240,0.6); cursor: pointer; text-align: left;"
                        >"📊 Metrics row"</button>
                        <button
                            on:click=move |_| spec_json.set(r#"{"layout":"column","components":[{"type":"heading","label":"Form"},{"type":"input","label":"Name","props":{"placeholder":"Your name"}},{"type":"textarea","label":"Message","props":{"placeholder":"Your message"}},{"type":"button","label":"Submit","props":{"variant":"primary"}}]}"#.to_string())
                            style="padding: 7px 12px; background: rgba(255,255,255,0.02); border: 1px solid rgba(255,60,20,0.08); border-radius: 6px; font-size: 11px; color: rgba(255,245,240,0.6); cursor: pointer; text-align: left;"
                        >"📝 Simple form"</button>
                    </div>
                </div>
            </div>
        </div>
    }
}
