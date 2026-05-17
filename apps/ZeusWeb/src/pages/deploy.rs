// ═══════════════════════════════════════════════════════════
// ZEUS — Deploy Page — Phase 4: One-Click Deploy
// Deploy target config + deployment history + rollback
// ═══════════════════════════════════════════════════════════

use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;
use crate::api;
use crate::components::design::*;

const TARGET_TYPES: &[(&str, &str, &str)] = &[
    ("vercel", "Vercel", "\u{25b2}"),
    ("netlify", "Netlify", "\u{1f310}"),
    ("docker", "Docker", "\u{1f433}"),
    ("vibesaas", "VibeSaas", "\u{26a1}"),
    ("self_hosted", "Self-Hosted", "\u{1f5a5}\u{fe0f}"),
];

fn status_color(status: &str) -> &'static str {
    match status {
        "live" => "rgba(34,197,94,0.9)",
        "building" | "deploying" => "rgba(234,179,8,0.9)",
        "failed" => "rgba(239,68,68,0.9)",
        "rolled_back" => "rgba(168,85,247,0.9)",
        "active" => "rgba(34,197,94,0.9)",
        "inactive" => "rgba(255,245,240,0.35)",
        "error" => "rgba(239,68,68,0.9)",
        _ => "rgba(255,245,240,0.35)",
    }
}

fn status_bg(status: &str) -> &'static str {
    match status {
        "live" => "rgba(34,197,94,0.12)",
        "building" | "deploying" => "rgba(234,179,8,0.12)",
        "failed" => "rgba(239,68,68,0.12)",
        "rolled_back" => "rgba(168,85,247,0.12)",
        _ => "rgba(255,245,240,0.06)",
    }
}

fn target_icon(target_type: &str) -> &'static str {
    TARGET_TYPES.iter()
        .find(|(t, _, _)| *t == target_type)
        .map(|(_, _, icon)| *icon)
        .unwrap_or("\u{1f680}")
}

#[component]
pub fn DeployPage() -> impl IntoView {
    let targets = RwSignal::new(Vec::<api::DeployTarget>::new());
    let deployments = RwSignal::new(Vec::<api::Deployment>::new());
    let loading = RwSignal::new(true);
    let templates = RwSignal::new(Vec::<api::MarketplaceListing>::new());
    let tab = RwSignal::new("targets".to_string()); // targets | history | templates
    let toast_msg = RwSignal::new(String::new());
    let toast_ok = RwSignal::new(true);

    // Add target form
    let show_add = RwSignal::new(false);
    let new_name = RwSignal::new(String::new());
    let new_type = RwSignal::new("vercel".to_string());
    let new_api_key = RwSignal::new(String::new());
    let new_project_id = RwSignal::new(String::new());
    let new_team_id = RwSignal::new(String::new());
    let new_region = RwSignal::new(String::new());
    let new_domain = RwSignal::new(String::new());
    let saving = RwSignal::new(false);

    // Rollback state
    let rolling_back = RwSignal::new(Option::<String>::None);

    // Load data
    let reload = move || {
        spawn_local(async move {
            let (t, d, tpl) = (
                api::fetch_deploy_targets().await,
                api::fetch_deploy_history(Some(50)).await,
                api::fetch_marketplace_listings(None, Some("deploy"), None, None).await,
            );
            if let Ok(r) = t { targets.set(r.targets); }
            if let Ok(r) = d { deployments.set(r.deployments); }
            if let Ok(r) = tpl { templates.set(r.listings); }
            loading.set(false);
        });
    };
    reload();

    // Add target handler
    let do_add_target = move || {
        let name = new_name.get();
        let ttype = new_type.get();
        if name.is_empty() { return; }
        saving.set(true);
        let config = api::DeployTargetConfig {
            api_key: new_api_key.get(),
            project_id: new_project_id.get(),
            team_id: new_team_id.get(),
            region: new_region.get(),
            custom_domain: new_domain.get(),
            extra: Default::default(),
        };
        spawn_local(async move {
            match api::create_deploy_target(&name, &ttype, &config).await {
                Ok(_) => {
                    toast_ok.set(true);
                    toast_msg.set(format!("Added: {}", name));
                    show_add.set(false);
                    new_name.set(String::new());
                    new_api_key.set(String::new());
                    new_project_id.set(String::new());
                    new_team_id.set(String::new());
                    new_region.set(String::new());
                    new_domain.set(String::new());
                    if let Ok(r) = api::fetch_deploy_targets().await { targets.set(r.targets); }
                }
                Err(e) => { toast_ok.set(false); toast_msg.set(format!("Error: {}", e)); }
            }
            saving.set(false);
        });
    };

    // Delete target handler
    let do_delete_target = move |id: String, name: String| {
        spawn_local(async move {
            match api::delete_deploy_target(&id).await {
                Ok(_) => {
                    toast_ok.set(true);
                    toast_msg.set(format!("Removed: {}", name));
                    if let Ok(r) = api::fetch_deploy_targets().await { targets.set(r.targets); }
                }
                Err(e) => { toast_ok.set(false); toast_msg.set(format!("Error: {}", e)); }
            }
        });
    };

    // Rollback handler
    let do_rollback = move |id: String, project: String| {
        rolling_back.set(Some(id.clone()));
        spawn_local(async move {
            match api::rollback_deployment(&id).await {
                Ok(_) => {
                    toast_ok.set(true);
                    toast_msg.set(format!("Rolled back: {}", project));
                    if let Ok(r) = api::fetch_deploy_history(Some(50)).await { deployments.set(r.deployments); }
                }
                Err(e) => { toast_ok.set(false); toast_msg.set(format!("Rollback failed: {}", e)); }
            }
            rolling_back.set(None);
        });
    };

    view! {
        <div style="padding: 32px;">
            // Toast
            <Show when=move || !toast_msg.get().is_empty()>
                <div style=move || format!(
                    "position: fixed; top: 20px; right: 20px; z-index: 9999; padding: 12px 20px; border-radius: 10px; \
                    font-size: 13px; font-family: 'Rajdhani', sans-serif; \
                    background: {}; border: 1px solid {}; color: rgba(255,245,240,0.9); \
                    box-shadow: 0 4px 20px rgba(0,0,0,0.4); cursor: pointer;",
                    if toast_ok.get() { "rgba(34,197,94,0.15)" } else { "rgba(239,68,68,0.15)" },
                    if toast_ok.get() { "rgba(34,197,94,0.3)" } else { "rgba(239,68,68,0.3)" },
                ) on:click=move |_| toast_msg.set(String::new())>
                    {move || toast_msg.get()}
                </div>
            </Show>

            // Header
            <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 24px;">
                <div>
                    <h1 style="font-family: 'Orbitron', monospace; font-size: 14px; letter-spacing: 6px; color: rgba(255,245,240,0.9); margin: 0;">
                        "\u{1f680} DEPLOY"
                    </h1>
                    <p style="font-size: 12px; color: rgba(255,245,240,0.7); margin: 4px 0 0;">
                        {move || {
                            let t = targets.get();
                            let d = deployments.get();
                            let live = d.iter().filter(|d| d.status == "live").count();
                            format!("{} target{} configured \u{00b7} {} deployment{} \u{00b7} {} live",
                                t.len(), if t.len() == 1 { "" } else { "s" },
                                d.len(), if d.len() == 1 { "" } else { "s" },
                                live)
                        }}
                    </p>
                </div>
                <div style="display: flex; gap: 8px;">
                    <Button primary=true on_click=Some(Callback::new(move |_| show_add.set(true)))>
                        "+ Add Target"
                    </Button>
                </div>
            </div>

            // Tabs
            <div style="display: flex; gap: 4px; margin-bottom: 20px;">
                {["targets", "history", "templates"].iter().map(|t| {
                    let t_str = t.to_string();
                    let t_label = match *t { "targets" => "DEPLOY TARGETS", "history" => "DEPLOYMENT HISTORY", _ => "TEMPLATES" };
                    let t_click = t.to_string();
                    view! {
                        <button
                            on:click=move |_| tab.set(t_click.clone())
                            style=move || format!(
                                "padding: 8px 18px; border-radius: 8px; font-family: 'Orbitron', monospace; font-size: 9px; \
                                letter-spacing: 2px; cursor: pointer; border: 1px solid {}; background: {}; color: {}; transition: all 0.15s;",
                                if tab.get() == t_str { "rgba(255,60,20,0.4)" } else { "rgba(255,245,240,0.08)" },
                                if tab.get() == t_str { "rgba(255,60,20,0.15)" } else { "transparent" },
                                if tab.get() == t_str { "rgba(255,245,240,0.9)" } else { "rgba(255,245,240,0.35)" },
                            )
                        >
                            {t_label}
                        </button>
                    }
                }).collect::<Vec<_>>()}
            </div>

            // Add target form (modal)
            <Show when=move || show_add.get()>
                <div style="position: fixed; inset: 0; background: rgba(0,0,0,0.7); z-index: 10000; display: flex; align-items: center; justify-content: center;"
                    on:click=move |_| show_add.set(false)>
                    <div style="background: rgba(20,12,8,0.95); border: 1px solid rgba(255,60,20,0.25); border-radius: 16px; \
                        width: 480px; padding: 28px; box-shadow: 0 20px 60px rgba(0,0,0,0.6);"
                        on:click=move |e| e.stop_propagation()>

                        <div style="font-family: 'Orbitron', monospace; font-size: 12px; letter-spacing: 4px; color: rgba(255,245,240,0.9); margin-bottom: 20px;">
                            "ADD DEPLOY TARGET"
                        </div>

                        // Name
                        <div style="margin-bottom: 14px;">
                            <label style="font-size: 10px; color: rgba(255,245,240,0.4); font-family: 'Orbitron', monospace; letter-spacing: 2px; display: block; margin-bottom: 6px;">
                                "NAME"
                            </label>
                            <input type="text" placeholder="My Vercel Project"
                                style="width: 100%; padding: 10px 14px; background: rgba(0,0,0,0.3); border: 1px solid rgba(255,245,240,0.08); \
                                    border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 13px; font-family: 'Rajdhani', sans-serif;"
                                prop:value=move || new_name.get()
                                on:input=move |e| new_name.set(event_target_value(&e))
                            />
                        </div>

                        // Target type
                        <div style="margin-bottom: 14px;">
                            <label style="font-size: 10px; color: rgba(255,245,240,0.4); font-family: 'Orbitron', monospace; letter-spacing: 2px; display: block; margin-bottom: 6px;">
                                "TARGET TYPE"
                            </label>
                            <div style="display: flex; gap: 6px; flex-wrap: wrap;">
                                {TARGET_TYPES.iter().map(|(val, label, icon)| {
                                    let v = val.to_string();
                                    let v2 = val.to_string();
                                    view! {
                                        <button
                                            on:click=move |_| new_type.set(v.clone())
                                            style=move || format!(
                                                "padding: 8px 14px; border-radius: 8px; font-size: 11px; cursor: pointer; \
                                                border: 1px solid {}; background: {}; color: {}; font-family: 'Rajdhani', sans-serif;",
                                                if new_type.get() == v2 { "rgba(255,60,20,0.4)" } else { "rgba(255,245,240,0.08)" },
                                                if new_type.get() == v2 { "rgba(255,60,20,0.15)" } else { "transparent" },
                                                if new_type.get() == v2 { "rgba(255,245,240,0.9)" } else { "rgba(255,245,240,0.7)" },
                                            )
                                        >
                                            {format!("{} {}", icon, label)}
                                        </button>
                                    }
                                }).collect::<Vec<_>>()}
                            </div>
                        </div>

                        // API Key
                        <div style="margin-bottom: 14px;">
                            <label style="font-size: 10px; color: rgba(255,245,240,0.4); font-family: 'Orbitron', monospace; letter-spacing: 2px; display: block; margin-bottom: 6px;">
                                "API KEY / TOKEN"
                            </label>
                            <input type="password" placeholder="Enter API key..."
                                style="width: 100%; padding: 10px 14px; background: rgba(0,0,0,0.3); border: 1px solid rgba(255,245,240,0.08); \
                                    border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 13px; font-family: monospace;"
                                prop:value=move || new_api_key.get()
                                on:input=move |e| new_api_key.set(event_target_value(&e))
                            />
                        </div>

                        // Project ID + Team ID row
                        <div style="display: flex; gap: 12px; margin-bottom: 14px;">
                            <div style="flex: 1;">
                                <label style="font-size: 10px; color: rgba(255,245,240,0.4); font-family: 'Orbitron', monospace; letter-spacing: 2px; display: block; margin-bottom: 6px;">
                                    "PROJECT ID"
                                </label>
                                <input type="text" placeholder="prj_..."
                                    style="width: 100%; padding: 10px 14px; background: rgba(0,0,0,0.3); border: 1px solid rgba(255,245,240,0.08); \
                                        border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 13px; font-family: monospace;"
                                    prop:value=move || new_project_id.get()
                                    on:input=move |e| new_project_id.set(event_target_value(&e))
                                />
                            </div>
                            <div style="flex: 1;">
                                <label style="font-size: 10px; color: rgba(255,245,240,0.4); font-family: 'Orbitron', monospace; letter-spacing: 2px; display: block; margin-bottom: 6px;">
                                    "TEAM ID"
                                </label>
                                <input type="text" placeholder="team_..."
                                    style="width: 100%; padding: 10px 14px; background: rgba(0,0,0,0.3); border: 1px solid rgba(255,245,240,0.08); \
                                        border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 13px; font-family: monospace;"
                                    prop:value=move || new_team_id.get()
                                    on:input=move |e| new_team_id.set(event_target_value(&e))
                                />
                            </div>
                        </div>

                        // Region + Domain row
                        <div style="display: flex; gap: 12px; margin-bottom: 20px;">
                            <div style="flex: 1;">
                                <label style="font-size: 10px; color: rgba(255,245,240,0.4); font-family: 'Orbitron', monospace; letter-spacing: 2px; display: block; margin-bottom: 6px;">
                                    "REGION"
                                </label>
                                <input type="text" placeholder="us-east-1"
                                    style="width: 100%; padding: 10px 14px; background: rgba(0,0,0,0.3); border: 1px solid rgba(255,245,240,0.08); \
                                        border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 13px; font-family: monospace;"
                                    prop:value=move || new_region.get()
                                    on:input=move |e| new_region.set(event_target_value(&e))
                                />
                            </div>
                            <div style="flex: 1;">
                                <label style="font-size: 10px; color: rgba(255,245,240,0.4); font-family: 'Orbitron', monospace; letter-spacing: 2px; display: block; margin-bottom: 6px;">
                                    "CUSTOM DOMAIN"
                                </label>
                                <input type="text" placeholder="app.example.com"
                                    style="width: 100%; padding: 10px 14px; background: rgba(0,0,0,0.3); border: 1px solid rgba(255,245,240,0.08); \
                                        border-radius: 8px; color: rgba(255,245,240,0.9); font-size: 13px; font-family: monospace;"
                                    prop:value=move || new_domain.get()
                                    on:input=move |e| new_domain.set(event_target_value(&e))
                                />
                            </div>
                        </div>

                        // Actions
                        <div style="display: flex; justify-content: flex-end; gap: 10px;">
                            <button
                                style="padding: 8px 18px; border-radius: 8px; background: transparent; border: 1px solid rgba(255,245,240,0.1); \
                                    color: rgba(255,245,240,0.5); font-size: 12px; cursor: pointer; font-family: 'Rajdhani', sans-serif;"
                                on:click=move |_| show_add.set(false)
                            >"Cancel"</button>
                            <button
                                style=move || format!(
                                    "padding: 8px 20px; border-radius: 8px; border: none; font-size: 12px; font-weight: 700; \
                                    cursor: {}; font-family: 'Orbitron', monospace; letter-spacing: 1px; \
                                    background: {}; color: rgba(255,245,240,0.95);",
                                    if new_name.get().is_empty() || saving.get() { "not-allowed" } else { "pointer" },
                                    if new_name.get().is_empty() { "rgba(255,245,240,0.08)" } else { "rgba(255,60,20,0.8)" },
                                )
                                disabled=move || new_name.get().is_empty() || saving.get()
                                on:click={
                                    let do_add = do_add_target;
                                    move |_| do_add()
                                }
                            >
                                {move || if saving.get() { "Saving..." } else { "Add Target" }}
                            </button>
                        </div>
                    </div>
                </div>
            </Show>

            // Content
            {move || {
                if loading.get() {
                    return view! {
                        <div style="display: flex; justify-content: center; padding: 60px 0; color: rgba(255,245,240,0.3); font-size: 13px;">
                            "Loading deploy data..."
                        </div>
                    }.into_any();
                }

                match tab.get().as_str() {
                    "history" => {
                        let deps = deployments.get();
                        if deps.is_empty() {
                            return view! {
                                <div style="display: flex; flex-direction: column; align-items: center; justify-content: center; min-height: 300px; gap: 16px;">
                                    <div style="width: 56px; height: 56px; border-radius: 14px; background: rgba(255,60,20,0.15); border: 1px solid rgba(255,60,20,0.1); display: flex; align-items: center; justify-content: center;">
                                        <Icon name="deploy" size=24 color="rgba(255,60,20,0.6)".to_string() />
                                    </div>
                                    <div style="font-family: 'Orbitron', monospace; font-size: 12px; letter-spacing: 4px; color: rgba(255,245,240,0.9);">
                                        "NO DEPLOYMENTS YET"
                                    </div>
                                    <div style="font-size: 13px; color: rgba(255,245,240,0.7); max-width: 360px; text-align: center; line-height: 1.6;">
                                        "Deployments will appear here when you build and ship from the War Room."
                                    </div>
                                </div>
                            }.into_any();
                        }

                        let do_rb = do_rollback;
                        view! {
                            <div style="display: flex; flex-direction: column; gap: 10px;">
                                {deps.into_iter().map(|d| {
                                    let did = d.id.clone();
                                    let dname = d.project_name.clone();
                                    let do_rb = do_rb;
                                    view! {
                                        <Card>
                                            <div style="display: flex; justify-content: space-between; align-items: flex-start;">
                                                <div style="flex: 1;">
                                                    <div style="display: flex; align-items: center; gap: 10px; margin-bottom: 6px;">
                                                        <span style="font-size: 18px;">{target_icon(&d.target_type)}</span>
                                                        <span style="font-family: 'Orbitron', monospace; font-size: 14px; color: rgba(255,245,240,0.95); font-weight: 600;">
                                                            {d.project_name.clone()}
                                                        </span>
                                                        <span style=format!(
                                                            "padding: 2px 8px; border-radius: 6px; font-size: 9px; font-family: 'Orbitron', monospace; \
                                                            letter-spacing: 1px; font-weight: 700; color: {}; background: {};",
                                                            status_color(&d.status), status_bg(&d.status)
                                                        )>
                                                            {d.status.to_uppercase()}
                                                        </span>
                                                    </div>
                                                    {(!d.url.is_empty()).then(|| view! {
                                                        <div style="margin-bottom: 4px;">
                                                            <a href=d.url.clone() target="_blank"
                                                                style="font-size: 12px; color: rgba(255,60,20,0.8); text-decoration: none;">
                                                                {d.url.clone()}
                                                            </a>
                                                        </div>
                                                    })}
                                                    <div style="display: flex; gap: 16px; font-size: 10px; color: rgba(255,245,240,0.35); margin-top: 4px;">
                                                        <span>"v"{d.version.to_string()}</span>
                                                        <span>{d.target_type.clone()}</span>
                                                        <span>{d.created_at.get(..16).unwrap_or(&d.created_at).to_string()}</span>
                                                    </div>
                                                    {(!d.error_message.is_empty()).then(|| view! {
                                                        <div style="margin-top: 8px; padding: 8px 12px; background: rgba(239,68,68,0.08); border: 1px solid rgba(239,68,68,0.15); \
                                                            border-radius: 8px; font-size: 11px; color: rgba(239,68,68,0.8);">
                                                            {d.error_message.clone()}
                                                        </div>
                                                    })}
                                                </div>
                                                <div style="display: flex; gap: 6px;">
                                                    {(d.status == "live").then(|| {
                                                        view! {
                                                            <button
                                                                style="padding: 6px 14px; border-radius: 8px; font-size: 10px; \
                                                                    font-family: 'Orbitron', monospace; letter-spacing: 1px; cursor: pointer; \
                                                                    background: rgba(168,85,247,0.15); border: 1px solid rgba(168,85,247,0.3); \
                                                                    color: rgba(168,85,247,0.9);"
                                                                on:click=move |_| do_rb(did.clone(), dname.clone())
                                                            >
                                                                {move || if rolling_back.get().as_deref() == Some(&d.id) { "Rolling back..." } else { "Rollback" }}
                                                            </button>
                                                        }
                                                    })}
                                                </div>
                                            </div>
                                        </Card>
                                    }
                                }).collect::<Vec<_>>()}
                            </div>
                        }.into_any()
                    }
                    "templates" => {
                        let tpls = templates.get();
                        if tpls.is_empty() {
                            return view! {
                                <div style="display: flex; flex-direction: column; align-items: center; justify-content: center; min-height: 300px; gap: 16px;">
                                    <div style="width: 56px; height: 56px; border-radius: 14px; background: rgba(255,60,20,0.15); border: 1px solid rgba(255,60,20,0.1); display: flex; align-items: center; justify-content: center;">
                                        <Icon name="tools" size=24 color="rgba(255,60,20,0.6)".to_string() />
                                    </div>
                                    <div style="font-family: 'Orbitron', monospace; font-size: 12px; letter-spacing: 4px; color: rgba(255,245,240,0.9);">
                                        "NO TEMPLATES"
                                    </div>
                                    <div style="font-size: 13px; color: rgba(255,245,240,0.7); max-width: 400px; text-align: center; line-height: 1.6;">
                                        "Deploy templates are pre-configured pipelines from the Agora marketplace. Publish a skill tagged \"deploy\" to see it here."
                                    </div>
                                </div>
                            }.into_any();
                        }

                        view! {
                            <div style="display: grid; grid-template-columns: repeat(auto-fill, minmax(320px, 1fr)); gap: 14px;">
                                {tpls.into_iter().map(|tpl| {
                                    let tpl_name = tpl.name.clone();
                                    view! {
                                        <Card>
                                            <div style="display: flex; flex-direction: column; gap: 10px;">
                                                // Header
                                                <div style="display: flex; align-items: center; gap: 10px;">
                                                    <span style="font-size: 22px;">"\u{1f4c4}"</span>
                                                    <div style="flex: 1;">
                                                        <div style="font-family: 'Orbitron', monospace; font-size: 13px; color: rgba(255,245,240,0.95); font-weight: 600;">
                                                            {tpl.name.clone()}
                                                        </div>
                                                        <div style="font-size: 11px; color: rgba(255,245,240,0.7); line-height: 1.4; margin-top: 2px;">
                                                            {tpl.description.clone()}
                                                        </div>
                                                    </div>
                                                </div>

                                                // Tags
                                                {(!tpl.tags.is_empty()).then(|| {
                                                    let tags = tpl.tags.clone();
                                                    view! {
                                                        <div style="display: flex; gap: 4px; flex-wrap: wrap;">
                                                            {tags.into_iter().take(5).map(|tag| {
                                                                view! {
                                                                    <span style="font-size: 9px; padding: 2px 8px; border-radius: 4px; \
                                                                        background: rgba(255,60,20,0.1); color: rgba(255,60,20,0.7); \
                                                                        font-family: 'Orbitron', monospace; letter-spacing: 0.5px;">
                                                                        {tag}
                                                                    </span>
                                                                }
                                                            }).collect::<Vec<_>>()}
                                                        </div>
                                                    }
                                                })}

                                                // Stats + Use button
                                                <div style="display: flex; align-items: center; gap: 12px;">
                                                    <span style="font-size: 11px; color: rgba(255,245,240,0.4);">
                                                        {format!("\u{2b50} {:.1}", tpl.rating)}
                                                    </span>
                                                    <span style="font-size: 11px; color: rgba(255,245,240,0.3);">
                                                        {format!("\u{2b07} {}", tpl.downloads)}
                                                    </span>
                                                    {if tpl.price_tokens > 0 {
                                                        view! {
                                                            <span style="font-size: 11px; color: rgba(255,60,20,0.7); font-weight: 600;">
                                                                {format!("{} tokens", tpl.price_tokens)}
                                                            </span>
                                                        }.into_any()
                                                    } else {
                                                        view! {
                                                            <span style="font-size: 11px; color: rgba(34,197,94,0.8); font-weight: 700;">
                                                                "FREE"
                                                            </span>
                                                        }.into_any()
                                                    }}
                                                    <span style="flex: 1;" />
                                                    <button
                                                        style="padding: 6px 16px; border-radius: 8px; font-size: 10px; \
                                                            font-family: 'Orbitron', monospace; letter-spacing: 1px; cursor: pointer; \
                                                            background: rgba(255,60,20,0.8); border: none; color: rgba(255,245,240,0.95); font-weight: 700;"
                                                        on:click=move |_| {
                                                            show_add.set(true);
                                                            toast_ok.set(true);
                                                            toast_msg.set(format!("Template: {} \u{2014} fill in your credentials", tpl_name));
                                                        }
                                                    >
                                                        "USE TEMPLATE"
                                                    </button>
                                                </div>
                                            </div>
                                        </Card>
                                    }
                                }).collect::<Vec<_>>()}
                            </div>
                        }.into_any()
                    }
                    _ => {
                        // Targets tab
                        let tgts = targets.get();
                        if tgts.is_empty() {
                            return view! {
                                <div style="display: flex; flex-direction: column; align-items: center; justify-content: center; min-height: 300px; gap: 16px;">
                                    <div style="width: 56px; height: 56px; border-radius: 14px; background: rgba(255,60,20,0.15); border: 1px solid rgba(255,60,20,0.1); display: flex; align-items: center; justify-content: center;">
                                        <Icon name="settings" size=24 color="rgba(255,60,20,0.6)".to_string() />
                                    </div>
                                    <div style="font-family: 'Orbitron', monospace; font-size: 12px; letter-spacing: 4px; color: rgba(255,245,240,0.9);">
                                        "NO DEPLOY TARGETS"
                                    </div>
                                    <div style="font-size: 13px; color: rgba(255,245,240,0.7); max-width: 360px; text-align: center; line-height: 1.6;">
                                        "Connect your first deploy target \u{2014} Vercel, Netlify, Docker, or self-hosted."
                                    </div>
                                    <Button primary=true on_click=Some(Callback::new(move |_| show_add.set(true)))>
                                        "+ Add Target"
                                    </Button>
                                </div>
                            }.into_any();
                        }

                        let do_del = do_delete_target;
                        view! {
                            <div style="display: grid; grid-template-columns: repeat(auto-fill, minmax(320px, 1fr)); gap: 14px;">
                                {tgts.into_iter().map(|t| {
                                    let tid = t.id.clone();
                                    let tname = t.name.clone();
                                    let do_del = do_del;
                                    view! {
                                        <Card>
                                            <div style="display: flex; justify-content: space-between; align-items: flex-start;">
                                                <div style="flex: 1;">
                                                    <div style="display: flex; align-items: center; gap: 10px; margin-bottom: 8px;">
                                                        <span style="font-size: 24px;">{target_icon(&t.target_type)}</span>
                                                        <div>
                                                            <div style="font-family: 'Orbitron', monospace; font-size: 14px; color: rgba(255,245,240,0.95); font-weight: 600;">
                                                                {t.name.clone()}
                                                            </div>
                                                            <div style="font-size: 10px; color: rgba(255,245,240,0.35);">
                                                                {t.target_type.to_uppercase()}
                                                            </div>
                                                        </div>
                                                    </div>

                                                    // Config details
                                                    <div style="display: flex; flex-direction: column; gap: 4px; font-size: 11px; margin-top: 8px;">
                                                        {(!t.config.project_id.is_empty()).then(|| view! {
                                                            <div style="display: flex; gap: 8px;">
                                                                <span style="color: rgba(255,245,240,0.3); width: 70px;">"Project:"</span>
                                                                <span style="color: rgba(255,245,240,0.6); font-family: monospace;">{t.config.project_id.clone()}</span>
                                                            </div>
                                                        })}
                                                        {(!t.config.region.is_empty()).then(|| view! {
                                                            <div style="display: flex; gap: 8px;">
                                                                <span style="color: rgba(255,245,240,0.3); width: 70px;">"Region:"</span>
                                                                <span style="color: rgba(255,245,240,0.6);">{t.config.region.clone()}</span>
                                                            </div>
                                                        })}
                                                        {(!t.config.custom_domain.is_empty()).then(|| view! {
                                                            <div style="display: flex; gap: 8px;">
                                                                <span style="color: rgba(255,245,240,0.3); width: 70px;">"Domain:"</span>
                                                                <span style="color: rgba(255,245,240,0.6);">{t.config.custom_domain.clone()}</span>
                                                            </div>
                                                        })}
                                                        <div style="display: flex; gap: 8px;">
                                                            <span style="color: rgba(255,245,240,0.3); width: 70px;">"API Key:"</span>
                                                            <span style="color: rgba(255,245,240,0.6); font-family: monospace;">
                                                                {if t.config.api_key.is_empty() { "\u{2014}".to_string() } else { "\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}".to_string() }}
                                                            </span>
                                                        </div>
                                                    </div>
                                                </div>

                                                <div style="display: flex; flex-direction: column; align-items: flex-end; gap: 8px;">
                                                    <span style=format!(
                                                        "padding: 2px 8px; border-radius: 6px; font-size: 9px; font-family: 'Orbitron', monospace; \
                                                        letter-spacing: 1px; font-weight: 700; color: {}; background: {};",
                                                        status_color(&t.status), status_bg(&t.status)
                                                    )>
                                                        {t.status.to_uppercase()}
                                                    </span>
                                                    <button
                                                        style="padding: 4px 10px; border-radius: 6px; font-size: 10px; cursor: pointer; \
                                                            background: rgba(239,68,68,0.1); border: 1px solid rgba(239,68,68,0.2); \
                                                            color: rgba(239,68,68,0.7);"
                                                        on:click=move |_| do_del(tid.clone(), tname.clone())
                                                    >
                                                        "Remove"
                                                    </button>
                                                </div>
                                            </div>
                                        </Card>
                                    }
                                }).collect::<Vec<_>>()}
                            </div>
                        }.into_any()
                    }
                }
            }}
        </div>
    }
}
