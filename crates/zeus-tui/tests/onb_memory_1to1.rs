//! 1:1 render tests for onboarding Screen 17/19 — Memory (MMRY) · JSX 1609–1653.
//!
//! Covers the zeus106 fidelity cut on `fix/tui-1to1-memory`:
//!   • 2-column layout (LEFT card list + STORAGE fields, RIGHT 280w disk panel),
//!   • all 3 embedding backends present (Ollama/OpenAI/FTS-only) with glyphs,
//!   • FTS-only default (#258) = ★ REC + ▸ SELECTED; Ollama card keeps ● DETECTED,
//!   • STORAGE: DB Path field (always) + Embedding Model field (hidden for FTS-only),
//!   • model default per provider (ollama→nomic-embed-text, openai→text-embedding-3-small),
//!   • RIGHT panel: DISK PROJECTION rows (1K/10K/100K/1M) + OLLAMA DETECTED cyan box,
//!   • RIGHT panel left-border divider (JSX borderLeft),
//!   • ↑/↓ select provider (NOT step-nav), Tab cycles fields,
//!   • Memory (step 16) is a vertical list — Right STEP-ADVANCES (no grid),
//!   • ESC backs out one step (does not quit).
//!
//! Separate file = conflict-free with the other agents' onb_*.rs files.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use zeus_tui::App;
use zeus_tui::app::frame;

use crossterm::event::KeyCode;

const MEMORY_STEP: usize = 16;

/// Walk to the Memory step (16). Steps 1/6/8/9/11/15 consume Right for in-screen
/// focus (grids), so we bump those directly — mirrors goto_step in onb_106b.
/// Memory itself is a vertical list (Right step-advances), so we stop AT it.
fn goto_memory(app: &mut App) {
    let mut guard = 0;
    while app.current_step < MEMORY_STEP {
        if app.current_step == 3 { app.current_step += 1; app.on_step_enter(); continue; }        let s = app.current_step;
        if s == 1 {
            app.handle_key(KeyCode::Enter);
        } else if s == 6 || s == 8 || s == 9 || s == 11 || s == 15 {
            app.current_step += 1;
            app.on_step_enter();
        } else {
            app.handle_key(KeyCode::Right);
        }
        if app.current_step == s {
            app.handle_key(KeyCode::Enter);
        }
        guard += 1;
        assert!(guard < 100, "goto_memory stalled before reaching Memory");
    }
    assert_eq!(app.current_step, MEMORY_STEP, "failed to reach Memory step");
}

fn render(app: &mut App) -> String {
    let backend = TestBackend::new(140, 44);
    let mut terminal = Terminal::new(backend).expect("terminal");
    terminal
        .draw(|f| frame(f, app))
        .expect("draw must not panic");
    buf_to_string(terminal.backend().buffer().clone())
}

fn buf_to_string(buf: ratatui::buffer::Buffer) -> String {
    let mut out = String::new();
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

// ── header + sub ──────────────────────────────────────────────────────────────

#[test]
fn memory_header_and_sub() {
    let mut app = App::new();
    goto_memory(&mut app);
    let s = render(&mut app);
    assert!(s.contains("Memory backend"), "missing header\n{s}");
    assert!(
        s.contains("Mnemosyne") && s.contains("embedding provider"),
        "missing sub copy\n{s}"
    );
}

// ── all 3 backends + glyphs ─────────────────────────────────────────────────────

#[test]
fn memory_all_three_backends_present() {
    let mut app = App::new();
    goto_memory(&mut app);
    let s = render(&mut app);
    assert!(s.contains("Ollama"), "missing Ollama\n{s}");
    assert!(s.contains("OpenAI"), "missing OpenAI\n{s}");
    assert!(s.contains("FTS-only"), "missing FTS-only\n{s}");
    assert!(s.contains("OLM"), "missing OLM glyph\n{s}");
    assert!(s.contains("OAI"), "missing OAI glyph\n{s}");
    assert!(s.contains("FTS"), "missing FTS glyph\n{s}");
    // Provider subs
    assert!(s.contains("Local, free, private"), "missing ollama sub\n{s}");
    assert!(s.contains("Cloud, paid, fast"), "missing openai sub\n{s}");
    assert!(
        s.contains("No embeddings, full-text search only"),
        "missing fts sub\n{s}"
    );
}

// ── default selection badges: ★ REC + ▸ SELECTED on FTS-only (#258); ● DETECTED on Ollama ─

#[test]
fn memory_fts_default_badges() {
    // #258: FTS-only is the default → ★ REC + ▸ SELECTED follow it.
    // ● DETECTED stays on the Ollama card (detection indicator, orthogonal to selection).
    let mut app = App::new();
    goto_memory(&mut app);
    // #260: badge is now LIVE — simulate the probe confirming Ollama reachable
    // so the `● DETECTED` indicator renders (default `None` = "… PROBING").
    app.memory_screen.set_ollama_detected(true);
    let s = render(&mut app);
    assert!(s.contains("★ REC"), "missing ★ REC (now on FTS-only default)\n{s}");
    assert!(
        s.contains("▸ SELECTED"),
        "missing ▸ SELECTED (now on FTS-only default)\n{s}"
    );
    assert!(
        s.contains("● DETECTED"),
        "● DETECTED must render on the Ollama card once the live probe confirms reachable\n{s}"
    );
}

// ── STORAGE fields: DB Path always, Embedding Model conditional ─────────────────

#[test]
fn memory_storage_fields_default() {
    let mut app = App::new();
    goto_memory(&mut app);
    let s = render(&mut app);
    assert!(s.contains("S T O R A G E"), "missing STORAGE label\n{s}");
    assert!(s.contains("DB Path"), "missing DB Path field\n{s}");
    assert!(
        s.contains("~/.zeus/mnemosyne.db"),
        "missing DB path default\n{s}"
    );
    // #258: FTS-only is the default → Embedding Model field is HIDDEN (no embeddings).
    assert!(
        !s.contains("Embedding Model"),
        "Embedding Model field must be hidden for FTS-only default\n{s}"
    );
}

#[test]
fn memory_openai_model_default() {
    let mut app = App::new();
    goto_memory(&mut app);
    // ↓ to OpenAI (index 1)
    app.handle_key(KeyCode::Down);
    let s = render(&mut app);
    assert!(
        s.contains("Embedding Model"),
        "missing Embedding Model field (openai)\n{s}"
    );
    assert!(
        s.contains("text-embedding-3-small"),
        "missing openai model default\n{s}"
    );
}

#[test]
fn memory_fts_only_hides_embedding_model() {
    let mut app = App::new();
    goto_memory(&mut app);
    // #258: FTS-only is now the DEFAULT selection — no nav needed.
    let s = render(&mut app);
    // FTS-only selected: DB Path stays, Embedding Model field is gone.
    assert!(s.contains("DB Path"), "DB Path should remain for FTS-only\n{s}");
    assert!(
        !s.contains("Embedding Model"),
        "Embedding Model must be hidden for FTS-only\n{s}"
    );
}

// ── RIGHT panel: disk projection + ollama box ───────────────────────────────────

#[test]
fn memory_disk_projection_rows() {
    let mut app = App::new();
    goto_memory(&mut app);
    let s = render(&mut app);
    assert!(
        s.contains("D I S K   P R O J E C T I O N"),
        "missing DISK PROJECTION header\n{s}"
    );
    assert!(s.contains("1K facts") && s.contains("~12 MB"), "missing 1K row\n{s}");
    assert!(
        s.contains("10K facts") && s.contains("~120 MB"),
        "missing 10K row\n{s}"
    );
    assert!(
        s.contains("100K facts") && s.contains("~1.2 GB"),
        "missing 100K row\n{s}"
    );
    assert!(s.contains("1M facts") && s.contains("~12 GB"), "missing 1M row\n{s}");
}

#[test]
fn memory_ollama_detected_box() {
    let mut app = App::new();
    goto_memory(&mut app);
    // #260: the cyan banner now renders ONLY when the live probe confirms Ollama
    // reachable (default `None`/`Some(false)` → no fabricated banner).
    app.memory_screen.set_ollama_detected(true);
    let s = render(&mut app);
    assert!(
        s.contains("● OLLAMA DETECTED"),
        "missing OLLAMA DETECTED box header\n{s}"
    );
    assert!(
        s.contains("localhost:11434"),
        "missing localhost:11434 in box\n{s}"
    );
}

#[test]
fn memory_right_panel_left_divider() {
    let mut app = App::new();
    goto_memory(&mut app);
    let s = render(&mut app);
    // JSX borderLeft on the 280w right column → a vertical │ rule must render.
    assert!(s.contains('│'), "missing right-panel left divider\n{s}");
}

// ── nav: ↑/↓ select (not step), Right step-advances, ESC=back ───────────────────

#[test]
fn memory_down_selects_not_step() {
    let mut app = App::new();
    goto_memory(&mut app);
    assert_eq!(app.current_step, MEMORY_STEP);
    app.handle_key(KeyCode::Down);
    // Step unchanged — ↓ moved the provider selection, not the step.
    assert_eq!(
        app.current_step, MEMORY_STEP,
        "Down must select provider, not advance step"
    );
    assert_eq!(
        app.memory_screen.selected_id(),
        "ollama",
        "Down from FTS-only default (index 2) wraps to Ollama (index 0)"
    );
}

#[test]
fn memory_right_advances_step() {
    let mut app = App::new();
    goto_memory(&mut app);
    assert_eq!(app.current_step, MEMORY_STEP);
    app.handle_key(KeyCode::Right);
    // Memory is a vertical list (no grid) → Right step-advances to 17 (Skills).
    assert_eq!(
        app.current_step,
        MEMORY_STEP + 1,
        "Right must advance the step on the vertical-list Memory screen"
    );
}

#[test]
fn memory_esc_backs_not_quits() {
    let mut app = App::new();
    goto_memory(&mut app);
    assert_eq!(app.current_step, MEMORY_STEP);
    app.handle_key(KeyCode::Esc);
    assert_eq!(
        app.current_step,
        MEMORY_STEP - 1,
        "ESC must back out one step (not quit)"
    );
}
