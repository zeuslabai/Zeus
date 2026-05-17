#![allow(dead_code)]
// ═══════════════════════════════════════════════════════════
// ZEUS — Sentient Orb Canvas Components
// Inline variant: compact orb for avatars/cards (from zeus-ios.jsx)
// Full variant: fibonacci sphere + wireframe + particles + tendrils (from zeus-orb.jsx)
// ═══════════════════════════════════════════════════════════

use leptos::prelude::*;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{window, HtmlCanvasElement, CanvasRenderingContext2d};
use std::f64::consts::PI;

const TAU: f64 = PI * 2.0;
const GOLDEN: f64 = 1.618033988749;

// ─── INLINE SENTIENT ORB ──────────────────────────────────
// Used in sidebar header, dashboard hero, agent cards, chat avatars, onboarding

#[component]
pub fn SentientOrb(
    #[prop(default = 120)] size: u32,
    #[prop(default = "dormant".to_string(), into)] mode: String,
    #[prop(default = 1.0)] intensity: f64,
) -> impl IntoView {
    let canvas_ref = NodeRef::<leptos::html::Canvas>::new();

    Effect::new(move |_| {
        let Some(canvas) = canvas_ref.get() else { return };
        let canvas: HtmlCanvasElement = canvas;
        let ctx = canvas
            .get_context("2d")
            .ok()
            .flatten()
            .and_then(|c| c.dyn_into::<CanvasRenderingContext2d>().ok());
        let Some(ctx) = ctx else { return };

        let dpr = 2.0_f64;
        let w = size as f64 * dpr;
        let h = size as f64 * dpr;
        canvas.set_width(w as u32);
        canvas.set_height(h as u32);

        let mode_str = mode.clone();
        let sz = size as f64;

        let state = std::rc::Rc::new(std::cell::RefCell::new(InlineOrbState::new(&mode_str, intensity)));
        let anim_id = std::rc::Rc::new(std::cell::RefCell::new(0i32));

        let state_c = state.clone();
        let anim_id_c = anim_id.clone();

        let f: std::rc::Rc<std::cell::RefCell<Option<Closure<dyn FnMut()>>>> =
            std::rc::Rc::new(std::cell::RefCell::new(None));
        let g = f.clone();

        *g.borrow_mut() = Some(Closure::new(move || {
            let mut s = state_c.borrow_mut();
            draw_inline_orb(&ctx, &mut s, w, h, sz, &mode_str);
            let id = request_animation_frame(f.borrow().as_ref().unwrap());
            *anim_id_c.borrow_mut() = id;
        }));

        let id = request_animation_frame(g.borrow().as_ref().unwrap());
        *anim_id.borrow_mut() = id;
    });

    let canvas_style = format!("width: {}px; height: {}px; background: transparent;", size, size);
    view! { <canvas node_ref=canvas_ref style={canvas_style} /> }
}

// ─── FULL-PAGE SENTIENT ORB ───────────────────────────────
// Fibonacci sphere (800 pts) + wireframe cage + 120 particles + tendrils
// Matches zeus-orb.jsx from the website exactly.

#[component]
pub fn SentientOrbFull(
    #[prop(default = 400)] _size: u32,
    #[prop(default = "idle".to_string(), into)] mode: String,
    #[prop(default = 1.0)] _intensity: f64,
) -> impl IntoView {
    let canvas_ref = NodeRef::<leptos::html::Canvas>::new();
    let mode_clone = mode.clone();

    Effect::new(move |_| {
        let Some(canvas) = canvas_ref.get() else { return };
        let canvas: HtmlCanvasElement = canvas;
        let ctx = canvas
            .get_context("2d")
            .ok()
            .flatten()
            .and_then(|c| c.dyn_into::<CanvasRenderingContext2d>().ok());
        let Some(ctx) = ctx else { return };

        let mode_str = mode_clone.clone();

        // Pre-compute static data
        let sphere_pts = generate_sphere_points(800);
        let wireframe = generate_wireframe(12, 16);
        let particles = generate_particles(120);

        let state = std::rc::Rc::new(std::cell::RefCell::new(
            FullOrbState::new(&mode_str, sphere_pts, wireframe, particles)
        ));

        let f: std::rc::Rc<std::cell::RefCell<Option<Closure<dyn FnMut()>>>> =
            std::rc::Rc::new(std::cell::RefCell::new(None));
        let g = f.clone();

        *g.borrow_mut() = Some(Closure::new(move || {
            let canvas_el = canvas.clone();
            let mut s = state.borrow_mut();
            s.update_target(&mode_str);
            draw_full_orb(&ctx, &canvas_el, &mut s);
            request_animation_frame(f.borrow().as_ref().unwrap());
        }));

        request_animation_frame(g.borrow().as_ref().unwrap());
    });

    let mode_display = mode.to_uppercase();
    view! {
        <div style="width: 100%; height: 100vh; background: #050508; display: flex; flex-direction: column; align-items: center; justify-content: center; position: relative; overflow: hidden; user-select: none;">
            // Title
            <div style="position: absolute; top: 32px; left: 0; right: 0; text-align: center; z-index: 10;">
                <div style="font-family: 'Orbitron', monospace; font-size: 11px; letter-spacing: 8px; color: rgba(255,80,40,0.5); margin-bottom: 4px;">"PROJECT"</div>
                <div style="font-family: 'Orbitron', monospace; font-size: 28px; font-weight: 200; letter-spacing: 16px; color: rgba(255,120,60,0.8); text-shadow: 0 0 30px rgba(255,60,20,0.3);">"ZEUS"</div>
            </div>

            <canvas node_ref=canvas_ref style="width: 100%; height: 100%; cursor: crosshair;" />

            // State indicator
            <div style="position: absolute; bottom: 100px; display: flex; flex-direction: column; align-items: center; gap: 6px;">
                <div style="font-family: 'Orbitron', monospace; font-size: 10px; letter-spacing: 6px; color: rgba(255,60,20,0.6);">{mode_display}</div>
                <div style="width: 40px; height: 1px; background: rgba(255,60,20,0.3);"></div>
            </div>

            // Corner decorations
            <svg style="position: absolute; top: 16px; left: 16px; opacity: 0.15;" width="40" height="40">
                <line x1="0" y1="0" x2="20" y2="0" stroke="rgba(255,80,40,1)" stroke-width="0.5" />
                <line x1="0" y1="0" x2="0" y2="20" stroke="rgba(255,80,40,1)" stroke-width="0.5" />
            </svg>
            <svg style="position: absolute; top: 16px; right: 16px; opacity: 0.15;" width="40" height="40">
                <line x1="20" y1="0" x2="40" y2="0" stroke="rgba(255,80,40,1)" stroke-width="0.5" />
                <line x1="40" y1="0" x2="40" y2="20" stroke="rgba(255,80,40,1)" stroke-width="0.5" />
            </svg>
            <svg style="position: absolute; bottom: 16px; left: 16px; opacity: 0.15;" width="40" height="40">
                <line x1="0" y1="40" x2="20" y2="40" stroke="rgba(255,80,40,1)" stroke-width="0.5" />
                <line x1="0" y1="20" x2="0" y2="40" stroke="rgba(255,80,40,1)" stroke-width="0.5" />
            </svg>
            <svg style="position: absolute; bottom: 16px; right: 16px; opacity: 0.15;" width="40" height="40">
                <line x1="20" y1="40" x2="40" y2="40" stroke="rgba(255,80,40,1)" stroke-width="0.5" />
                <line x1="40" y1="20" x2="40" y2="40" stroke="rgba(255,80,40,1)" stroke-width="0.5" />
            </svg>
        </div>
    }
}

// ══════════════════════════════════════════════════════════
// INLINE ORB INTERNALS (compact, used as avatar)
// ══════════════════════════════════════════════════════════

struct InlineOrbState {
    time: f64, spike: f64, glow: f64, rotation: f64, pulse_speed: f64,
    speak_wave: f64, breath_phase: f64,
    ts: f64, tg: f64, tr: f64, tp: f64,
}

impl InlineOrbState {
    fn new(mode: &str, intensity: f64) -> Self {
        let mut s = Self { time: 0.0, spike: 0.1, glow: 0.1, rotation: 0.1, pulse_speed: 0.3,
            speak_wave: 0.0, breath_phase: 0.0, ts: 0.1, tg: 0.1, tr: 0.1, tp: 0.3 };
        s.update_targets(mode, intensity);
        s
    }
    fn update_targets(&mut self, mode: &str, intensity: f64) {
        let (ts, tg, tr, tp) = match mode {
            "dormant"   => (0.08, 0.10, 0.05, 0.3),
            "waking"    => (0.20, 0.30, 0.15, 0.8),
            "listening" => (0.30, 0.45, 0.25, 1.2),
            "thinking"  => (0.25, 0.50, 0.12, 0.6),
            "active"    => (0.50, 0.70, 0.40, 2.0),
            "speaking"  => (0.60, 0.85, 0.50, 3.0),
            "alive"     => (0.75, 0.95, 0.55, 2.5),
            "surge" | "alert" | "rage" => (1.0, 1.0, 1.2, 5.0),
            _           => (0.35, 0.40, 0.30, 1.0),
        };
        self.ts = ts * intensity; self.tg = tg * intensity;
        self.tr = tr; self.tp = tp;
    }
}

fn draw_inline_orb(ctx: &CanvasRenderingContext2d, s: &mut InlineOrbState, w: f64, h: f64, sz: f64, mode: &str) {
    s.time += 0.016 * s.pulse_speed; s.breath_phase += 0.016 * s.pulse_speed * 1.5;
    s.spike = lerp(s.spike, s.ts, 0.03); s.glow = lerp(s.glow, s.tg, 0.03);
    s.pulse_speed = lerp(s.pulse_speed, s.tp, 0.04); s.rotation = lerp(s.rotation, s.tr, 0.03);
    if mode == "speaking" || mode == "surge" {
        s.speak_wave = lerp(s.speak_wave, 0.6 + (s.time * 8.0).sin() * 0.4, 0.1);
    } else { s.speak_wave = lerp(s.speak_wave, 0.0, 0.05); }

    ctx.set_fill_style_str("rgba(0,0,0,0.3)");
    ctx.fill_rect(0.0, 0.0, w, h);

    let cx = w / 2.0; let cy = h / 2.0; let base_r = sz * 0.38; let t = s.time;
    let pulse = (t * 2.0).sin() * 0.15 + 0.85;
    let glow_r = base_r * (2.2 + s.glow * 1.2) * pulse;
    if let Ok(grad) = ctx.create_radial_gradient(cx, cy, 0.0, cx, cy, glow_r) {
        let ga = 0.15 + s.glow * 0.25;
        let _ = grad.add_color_stop(0.0, &format!("rgba(255,60,10,{:.3})", ga));
        let _ = grad.add_color_stop(0.4, &format!("rgba(200,30,5,{:.3})", ga * 0.4));
        let _ = grad.add_color_stop(1.0, "rgba(0,0,0,0)");
        ctx.set_fill_style(&grad);
        ctx.begin_path();
        let _ = ctx.arc(cx, cy, glow_r, 0.0, TAU);
        ctx.fill();
    }

    let r = base_r * (s.breath_phase.sin() * 0.04 + 1.0);
    let spike_h = r * s.spike;
    let cos_ry = (t * s.rotation).cos(); let sin_ry = (t * s.rotation).sin();
    let cos_rx = (t * s.rotation * 0.6).cos(); let sin_rx = (t * s.rotation * 0.6).sin();

    for i in 0..=35 {
        let phi = (i as f64 / 35.0) * PI;
        for j in 0..=50 {
            let theta = (j as f64 / 50.0) * TAU;
            let n1 = (phi * 8.0 + t * 2.5).sin() * (theta * 6.0 + t * 1.8).cos();
            let n2 = (phi * 12.0 - t * 3.2).sin() * (theta * 10.0 + t * 2.1).cos();
            let spk = if mode == "speaking" || mode == "surge" {
                (phi * 20.0 + t * 12.0).sin() * (theta * 15.0 + t * 8.0).cos() * s.speak_wave * 0.4
            } else { 0.0 };
            let d = (n1 * 0.5 + n2 * 0.3 + spk).max(0.0);
            let total_r = r + d * spike_h;
            let x = total_r * phi.sin() * theta.cos();
            let z = total_r * phi.sin() * theta.sin();
            let y = total_r * phi.cos();
            let x2 = x * cos_ry - z * sin_ry;
            let z2 = x * sin_ry + z * cos_ry;
            let y2 = y * cos_rx - z2 * sin_rx;
            let z3 = y * sin_rx + z2 * cos_rx;
            let depth = ((z3 + r * 2.0) / (r * 4.0)).max(0.1);
            let alpha = depth * (0.4 + d * 0.6);
            let point_sz = (0.8 + d * 2.5) * depth;
            let cr = (180.0 + d * 75.0).min(255.0) as u8;
            let cg = (20.0 + d * 60.0).min(255.0) as u8;
            let cb = (5.0 + d * 15.0).min(255.0) as u8;
            ctx.set_fill_style_str(&format!("rgba({},{},{},{:.3})", cr, cg, cb, alpha));
            ctx.begin_path();
            let _ = ctx.arc(cx + x2, cy + y2, point_sz, 0.0, TAU);
            ctx.fill();
        }
    }
}

// ══════════════════════════════════════════════════════════
// FULL ORB INTERNALS (fibonacci sphere, from zeus-orb.jsx)
// ══════════════════════════════════════════════════════════

#[derive(Clone)]
struct SpherePoint { base_x: f64, base_y: f64, base_z: f64 }

#[derive(Clone)]
struct WirePoint { x: f64, y: f64, z: f64 }

struct Wireframe { parallels: Vec<Vec<WirePoint>>, meridians: Vec<Vec<WirePoint>> }

#[derive(Clone)]
struct Particle {
    x: f64, y: f64, z: f64,
    size: f64, speed: f64, phase: f64, brightness: f64,
    orbit_axis: [f64; 3],
}

#[derive(Clone)]
struct StateConfig {
    base_color: [f64; 3], glow_color: [f64; 3], core_color: [f64; 3],
    spike_length: f64, spike_speed: f64, pulse_speed: f64, pulse_amount: f64,
    rotation_speed: f64, particle_speed: f64, glow_intensity: f64, noise_scale: f64,
}

impl StateConfig {
    fn for_mode(mode: &str) -> Self {
        match mode {
            "speaking" => Self {
                base_color: [255.0, 50.0, 10.0], glow_color: [255.0, 120.0, 30.0], core_color: [255.0, 220.0, 100.0],
                spike_length: 0.55, spike_speed: 1.2, pulse_speed: 2.5, pulse_amount: 0.12,
                rotation_speed: 0.4, particle_speed: 1.2, glow_intensity: 1.0, noise_scale: 2.5,
            },
            "thinking" => Self {
                base_color: [180.0, 30.0, 60.0], glow_color: [200.0, 50.0, 100.0], core_color: [255.0, 120.0, 180.0],
                spike_length: 0.25, spike_speed: 0.6, pulse_speed: 1.5, pulse_amount: 0.06,
                rotation_speed: 0.8, particle_speed: 0.6, glow_intensity: 0.8, noise_scale: 3.0,
            },
            "listening" | "active" => Self {
                base_color: [200.0, 60.0, 20.0], glow_color: [240.0, 100.0, 40.0], core_color: [255.0, 180.0, 80.0],
                spike_length: 0.2, spike_speed: 0.5, pulse_speed: 1.0, pulse_amount: 0.03,
                rotation_speed: 0.1, particle_speed: 0.2, glow_intensity: 0.5, noise_scale: 1.5,
            },
            "alert" | "surge" | "rage" => Self {
                base_color: [255.0, 20.0, 0.0], glow_color: [255.0, 60.0, 0.0], core_color: [255.0, 255.0, 200.0],
                spike_length: 0.7, spike_speed: 2.0, pulse_speed: 4.0, pulse_amount: 0.18,
                rotation_speed: 1.0, particle_speed: 2.0, glow_intensity: 1.2, noise_scale: 3.5,
            },
            _ => Self { // idle / dormant / waking
                base_color: [220.0, 40.0, 30.0], glow_color: [255.0, 80.0, 20.0], core_color: [255.0, 160.0, 60.0],
                spike_length: 0.35, spike_speed: 0.3, pulse_speed: 0.8, pulse_amount: 0.04,
                rotation_speed: 0.15, particle_speed: 0.3, glow_intensity: 0.6, noise_scale: 1.8,
            },
        }
    }

    fn lerp_to(&self, other: &StateConfig, t: f64) -> StateConfig {
        let e = t * t * (3.0 - 2.0 * t); // smoothstep
        StateConfig {
            base_color: lerp_color(&self.base_color, &other.base_color, e),
            glow_color: lerp_color(&self.glow_color, &other.glow_color, e),
            core_color: lerp_color(&self.core_color, &other.core_color, e),
            spike_length: lerp(self.spike_length, other.spike_length, e),
            spike_speed: lerp(self.spike_speed, other.spike_speed, e),
            pulse_speed: lerp(self.pulse_speed, other.pulse_speed, e),
            pulse_amount: lerp(self.pulse_amount, other.pulse_amount, e),
            rotation_speed: lerp(self.rotation_speed, other.rotation_speed, e),
            particle_speed: lerp(self.particle_speed, other.particle_speed, e),
            glow_intensity: lerp(self.glow_intensity, other.glow_intensity, e),
            noise_scale: lerp(self.noise_scale, other.noise_scale, e),
        }
    }
}

struct FullOrbState {
    time: f64,
    sphere_pts: Vec<SpherePoint>,
    wireframe: Wireframe,
    particles: Vec<Particle>,
    from_cfg: StateConfig,
    to_cfg: StateConfig,
    transition: f64, // 0..1
    current_mode: String,
}

impl FullOrbState {
    fn new(mode: &str, sphere_pts: Vec<SpherePoint>, wireframe: Wireframe, particles: Vec<Particle>) -> Self {
        let cfg = StateConfig::for_mode(mode);
        Self {
            time: 0.0, sphere_pts, wireframe, particles,
            from_cfg: cfg.clone(), to_cfg: cfg,
            transition: 1.0, current_mode: mode.to_string(),
        }
    }

    fn update_target(&mut self, mode: &str) {
        if mode != self.current_mode {
            self.from_cfg = self.from_cfg.lerp_to(&self.to_cfg, self.transition);
            self.to_cfg = StateConfig::for_mode(mode);
            self.transition = 0.0;
            self.current_mode = mode.to_string();
        }
    }

    fn current_config(&self) -> StateConfig {
        if self.transition >= 1.0 { return self.to_cfg.clone(); }
        self.from_cfg.lerp_to(&self.to_cfg, self.transition)
    }
}

fn draw_full_orb(ctx: &CanvasRenderingContext2d, canvas: &HtmlCanvasElement, s: &mut FullOrbState) {
    s.time += 0.016;
    if s.transition < 1.0 { s.transition = (s.transition + 0.016 * 1.5).min(1.0); }

    let dpr = window().map(|w| w.device_pixel_ratio()).unwrap_or(1.0);
    // Use offset dimensions to size the canvas to fill its container
    let cw = canvas.offset_width() as f64;
    let ch = canvas.offset_height() as f64;
    let pw = (cw * dpr) as u32;
    let ph = (ch * dpr) as u32;
    if canvas.width() != pw { canvas.set_width(pw); }
    if canvas.height() != ph { canvas.set_height(ph); }
    let _ = ctx.scale(dpr, dpr);

    let cx = cw / 2.0; let cy = ch / 2.0;
    let base_radius = cw.min(ch) * 0.28;
    let cfg = s.current_config();
    let t = s.time;

    let pulse = (t * cfg.pulse_speed).sin() * cfg.pulse_amount;
    let radius = base_radius * (1.0 + pulse);
    let rot_y = t * cfg.rotation_speed;
    let rot_x = (t * 0.1).sin() * 0.15;

    // Clear
    ctx.set_fill_style_str("#050508");
    ctx.fill_rect(0.0, 0.0, cw, ch);

    // Background vignette
    if let Ok(vignette) = ctx.create_radial_gradient(cx, cy, 0.0, cx, cy, cw.max(ch) * 0.7) {
        let _ = vignette.add_color_stop(0.0, "rgba(20,5,5,0)");
        let _ = vignette.add_color_stop(1.0, "rgba(0,0,0,0.8)");
        ctx.set_fill_style(&vignette);
        ctx.fill_rect(0.0, 0.0, cw, ch);
    }

    // Outer glow
    let glow_r = radius * (2.2 + cfg.glow_intensity * 0.8);
    if let Ok(outer_glow) = ctx.create_radial_gradient(cx, cy, radius * 0.3, cx, cy, glow_r) {
        let [gr, gg, gb] = cfg.glow_color;
        let _ = outer_glow.add_color_stop(0.0, &format!("rgba({:.0},{:.0},{:.0},{:.3})", gr, gg, gb, 0.15 * cfg.glow_intensity));
        let _ = outer_glow.add_color_stop(0.4, &format!("rgba({:.0},{:.0},{:.0},{:.3})", gr, gg, gb, 0.06 * cfg.glow_intensity));
        let _ = outer_glow.add_color_stop(1.0, "rgba(0,0,0,0)");
        ctx.set_fill_style(&outer_glow);
        ctx.fill_rect(0.0, 0.0, cw, ch);
    }

    // Wireframe cage
    let cage_r = radius * 1.35;
    let cage_rot_y = rot_y * 0.3;
    let cage_rot_x = rot_x * 0.5 + t * 0.02;
    let [br, bg, bb] = cfg.base_color;
    ctx.set_stroke_style_str(&format!("rgba({:.0},{:.0},{:.0},0.08)", br, bg, bb));
    ctx.set_line_width(0.5);

    for line in &s.wireframe.parallels {
        ctx.begin_path();
        let mut started = false;
        for p in line {
            let proj = project(p.x, p.y, p.z, cx, cy, cage_r, cage_rot_y, cage_rot_x);
            if proj.2 < -0.3 { started = false; continue; }
            if !started { ctx.move_to(proj.0, proj.1); started = true; }
            else { ctx.line_to(proj.0, proj.1); }
        }
        ctx.stroke();
    }
    for line in &s.wireframe.meridians {
        ctx.begin_path();
        let mut started = false;
        for p in line {
            let proj = project(p.x, p.y, p.z, cx, cy, cage_r, cage_rot_y, cage_rot_x);
            if proj.2 < -0.3 { started = false; continue; }
            if !started { ctx.move_to(proj.0, proj.1); started = true; }
            else { ctx.line_to(proj.0, proj.1); }
        }
        ctx.stroke();
    }

    // Displaced sphere points (sorted by Z, painter's algorithm)
    let mut pts: Vec<(f64, f64, f64, f64, f64)> = s.sphere_pts.iter().map(|p| {
        let noise_val = fbm(
            p.base_x * cfg.noise_scale + t * cfg.spike_speed * 0.3,
            p.base_y * cfg.noise_scale + t * cfg.spike_speed * 0.2,
            p.base_z * cfg.noise_scale + t * cfg.spike_speed * 0.25,
            4,
        );
        let disp = 1.0 + noise_val.max(0.0) * cfg.spike_length
            + noise_val.max(0.0).powi(3) * cfg.spike_length * 1.5;
        let x = p.base_x * disp; let y = p.base_y * disp; let z = p.base_z * disp;
        let proj = project(x, y, z, cx, cy, radius, rot_y, rot_x);
        let intensity = (disp - 1.0) / (cfg.spike_length * 2.5);
        let depth_fade = ((proj.2 + 1.5) / 3.0).clamp(0.0, 1.0);
        (proj.0, proj.1, proj.2, intensity, depth_fade)
    }).collect();
    pts.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));

    for (px, py, pz, intensity, depth_fade) in &pts {
        if *pz < -1.2 { continue; }
        let persp = 3.5 / (3.5 + pz);
        let color = lerp_color(
            &lerp_color(&cfg.base_color, &cfg.glow_color, *intensity),
            &cfg.core_color, intensity.powi(2)
        );
        let alpha = (depth_fade * (0.5 + intensity * 0.5)).max(0.1);
        let point_sz = (1.5 + intensity * 3.0).max(0.8) * persp;

        // Glow for bright spikes
        if *intensity > 0.4 {
            let glow_sz = point_sz * (2.0 + intensity * 3.0);
            if let Ok(glow) = ctx.create_radial_gradient(*px, *py, 0.0, *px, *py, glow_sz) {
                let _ = glow.add_color_stop(0.0, &format!("rgba({:.0},{:.0},{:.0},{:.3})", color[0], color[1], color[2], alpha * 0.3));
                let _ = glow.add_color_stop(1.0, "rgba(0,0,0,0)");
                ctx.set_fill_style(&glow);
                ctx.fill_rect(px - glow_sz, py - glow_sz, glow_sz * 2.0, glow_sz * 2.0);
            }
        }

        ctx.set_fill_style_str(&format!("rgba({:.0},{:.0},{:.0},{:.3})", color[0], color[1], color[2], alpha));
        ctx.begin_path();
        let _ = ctx.arc(*px, *py, point_sz, 0.0, TAU);
        ctx.fill();

        // White hot core on tallest spikes
        if *intensity > 0.6 {
            ctx.set_fill_style_str(&format!("rgba(255,255,255,{:.3})", (intensity - 0.6) * 0.5 * alpha));
            ctx.begin_path();
            let _ = ctx.arc(*px, *py, point_sz * 0.4, 0.0, TAU);
            ctx.fill();
        }
    }

    // Inner core glow
    if let Ok(core_glow) = ctx.create_radial_gradient(cx, cy, 0.0, cx, cy, radius * 0.7) {
        let [cr, cg, cb] = cfg.core_color;
        let [br2, bg2, bb2] = cfg.base_color;
        let _ = core_glow.add_color_stop(0.0, &format!("rgba({:.0},{:.0},{:.0},{:.3})", cr, cg, cb, 0.12 * cfg.glow_intensity));
        let _ = core_glow.add_color_stop(0.5, &format!("rgba({:.0},{:.0},{:.0},{:.3})", br2, bg2, bb2, 0.04 * cfg.glow_intensity));
        let _ = core_glow.add_color_stop(1.0, "rgba(0,0,0,0)");
        ctx.set_fill_style(&core_glow);
        ctx.begin_path();
        let _ = ctx.arc(cx, cy, radius * 0.7, 0.0, TAU);
        ctx.fill();
    }

    // Particles
    let particles = s.particles.clone();
    for particle in &particles {
        let speed = particle.speed * cfg.particle_speed;
        let angle = t * speed + particle.phase;
        let cos_a = angle.cos(); let sin_a = angle.sin();
        let px2 = particle.x * cos_a + particle.z * sin_a;
        let py2 = particle.y + (t * 0.5 + particle.phase).sin() * 0.1;
        let pz2 = -particle.x * sin_a + particle.z * cos_a;
        let proj = project(px2, py2, pz2, cx, cy, radius, rot_y * 0.2, rot_x * 0.2);
        if proj.2 < -1.5 { continue; }
        let flicker = 0.5 + 0.5 * (t * 3.0 + particle.phase * 7.0).sin();
        let depth_alpha = ((proj.2 + 1.5) / 3.0).max(0.1);
        let alpha = particle.brightness * flicker * depth_alpha * 0.7;
        let sz = particle.size * (3.5 / (3.5 + proj.2));
        let [gr2, gg2, gb2] = cfg.glow_color;
        ctx.set_fill_style_str(&format!("rgba({:.0},{:.0},{:.0},{:.3})", gr2, gg2, gb2, alpha));
        ctx.fill_rect(proj.0 - sz / 2.0, proj.1 - sz / 2.0, sz, sz);

        if alpha > 0.3
            && let Ok(pg) = ctx.create_radial_gradient(proj.0, proj.1, 0.0, proj.0, proj.1, sz * 3.0) {
                let _ = pg.add_color_stop(0.0, &format!("rgba({:.0},{:.0},{:.0},{:.3})", gr2, gg2, gb2, alpha * 0.2));
                let _ = pg.add_color_stop(1.0, "rgba(0,0,0,0)");
                ctx.set_fill_style(&pg);
                ctx.fill_rect(proj.0 - sz * 3.0, proj.1 - sz * 3.0, sz * 6.0, sz * 6.0);
            }
    }

    // Energy tendrils (visible when speaking/alert)
    if cfg.spike_length > 0.4 {
        for i in 0..5 {
            let base_angle = (i as f64 / 5.0) * TAU + t * 0.5;
            ctx.begin_path();
            let [cr2, cg2, cb2] = cfg.core_color;
            ctx.set_stroke_style_str(&format!("rgba({:.0},{:.0},{:.0},{:.3})", cr2, cg2, cb2, (cfg.spike_length - 0.4) * 0.3));
            ctx.set_line_width(1.0);
            for j in 0..30 {
                let frac = j as f64 / 30.0;
                let r2 = radius * (0.6 + frac * 0.8);
                let angle = base_angle + (t * 2.0 + frac * 4.0 + i as f64).sin() * 0.5;
                let x = cx + angle.cos() * r2;
                let y = cy + angle.sin() * r2 + (t * 3.0 + frac * 5.0).sin() * 10.0;
                if j == 0 { ctx.move_to(x, y); } else { ctx.line_to(x, y); }
            }
            ctx.stroke();
        }
    }
}

// ══════════════════════════════════════════════════════════
// GEOMETRY GENERATORS
// ══════════════════════════════════════════════════════════

fn generate_sphere_points(count: usize) -> Vec<SpherePoint> {
    (0..count).map(|i| {
        let theta = ((1.0 - 2.0 * (i as f64 + 0.5) / count as f64).clamp(-1.0, 1.0)).acos();
        let phi = TAU * i as f64 / GOLDEN;
        SpherePoint {
            base_x: theta.sin() * phi.cos(),
            base_y: theta.sin() * phi.sin(),
            base_z: theta.cos(),
        }
    }).collect()
}

fn generate_wireframe(lat_count: usize, lon_count: usize) -> Wireframe {
    let mut parallels = Vec::new();
    for lat in 0..=lat_count {
        let theta = (lat as f64 / lat_count as f64) * PI;
        let pts: Vec<WirePoint> = (0..=lon_count).map(|lon| {
            let phi = (lon as f64 / lon_count as f64) * TAU;
            WirePoint { x: theta.sin() * phi.cos(), y: theta.sin() * phi.sin(), z: theta.cos() }
        }).collect();
        parallels.push(pts);
    }
    let mut meridians = Vec::new();
    for lon in 0..lon_count {
        let phi = (lon as f64 / lon_count as f64) * TAU;
        let pts: Vec<WirePoint> = (0..=lat_count).map(|lat| {
            let theta = (lat as f64 / lat_count as f64) * PI;
            WirePoint { x: theta.sin() * phi.cos(), y: theta.sin() * phi.sin(), z: theta.cos() }
        }).collect();
        meridians.push(pts);
    }
    Wireframe { parallels, meridians }
}

fn generate_particles(count: usize) -> Vec<Particle> {
    // Deterministic pseudo-random using LCG (no rand crate needed in WASM)
    let mut seed = 42u64;
    let mut rng = move || -> f64 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((seed >> 33) as f64) / (u32::MAX as f64)
    };
    (0..count).map(|_| {
        let theta = rng() * PI;
        let phi = rng() * TAU;
        let r = 1.4 + rng() * 0.8;
        Particle {
            x: r * theta.sin() * phi.cos(),
            y: r * theta.sin() * phi.sin(),
            z: r * theta.cos(),
            size: 1.0 + rng() * 3.0,
            speed: 0.2 + rng() * 0.8,
            phase: rng() * TAU,
            brightness: 0.3 + rng() * 0.7,
            orbit_axis: [rng() - 0.5, rng() - 0.5, rng() - 0.5],
        }
    }).collect()
}

// ══════════════════════════════════════════════════════════
// NOISE + MATH
// ══════════════════════════════════════════════════════════

fn noise3d(x: f64, y: f64, z: f64) -> f64 {
    let n = (x * 12.9898 + y * 78.233 + z * 45.164).sin() * 43758.5453;
    (n - n.floor()) * 2.0 - 1.0
}

fn smooth_noise(x: f64, y: f64, z: f64) -> f64 {
    let ix = x.floor(); let iy = y.floor(); let iz = z.floor();
    let fx = x - ix; let fy = y - iy; let fz = z - iz;
    let sx = fx * fx * (3.0 - 2.0 * fx);
    let sy = fy * fy * (3.0 - 2.0 * fy);
    let sz = fz * fz * (3.0 - 2.0 * fz);
    let n000 = noise3d(ix, iy, iz);       let n100 = noise3d(ix+1.0, iy, iz);
    let n010 = noise3d(ix, iy+1.0, iz);   let n110 = noise3d(ix+1.0, iy+1.0, iz);
    let n001 = noise3d(ix, iy, iz+1.0);   let n101 = noise3d(ix+1.0, iy, iz+1.0);
    let n011 = noise3d(ix, iy+1.0, iz+1.0); let n111 = noise3d(ix+1.0, iy+1.0, iz+1.0);
    n000*(1.0-sx)*(1.0-sy)*(1.0-sz) + n100*sx*(1.0-sy)*(1.0-sz) +
    n010*(1.0-sx)*sy*(1.0-sz)        + n110*sx*sy*(1.0-sz) +
    n001*(1.0-sx)*(1.0-sy)*sz        + n101*sx*(1.0-sy)*sz +
    n011*(1.0-sx)*sy*sz              + n111*sx*sy*sz
}

fn fbm(x: f64, y: f64, z: f64, octaves: u32) -> f64 {
    let (mut val, mut amp, mut freq) = (0.0, 0.5, 1.0);
    for _ in 0..octaves {
        val += amp * smooth_noise(x * freq, y * freq, z * freq);
        amp *= 0.5; freq *= 2.0;
    }
    val
}

/// Perspective projection with Y and X rotation.
/// Returns (screen_x, screen_y, z_depth).
#[allow(clippy::too_many_arguments)]
fn project(x: f64, y: f64, z: f64, cx: f64, cy: f64, scale: f64, rot_y: f64, rot_x: f64) -> (f64, f64, f64) {
    let x1 = x * rot_y.cos() - z * rot_y.sin();
    let z1 = x * rot_y.sin() + z * rot_y.cos();
    let y1 = y * rot_x.cos() - z1 * rot_x.sin();
    let z2 = y * rot_x.sin() + z1 * rot_x.cos();
    let persp = 3.5 / (3.5 + z2);
    (cx + x1 * scale * persp, cy + y1 * scale * persp, z2)
}

fn lerp(a: f64, b: f64, t: f64) -> f64 { a + (b - a) * t }

fn lerp_color(a: &[f64; 3], b: &[f64; 3], t: f64) -> [f64; 3] {
    [lerp(a[0], b[0], t), lerp(a[1], b[1], t), lerp(a[2], b[2], t)]
}

// ─── HELPERS ──────────────────────────────────────────────

fn request_animation_frame(f: &Closure<dyn FnMut()>) -> i32 {
    window().unwrap().request_animation_frame(f.as_ref().unchecked_ref()).unwrap()
}
