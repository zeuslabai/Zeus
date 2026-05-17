#![allow(dead_code)]
// ═══════════════════════════════════════════════════════════
// ZEUS — Shared Design Components (v3 — Full Inline Styles)
// Every component uses inline styles matching JSX S object
// Zero CSS class dependencies
// ═══════════════════════════════════════════════════════════

use leptos::prelude::*;

// ─── ICON ─────────────────────────────────────────────────
// 27 inline SVGs matching JSX `paths` object exactly

#[component]
pub fn Icon(
    #[prop(into)] name: String,
    #[prop(default = 18)] size: u32,
    #[prop(default = "currentColor".to_string(), into)] color: String,
) -> impl IntoView {
    let sz = size.to_string();
    let paths = match name.as_str() {
        "dashboard" => view! {
            <rect x="3" y="3" width="7" height="7" rx="1" />
            <rect x="14" y="3" width="7" height="7" rx="1" />
            <rect x="3" y="14" width="7" height="7" rx="1" />
            <rect x="14" y="14" width="7" height="7" rx="1" />
        }.into_any(),
        "chat" => view! {
            <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" />
        }.into_any(),
        "tools" => view! {
            <path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z" />
        }.into_any(),
        "memory" => view! {
            <path d="M4 19.5A2.5 2.5 0 0 1 6.5 17H20" />
            <path d="M6.5 2H20v20H6.5A2.5 2.5 0 0 1 4 19.5v-15A2.5 2.5 0 0 1 6.5 2z" />
        }.into_any(),
        "agents" => view! {
            <circle cx="12" cy="8" r="5" />
            <path d="M20 21a8 8 0 0 0-16 0" />
        }.into_any(),
        "sessions" | "activity" => view! {
            <polyline points="22 12 18 12 15 21 9 3 6 12 2 12" />
        }.into_any(),
        "channels" => view! {
            <path d="M4 11a9 9 0 0 1 9 9" />
            <path d="M4 4a16 16 0 0 1 16 16" />
            <circle cx="5" cy="19" r="1" />
        }.into_any(),
        "analytics" => view! {
            <line x1="18" y1="20" x2="18" y2="10" />
            <line x1="12" y1="20" x2="12" y2="4" />
            <line x1="6" y1="20" x2="6" y2="14" />
        }.into_any(),
        "security" => view! {
            <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" />
        }.into_any(),
        "settings" => view! {
            <circle cx="12" cy="12" r="3" />
            <path d="M12 1v2M12 21v2M4.22 4.22l1.42 1.42M18.36 18.36l1.42 1.42M1 12h2M21 12h2M4.22 19.78l1.42-1.42M18.36 5.64l1.42-1.42" />
        }.into_any(),
        "projects" => view! {
            <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
        }.into_any(),
        "teams" => view! {
            <path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2" />
            <circle cx="9" cy="7" r="4" />
            <path d="M23 21v-2a4 4 0 0 0-3-3.87" />
            <path d="M16 3.13a4 4 0 0 1 0 7.75" />
        }.into_any(),
        "skills" => view! {
            <polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2" />
        }.into_any(),
        "mcp" | "globe" => view! {
            <circle cx="12" cy="12" r="10" />
            <line x1="2" y1="12" x2="22" y2="12" />
            <path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z" />
        }.into_any(),
        "voice" => view! {
            <path d="M12 1a3 3 0 0 0-3 3v8a3 3 0 0 0 6 0V4a3 3 0 0 0-3-3z" />
            <path d="M19 10v2a7 7 0 0 1-14 0v-2" />
            <line x1="12" y1="19" x2="12" y2="23" />
        }.into_any(),
        "sandbox" => view! {
            <rect x="3" y="11" width="18" height="11" rx="2" />
            <path d="M7 11V7a5 5 0 0 1 10 0v4" />
        }.into_any(),
        "approvals" => view! {
            <polyline points="9 11 12 14 22 4" />
            <path d="M21 12v7a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11" />
        }.into_any(),
        "chevron" => view! {
            <polyline points="9 18 15 12 9 6" />
        }.into_any(),
        "search" => view! {
            <circle cx="11" cy="11" r="8" />
            <line x1="21" y1="21" x2="16.65" y2="16.65" />
        }.into_any(),
        "send" => view! {
            <line x1="22" y1="2" x2="11" y2="13" />
            <polygon points="22 2 15 22 11 13 2 9 22 2" />
        }.into_any(),
        "plus" => view! {
            <line x1="12" y1="5" x2="12" y2="19" />
            <line x1="5" y1="12" x2="19" y2="12" />
        }.into_any(),
        "play" => view! {
            <polygon points="5 3 19 12 5 21 5 3" />
        }.into_any(),
        "pause" => view! {
            <rect x="6" y="4" width="4" height="16" />
            <rect x="14" y="4" width="4" height="16" />
        }.into_any(),
        "zap" => view! {
            <polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2" />
        }.into_any(),
        "cpu" => view! {
            <rect x="4" y="4" width="16" height="16" rx="2" />
            <rect x="9" y="9" width="6" height="6" />
            <line x1="9" y1="1" x2="9" y2="4" />
            <line x1="15" y1="1" x2="15" y2="4" />
            <line x1="9" y1="20" x2="9" y2="23" />
            <line x1="15" y1="20" x2="15" y2="23" />
            <line x1="20" y1="9" x2="23" y2="9" />
            <line x1="20" y1="14" x2="23" y2="14" />
            <line x1="1" y1="9" x2="4" y2="9" />
            <line x1="1" y1="14" x2="4" y2="14" />
        }.into_any(),
        "logout" => view! {
            <path d="M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4" />
            <polyline points="16 17 21 12 16 7" />
            <line x1="21" y1="12" x2="9" y2="12" />
        }.into_any(),
        // Default fallback: zap icon
        _ => view! {
            <polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2" />
        }.into_any(),
    };

    view! {
        <svg
            width={sz.clone()}
            height={sz}
            viewBox="0 0 24 24"
            fill="none"
            stroke={color}
            stroke-width="1.5"
            stroke-linecap="round"
            stroke-linejoin="round"
            style="flex-shrink: 0; display: inline-block;"
        >
            {paths}
        </svg>
    }
}

// ─── CARD ─────────────────────────────────────────────────
// JSX: background: S.surface, border: 1px solid S.border, borderRadius: 12, padding: 20px

#[component]
pub fn Card(
    children: Children,
    #[prop(default = false)] glow: bool,
    #[prop(default = "".to_string(), into)] style: String,
) -> impl IntoView {
    let border = if glow { "rgba(255,60,20,0.25)" } else { "rgba(255,60,20,0.1)" };
    let shadow = if glow { "0 0 30px rgba(255,60,20,0.15)" } else { "none" };
    let base_style = format!(
        "background: rgba(255,255,255,0.03); border: 1px solid {}; border-radius: 12px; padding: 20px; transition: all 0.3s ease; box-shadow: {}; {}",
        border, shadow, style
    );
    view! {
        <div style={base_style}>
            {children()}
        </div>
    }
}

// ─── METRIC CARD ──────────────────────────────────────────
// JSX: Card with flex:1, minWidth:160 + label (Orbitron 10px) + value (Orbitron 22px bold)

#[component]
pub fn MetricCard(
    #[prop(into)] label: String,
    #[prop(into)] value: String,
    #[prop(into)] icon: String,
    #[prop(default = None)] trend: Option<String>,
) -> impl IntoView {
    let is_positive = trend.as_ref().map(|t| t.starts_with('+')).unwrap_or(false);
    view! {
        <div style="background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 12px; padding: 20px; transition: all 0.3s ease; flex: 1; min-width: 160px;">
            <div style="display: flex; justify-content: space-between; align-items: flex-start;">
                <div>
                    <div style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 3px; color: rgba(255,245,240,0.7); text-transform: uppercase; margin-bottom: 8px;">{label}</div>
                    <div style="font-family: 'Orbitron', monospace; font-size: 22px; font-weight: 700; color: rgba(255,245,240,0.9);">{value}</div>
                    {trend.map(|t| {
                        let color = if is_positive { "#22c55e" } else { "rgba(255,245,240,0.7)" };
                        view! { <div style={format!("font-size: 11px; color: {}; margin-top: 4px;", color)}>{t}</div> }
                    })}
                </div>
                <div style="color: rgba(255,60,20,0.6); opacity: 0.6;">
                    <Icon name={icon} size=20 />
                </div>
            </div>
        </div>
    }
}

// ─── STATUS DOT ───────────────────────────────────────────
// JSX: 6x6 circle, color mapped from status, boxShadow glow

#[component]
pub fn StatusDot(#[prop(into)] status: String) -> impl IntoView {
    let color = match status.as_str() {
        "connected" | "active" => "#22c55e",
        "idle" => "#eab308",
        "disconnected" | "error" => "#ef4444",
        _ => "rgba(255,245,240,0.7)",
    };
    let style = format!(
        "width: 6px; height: 6px; border-radius: 50%; background: {}; box-shadow: 0 0 6px {}; flex-shrink: 0;",
        color, color
    );
    view! { <div style={style}></div> }
}

// ─── BADGE ────────────────────────────────────────────────
// JSX: Orbitron 8px, ls:2, color, border: 1px solid ${color}33, padding: 2px 8px, radius: 4

#[component]
pub fn Badge(
    #[prop(into)] text: String,
    #[prop(default = "rgba(255,60,20,0.6)".to_string(), into)] color: String,
) -> impl IntoView {
    let border_color = if color.starts_with("rgba(") {
        if let Some(base) = color.strip_suffix(')') {
            if let Some(pos) = base.rfind(',') {
                format!("{},.33)", &base[..pos])
            } else {
                "rgba(255,60,20,0.2)".to_string()
            }
        } else {
            "rgba(255,60,20,0.2)".to_string()
        }
    } else if color.starts_with('#') {
        format!("{}55", color)
    } else {
        "rgba(255,60,20,0.2)".to_string()
    };
    let style = format!(
        "font-family: 'Orbitron', monospace; font-size: 8px; letter-spacing: 2px; color: {}; border: 1px solid {}; padding: 2px 8px; border-radius: 4px; text-transform: uppercase; display: inline-block;",
        color, border_color
    );
    view! { <span style={style}>{text}</span> }
}

// ─── SECTION TITLE ────────────────────────────────────────
// JSX: flex row, h2 Orbitron 11px, ls:5, textDim, uppercase, margin:0

#[component]
pub fn SectionTitle(
    children: Children,
    #[prop(optional)] action: Option<Children>,
) -> impl IntoView {
    view! {
        <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 16px;">
            <h2 style="font-family: 'Orbitron', monospace; font-size: 11px; letter-spacing: 5px; color: rgba(255,245,240,0.7); text-transform: uppercase; margin: 0;">{children()}</h2>
            {action.map(|a| a())}
        </div>
    }
}

// ─── SEARCH BAR ───────────────────────────────────────────
// JSX: relative div, absolute search icon, input with surface bg + border

#[component]
pub fn SearchBar(
    #[prop(default = "Search...".to_string(), into)] placeholder: String,
    value: RwSignal<String>,
) -> impl IntoView {
    view! {
        <div style="position: relative; margin-bottom: 16px;">
            <div style="position: absolute; left: 12px; top: 50%; transform: translateY(-50%); color: rgba(255,245,240,0.5);">
                <Icon name="search" size=14 />
            </div>
            <input
                type="text"
                placeholder={placeholder}
                prop:value={move || value.get()}
                on:input=move |ev| {
                    value.set(event_target_value(&ev));
                }
                style="width: 100%; padding: 10px 12px 10px 36px; background: rgba(255,255,255,0.03); border: 1px solid rgba(255,60,20,0.1); border-radius: 8px; color: rgba(255,245,240,0.9); font-family: 'Rajdhani', sans-serif; font-size: 13px; outline: none; box-sizing: border-box;"
            />
        </div>
    }
}

// ─── BUTTON ───────────────────────────────────────────────
// JSX: Orbitron 9px (8 if small), ls:2, uppercase, primary=ember bg/border/orange text

#[component]
pub fn Button(
    children: Children,
    #[prop(default = false)] primary: bool,
    #[prop(default = false)] small: bool,
    #[prop(default = None)] on_click: Option<Callback<()>>,
    #[prop(default = "".to_string(), into)] style: String,
) -> impl IntoView {
    let font_size = if small { "8px" } else { "9px" };
    let padding = if small { "4px 10px" } else { "8px 16px" };
    let bg = if primary { "rgba(255,60,20,0.15)" } else { "transparent" };
    let border = if primary { "1px solid rgba(255,60,20,0.5)" } else { "1px solid rgba(255,60,20,0.1)" };
    let color = if primary { "rgba(255,140,80,1)" } else { "rgba(255,245,240,0.7)" };
    let btn_style = format!(
        "font-family: 'Orbitron', monospace; font-size: {}; letter-spacing: 2px; text-transform: uppercase; background: {}; border: {}; color: {}; padding: {}; border-radius: 6px; cursor: pointer; transition: all 0.3s; display: flex; align-items: center; gap: 6px; {}",
        font_size, bg, border, color, padding, style
    );
    view! {
        <button
            style={btn_style}
            on:click=move |_| {
                if let Some(ref cb) = on_click {
                    cb.run(());
                }
            }
        >
            {children()}
        </button>
    }
}

// ─── PROGRESS BAR ─────────────────────────────────────────
// JSX: 3px track, colored fill, transition

#[component]
pub fn ProgressBar(
    #[prop(into)] value: f64,
    #[prop(default = 100.0)] max: f64,
    #[prop(default = "#ff3c14".to_string(), into)] color: String,
) -> impl IntoView {
    let pct = (value / max * 100.0).min(100.0);
    let fill_style = format!(
        "height: 100%; width: {}%; background: {}; border-radius: 2px; transition: width 0.5s ease;",
        pct, color
    );
    view! {
        <div style="height: 3px; background: rgba(255,255,255,0.05); border-radius: 2px; overflow: hidden;">
            <div style={fill_style}></div>
        </div>
    }
}

// ─── PLACEHOLDER PAGE ─────────────────────────────────────

#[allow(dead_code)]
#[component]
pub fn PlaceholderPage(
    #[prop(into)] title: String,
    #[prop(into)] desc: String,
) -> impl IntoView {
    view! {
        <div style="padding: 32px; display: flex; flex-direction: column; align-items: center; justify-content: center; min-height: 60vh;">
            <div style="width: 100px; height: 100px;">
                <super::sentient_orb::SentientOrb size=100 mode="thinking".to_string() />
            </div>
            <h1 style="font-family: 'Orbitron', monospace; font-size: 14px; letter-spacing: 6px; color: rgba(255,245,240,0.9); margin-top: 24px; text-transform: uppercase;">
                {title}
            </h1>
            <p style="font-family: 'Rajdhani', sans-serif; font-size: 13px; color: rgba(255,245,240,0.7); margin-top: 8px; text-align: center; max-width: 400px;">
                {desc}
            </p>
        </div>
    }
}
