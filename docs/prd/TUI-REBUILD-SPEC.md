I have confirmed the actual state: 8 tabs, the theme constants, and the live render path. I now have everything needed to produce the document grounded in real code.

---

# Zeus TUI Rebuild — Drift Report + Ratatui Implementation Spec + Titan Work Breakdown

Build target: faithful ratatui reimplementation of two prototypes — `/Users/mike/zeus-prototypes/zeus-tui-production.jsx` (chat) and `/Users/mike/zeus-prototypes/zeus-tui-onboarding.jsx`. Current code on `origin/TUI`: `crates/zeus-tui/src/{ui.rs (573 lines, live render), theme.rs (40 lines), app.rs (Tab enum L652, TAB_COUNT=8 L1892), onboarding/{mod.rs,render.rs}}`.

This is the root cause of merakizzz's "looks exactly like main": the current code is a *different design* (blood-red `RED=rgb(255,0,60)` palette, rounded borders, 8-tab reverse-video tab bar, 24-col sidebar, pixel-art onboarding) that predates the prototypes. It was never built against these JSX files. The prototypes specify a *new* design system (fire-orange `#ff3c14` on warm-black, square borders, 9-tab underline+fill, ZeusFace mascot, non-blocking queue, 19-step master/detail onboarding). The gap is not polish — it is a full visual-system swap plus several missing components.

---

## PART 1 — DRIFT SUMMARY

### 1A. Chat / Production prototype drift

**Palette is the wrong design system.** Current `theme.rs` is a *blood-red* scheme; the prototype is *fire-orange on warm-black*. Concrete mismatches:
- `BG = rgb(10,0,8)` (purple-black) vs spec `bg = #0a0a0f = rgb(10,10,15)` (warm-black). Backgrounds are tinted the wrong direction.
- No `bg2/bg3` layering. Current has `BG_PANEL rgb(16,8,16)` + `BG_HIGHLIGHT rgb(26,16,32)`; spec needs `bg2 #12100e = rgb(18,16,14)` and `bg3 #1a1610 = rgb(26,22,16)` (warm browns, not purple-greys).
- `ACCENT = FIRE_ORANGE = rgb(255,102,0)` vs spec `accent = #ff3c14 = rgb(255,60,20)`. The brand accent is a different orange (current is too yellow). Spec also needs `accentDim #a0301a`, `accentBright #ff6842`.
- `RED = rgb(255,0,60)` is used as a primary brand color in current; spec reserves red strictly for errors (`#ef4444`).
- Foreground ramp wrong: current `TEXT rgb(160,128,144)` (mauve) vs spec `fg #d4cfc8` (warm grey). Current `DIM rgb(90,64,96)` (purple) vs spec `dim #5a5650`. Current `MUTED rgb(58,32,48)` vs spec `muted #3a3632`.
- Status colors differ: current `GREEN rgb(0,255,136)`, `YELLOW rgb(255,170,0)`, `CYAN rgb(0,210,220)`, `PURPLE rgb(170,0,255)` vs spec `green #22c55e`, `yellow #eab308`, `cyan #06b6d4`, `purple #a855f7`, plus spec adds `blue #3b82f6`, `amber #ffa050`, and `*Dim` fill variants (`greenDim`, `amberDim`, etc.) that current lacks entirely.
- **Rounded borders everywhere** (`BorderType::Rounded`) vs prototype's square/plain terminal-grid feel.

**Missing signature components (the identity elements):**
- **No ZeusFace mascot.** The prototype's defining element — animated ASCII emoticon `(◉‿◉)` with 10 state-frame-sets, state→color, glow, variable speed — does not exist. Current assistant label is just the agent name in fire-orange. Spec requires `(◉‿◉) zeus` as the literal speaker prefix, a face in the input bar, and a face in the streaming footer.
- **No `tool_call` bordered cards.** Current renders tool messages as inline list lines (`tool` label, dim args line, `→ result preview`). Spec requires a bordered card: bg2 + 1px muted border + 2px state-colored *left border*, `⚙ tool_call · <name> args ... <glyph> <status>` header, indented output block on `bg` with `↳ N lines returned`, 5-line collapse + `▾ N more lines` expander, error block.
- **No non-blocking queue UI.** Current shows `(N queued)` only in the status bar. Spec requires the amber `📥 Queued: N message(s) — will fire as turns complete` banner above the input with `Esc cancel last · Ctrl+Esc clear all`, plus the input never blocking (placeholder swaps to `type to queue (input never blocks)…` while streaming).
- **No slash-command overlay.** Current has a centered command-palette popup; spec requires an *inline* slash overlay anchored above the input when input starts with `/`, header `SLASH COMMANDS` (accentDim, letterSpacing 3), filtered rows (`/help /clear /compact /spawn /stop /reset /model`).
- **No block-char context meter with grading text.** Current has an 8-cell `[████░░░░]` bar. Spec wants 10-segment `[▓▓▓▓░░░░░░] 47%` with `▓`/`░` glyphs, green/amber/red thresholds at 60/80, and a `⚠ near limit · /compact` nudge past 80%.
- **No char counter** `{n}/4096` or `↵` glyph in the input bar.
- **No channel-source badge** `↰ discord` pill or **provider badge** pill on messages (current has a `[channel]` prefix and `[provider]` prefix but not as the spec's styled pills).
- **No cooking footer in spec format.** Current shows a placeholder cooking badge; spec wants a dedicated streaming footer row: face(working/thinking) + `cooking │ iter N/8 · M tools · thinking…`.

**Structural drift:**
- **8 tabs, wrong set/order.** Current: `chat · office · pantheon · approvals · settings · tools · memory · channels`. Spec: 9 tabs `chat ▸ · office ◇ · pantheon ◈ · tools ⚙ · memory ▤ · channels ⇌ · approvals ✓ · settings ⊕ · more… ▸▸`. Order differs and the `advanced/more…` tab (gateway to 13 subsystem cards) is entirely absent.
- **Tab active treatment wrong.** Current = fire-orange BOLD + REVERSED. Spec = 2px accent bottom-border + `bg3` fill + bold + accent glyph (no reverse-video).
- **No per-tab glyphs** before names.
- **24-col right sidebar (agents/channels) does not exist in the prototype** chat screen and should be removed — chat is messages + queue + slash + input, full width.
- **TopBar/HintBar content differs.** Spec TopBar: `ZEUS`(tracked 3) · host · ●conn · model · v… · ctx meter · right `Ctrl+K palette │ Ctrl+C quit`. Spec HintBar: tab-specific keycaps + queue indicator + status. Current merges most of this into one status bar.

### 1B. Onboarding prototype drift

- **20 steps, wrong design.** Current `OnboardingStep::total()==20` with a `QuickStart` config-form step inserted; prototype is **19 steps** (`WLCM…DONE`) with mode as a *branch point* not an extra step.
- **Pixel-art / rounded-hexagon chrome.** Current uses rounded borders titled `⬡ Welcome`, a `Gauge` progress bar, and its own `render.rs::colors`. Prototype uses square borders, a **windowed breadcrumb rail** with numbered `✓/⏭/01` chips joined by `›`, and the unified fire-orange theme.
- **No ZeusFace in onboarding.** Prototype puts a reactive face in the TopBar of *every* step, the Welcome greeting, Auth test feedback, and Complete header. Current has none.
- **No master/detail + right-rail layouts.** Prototype steps 03 (provider), 14 (voice), 15 (images), 17 (memory) are 3-pane (list │ detail │ HINTS rail); step 10 (agent) has a live SOUL.md preview pane. Current renders provider/voice/etc. as flat card grids or config forms — no detail rail, no live preview.
- **No inline config-as-TOML preview boxes** (`WILL WRITE TO ~/.zeus/config.toml → model = "anthropic/..."`).
- **No live test affordances** in spec form: TEST CONNECTION with 4 states + latency/model-count readout, SEND TEST per channel, TEST VOICE, TEST ALL BACKENDS on Complete.
- **No detection banners** in spec style (existing-config re-entry "Welcome back Zeus100", existing-workspace fact/session counts, Ollama LIVE FETCH, setup-token detected, port-in-use-with-PID error).
- **No ordered fallback chain builder** (numbered 1/2/3 with up/down/✕ + `[ ]` reorder). Current is a flat card-grid picker.
- **No animated pill toggle switches** (30×16 sliding knob) for gateway features / features step.
- **No StepIndicator windowing** (±4 + endpoints + `···` ellipsis) — current is a 5-step window without the chip-state vocabulary.
- **No mandatory-gate UX** (Talos FORCE-ON banner with "193 tools silently fail" warning; Standard ★REC green-border default in security).
- **No disk-usage projection tables** (workspace, memory steps).
- **No StepHeader / Field / Card / StatusBar reusable component kit** matching the prototype's prop surface (Field with secret SHOW/HIDE + valid ✓/✕ + required `*` + hint + error + options; Card with multiselect/badges/dim/large/extra slot).

---

## PART 2 — RATATUI IMPLEMENTATION SPEC

### 2.0 Shared theme module (build first — both screens depend on it)

Rewrite `crates/zeus-tui/src/theme.rs`. Define exact constants (truecolor required; gate on terminal support, fall back to nearest 256):

```rust
// Backgrounds (warm-black, layered)
pub const BG:   Color = Color::Rgb(10, 10, 15);   // #0a0a0f  app root, code/output, inputs
pub const BG2:  Color = Color::Rgb(18, 16, 14);   // #12100e  bars, panels, cards, sidebars
pub const BG3:  Color = Color::Rgb(26, 22, 16);   // #1a1610  active/selected row, hover

// Neutrals (warm grey ramp)
pub const WHITE: Color = Color::Rgb(240, 236, 230); // #f0ece6  headings, big numbers, selected names
pub const FG:    Color = Color::Rgb(212, 207, 200); // #d4cfc8  body text
pub const DIM:   Color = Color::Rgb(90, 86, 80);    // #5a5650  secondary/help
pub const MUTED: Color = Color::Rgb(58, 54, 50);    // #3a3632  borders, dividers, separators
pub const DARK:  Color = Color::Rgb(42, 36, 32);    // #2a2420

// Accent (FIRE)
pub const ACCENT:        Color = Color::Rgb(255, 60, 20);  // #ff3c14
pub const ACCENT_DIM:    Color = Color::Rgb(160, 48, 26);  // #a0301a  section labels, keycaps
pub const ACCENT_BRIGHT: Color = Color::Rgb(255, 104, 66); // #ff6842  tool name
pub const ACCENT_FAINT:  Color = Color::Rgb(64, 16, 8);    // #401008  selected-card fill

// Semantic + Dim fill variants
pub const GREEN:  Color = Color::Rgb(34, 197, 94);   pub const GREEN_DIM:  Color = Color::Rgb(26, 74, 46);
pub const YELLOW: Color = Color::Rgb(234, 179, 8);   pub const YELLOW_DIM: Color = Color::Rgb(107, 90, 16);
pub const BLUE:   Color = Color::Rgb(59, 130, 246);  pub const BLUE_DIM:   Color = Color::Rgb(26, 42, 74);
pub const CYAN:   Color = Color::Rgb(6, 182, 212);   pub const CYAN_DIM:   Color = Color::Rgb(22, 78, 99);
pub const RED:    Color = Color::Rgb(239, 68, 68);   pub const RED_DIM:    Color = Color::Rgb(74, 26, 26);
pub const AMBER:  Color = Color::Rgb(255, 160, 80);  pub const AMBER_DIM:  Color = Color::Rgb(90, 48, 16);
pub const PURPLE: Color = Color::Rgb(168, 85, 247);  pub const PURPLE_DIM: Color = Color::Rgb(58, 26, 74);
```

Helpers to add:
- `fn section_label(text) -> Line` — uppercase, `ACCENT_DIM`, `BOLD`, letter-spacing emulated by joining chars with a thin space (`'\u{2009}'`) or a single space; fontSize-9 collapses to one row. This is the recurring `SLASH COMMANDS` / `FLEET STATS` style.
- `fn keycap(k) -> Span` — `ACCENT_DIM` + `BOLD`.
- `fn left_border_block(color)` — produce a `Block` with only the left border, drawn as a `▏`/`│` column in `color` (ratatui `Borders::LEFT` with a `Style`); used by tool cards, approval cards, mission rows, etc. To get a *2px* feel, render a literal full-block `▌` column in the state color as the first cell of the card region.
- `fn block_meter(pct: u8) -> Vec<Span>` — 10 segments, `▓`*filled + `░`*empty, color by threshold (`<60` green, `<80` amber, else red).
- Glow is dropped (terminals can't blur); approximate with `Modifier::BOLD` + the state color only.

Borders: replace all `BorderType::Rounded` with `BorderType::Plain` (square) project-wide in the new screens.

---

### 2.1 CHAT SCREEN — ratatui spec

**Top-level layout** (`ui.rs::render`, replace existing vertical split):
```
Layout::vertical([
  Constraint::Length(1),  // TopBar
  Constraint::Length(1),  // TabBar
  Constraint::Min(0),     // Content (active tab)
  Constraint::Length(1),  // HintBar
])
```
Overlay: CommandPalette rendered last via `Clear` over a centered `Rect` (width 60, maxHeight ~14 rows), `BG2` + 1px `ACCENT` border, accent-bold title. Slash overlay is NOT a centered popup — it is inline (see input region).

**State additions to `App`** (in `app.rs`):
```rust
struct FaceState { kind: FaceKind, frame: usize }   // kind: Ready/Thinking/Working/Tool/Success/Error/Alert/Queued/Listening/Sleeping
queue: Vec<String>,            // pending messages (non-blocking)
expanded_tool: Option<usize>,  // which tool_call card is expanded
cooking_iter: u8, cooking_tools: u8,
tick: u64,                     // animation clock, advanced ~6-8 Hz by the event loop
gateway_version: String, hostname: String, ctx_pct: u8,
```
Tab enum → 9 variants in spec order: `Chat, Office, Pantheon, Tools, Memory, Channels, Approvals, Settings, Advanced`. `TAB_COUNT = 9`. `Advanced` holds a sub-view enum (`AdvSub::{Landing, Agents, Skills, Mcp, Projects, Canvas, Voice, NodeComms, VectorStores, Economy, Extensions, KnowledgeGraph, Spawner, Deploy}`).

**TopBar** (`render_top_bar`, 1 row, `BG2`): build a single `Line` of `Span`s separated by `│`(MUTED):
`ZEUS`(ACCENT,BOLD, chars space-joined for tracking) · host(DIM) · `●`+state (green/amber/red by conn) · model(FG) · `v{ver}`(DIM) · `ctx`(DIM)+`block_meter(pct)`+`{pct}%`+optional `⚠ near limit · /compact`(AMBER) · spacer · `Ctrl+K palette`(DIM) `│` `Ctrl+C quit`(DIM). Use `Layout` with a left chunk and a right-aligned chunk, or compute spacer width manually.

**TabBar** (`render_tab_bar`, 1 row, `BG2`): iterate the 9 tabs. Each tab = `glyph`(ACCENT if active else MUTED) + ` ` + `name`(active: FG+BOLD; inactive: DIM). Active tab: set the cell `bg = BG3` for the tab's span range and render a `▔` (upper-block) underline is impossible in 1 row — instead encode active via `bg=BG3` + accent glyph + bold (the 2px underline collapses; document this as the terminal equivalent). Unread badge: ` (N) ` pill, `bg=ACCENT, fg=BG` (approvals tab uses `bg=AMBER`). After spacer, hint `Tab to switch · ⇧Tab back · : palette`(MUTED).

**HintBar** (`render_hint_bar`, 1 row, `BG2`, top border emulated by the row above or a `Block` with `Borders::TOP`): left = tab-specific hints, each `keycap(k)` + ` ` + label(DIM), joined by `  `. Then spacer. Queue indicator if `queue` nonempty: `●`(AMBER) + `{n} queued`(AMBER) + `│`. Right: status string(DIM). Chat hints: `↵ send`, `↑↓ scroll`, `Esc clear/cancel`, `Ctrl+L clear`, `/ commands`, `e expand`. Chat status: `ready` or `cooking · iter N/8 · M tools`.

**Chat content region** (`render_chat`):
```
Layout::vertical([
  Constraint::Min(0),                                 // messages
  Constraint::Length(queue_banner_h),                 // 0 or 1, queue banner
  Constraint::Length(slash_h),                        // 0 or up to 8, slash overlay
  Constraint::Length(3),                              // input block
])
```

*Messages* (`render_messages`): a manually word-wrapped list on `BG`, padding 1 col each side. For each `ChatMessage`, emit a fixed-width speaker column (8–10 cells) + body:
- User: `▸ user`(CYAN,BOLD) + body(FG). Optional `↰ {source}` pill (`bg=BG2, fg=DIM`, 1px MUTED border emulated by surrounding brackets/spaces).
- Assistant: `(◉‿◉) zeus`(ACCENT,BOLD) — the literal current face frame for the assistant resting state, or just the static `(◉‿◉)` glyph — + body(FG). Optional provider pill (`bg=BG2, fg=DIM`).
- tool_call card: render as a sub-block. Left column = a `▌` full-block in the state color (`running`→AMBER, `success`→GREEN, `failed`→RED, `awaiting_approval`→YELLOW, else DIM). Card body on `BG2`: header `⚙`(AMBER) `tool_call`(AMBER,BOLD) `·` `{name}`(ACCENT_BRIGHT,BOLD) `{args}`(DIM) … right-aligned `{glyph} {status}`(state color; running appends animated dots from `tick`). Output block (if present), indented: meta `↳ {n} lines returned`(MUTED) + `· expand: e` if truncated; body on `BG`, 1px MUTED border, `whiteSpace pre` (do NOT wrap — truncate to width), first 5 lines collapsed; if `expanded_tool == Some(idx)` show up to ~30 lines with scroll. Truncation row `▾ {n-5} more lines (press e)`(ACCENT,BOLD). Error block: `✕ {error}`(RED).
- Streaming footer (when last msg streaming): row with face (Working if tools running else Thinking) + `cooking`(label) `│` `iter {iter}/8`(ACCENT,BOLD) `· {tools} tools`(FG) `· thinking`(AMBER,italic)+dots.
- Auto-scroll: keep `scroll_offset` pinned to bottom unless user scrolled up; when scrolled, replace bottom row with `↑ scrolled up — NN% — Esc to jump to bottom`.

*Queue banner* (`render_queue_banner`, height 1 when `queue` nonempty): `BG = AMBER_DIM`, top border AMBER. `📥 Queued: {n} message(s)`(AMBER,BOLD) `— will fire as turns complete`(DIM) … `Esc cancel last · Ctrl+Esc clear all`(AMBER keycaps).

*Slash overlay* (`render_slash`, height = min(8, matches+1) when `input` starts with `/`): `BG3`, top border ACCENT_DIM. Row 0 = `section_label("SLASH COMMANDS")`. Then filtered command rows: command(ACCENT,BOLD, width 12) + description(DIM). Commands: help, clear, compact, spawn, stop, reset, model.

*Input block* (`render_input`, 3 rows, `BG2`, top border MUTED): horizontal `Layout` — face cell (width ~6) + `│`(MUTED) + input(flex, FG on transparent, horizontal-scroll window, `█` cursor at `cursor_pos`) + right cell (`{n}/4096`(MUTED) + `↵`(DIM)). Face state: `isStreaming`→Working, `!queue.empty`→Queued, `!input.empty`→Listening, else Ready. Placeholder: streaming→`type to queue (input never blocks)…`, else `message…` (render in MUTED when input empty).

**ZeusFace** (`src/zeus_face.rs`, NEW): 
```rust
pub enum FaceKind { Ready, Listening, Thinking, Working, Tool, Success, Error, Alert, Queued, Sleeping }
pub fn frames(k: FaceKind) -> &'static [&'static str] { /* the frame arrays from spec §7 */ }
pub fn color(k: FaceKind) -> Color { /* Ready→ACCENT, Thinking/Working/Queued→AMBER, Tool→CYAN, Success→GREEN, Error→RED, Alert→YELLOW, Listening→CYAN, Sleeping→DIM */ }
pub fn render(k: FaceKind, tick: u64, label: Option<&str>) -> Line {
    let f = frames(k); let frame = f[(tick as usize / SPEED_DIV) % f.len()];
    // Span(frame, Style::default().fg(color(k)).bold()) + optional italic label in same color
}
```
`SPEED_DIV` varies by state to emulate 200/350/600ms (faster divisor for Working/streaming). The frame arrays are listed verbatim in chat spec §7 — copy them exactly. Glow → BOLD only. Pad frames to a fixed `minWidth` (pad with spaces) to prevent jitter.

**AnimatedDots**: `["", ".", "..", "..."][(tick/N) % 4]`.

**Key handling** (no sidebar; remove `render_sidebar`):
- `Ctrl+K`/`:` (when not in input) → palette; `Esc` closes palette, else clears input / cancels last queue item.
- `Tab`/`Shift+Tab` (not in input) → cycle 9 tabs (wrap); leaving Advanced resets sub-view.
- Enter: if streaming → push to `queue`; else append user msg + start streaming.
- `Ctrl+Esc` → clear queue. `e` → toggle `expanded_tool` for focused card. `Ctrl+L` → clear chat.

**Command palette** (`render_palette`, overlay): `Clear` + centered `Rect`, `BG2`, 1px ACCENT border, title `▸ search`. Rows: type-colored keycap (tab=ACCENT, tool=AMBER, slash=CYAN, skill=YELLOW, settings=PURPLE, advanced=GREEN) + uppercase type tag(MUTED, width ~9) + label + first-row `↵`. First row highlighted `bg=BG3` + accent left-block. Footer `↑↓ navigate · ↵ execute · Esc close`.

---

### 2.2 ONBOARDING SCREEN — ratatui spec

Rewrite `onboarding/mod.rs` + `render.rs`. Drop the local color module; use the shared `theme.rs`. Drop the `Gauge`, the `⬡` hex titles, rounded borders. **19 steps**, not 20 (remove the inserted `QuickStart` config step; `Mode` is a branch point, not a config form).

**Step enum** (19): `Welcome, Mode, Provider, Auth, Model, Fallback, Channels, ChanConfig, Gateway, Agent, Workspace, Security, Features, Voice, Images, Orchestration, Memory, Skills, Complete`. Keep `SignalPair`/`WhatsAppPair` as conditional sub-screens reachable from ChanConfig (defined-but-may-be-wired; prototype leaves SignalPair unwired but the design intends it). Each step carries `{code: &str (4-letter, e.g. "WLCM"), name, required: bool}`.

**Persistent frame** (vertical):
```
Layout::vertical([
  Constraint::Length(1),  // TopBar
  Constraint::Length(1),  // StepIndicator breadcrumb rail
  Constraint::Min(0),     // step content
  Constraint::Length(1),  // StatusBar
])
```

**TopBar** (`render_top_bar`): `ZEUS`(ACCENT, tracked) `│` `ONBOARDING`(ACCENT_DIM) `│` `Step {n} of 19`(FG) `│` `{code}`(DIM) … spacer … `ZeusFace`(small, reactive) … `●`(GREEN)+`config draft` … `~/.zeus/config.toml`(ACCENT_BRIGHT).

**StepIndicator** (`render_breadcrumb`, 1 row, `BG2`): windowed — show steps within ±4 of current plus step 0 and step 18; gaps render `···`(MUTED); join with `›`(MUTED). Each step = chip `[NN]` + name. Chip states: current → `bg=ACCENT, fg=BG`; completed → `fg=ACCENT` + `✓`; skipped → `fg=MUTED` + `⏭`; pending → `fg=MUTED` + zero-padded number. Current name FG+BOLD, others DIM.

**StatusBar** (`render_status_bar`, 1 row, `BG2`, top border): left = keybind legend `● onboard`(GREEN) `↑↓ Navigate` `↵ Select` `Tab Field` + per-step extraKeys (`t Test`, `[ ] Reorder`, `/ Filter`, `Sp Toggle`) `? Help` `Esc Back`. Right = validation dot `● VALID`(GREEN) / `● INCOMPLETE`(YELLOW) / `● READY`(DIM) `│` `SKIP →`(if optional) `│` `CONTINUE → {next:02}/19`(ACCENT if allowed else MUTED).

**Reactive face state** (`compute_face_state`): auth+testing→Thinking; auth+error→Error; auth+success→Success; complete→Success; validation incomplete→Listening; step 0→Ready; else→Working.

**Reusable component kit** (`onboarding/components.rs`, NEW — build before steps):
- `StepHeader(rect, code, n, title, subtitle)` — left-rail header: `STEP {n:02}/19 │ {code}`(ACCENT_DIM, tracked) + title(WHITE,BOLD) + subtitle(DIM).
- `Field(rect, label, value, opts)` — 140px-equiv label column (width ~16, ACCENT_DIM caps) + `│`(MUTED) + input. `opts`: `secret`(mask + `SHOW/HIDE`), `valid: Option<bool>`(green ✓ / red ✕), `required`(`*`), `hint`(ℹ italic DIM), `error`(✕ RED), `options`(↓). Focused → `border=ACCENT, bg=BG2`.
- `Card(rect, glyph, name, sub, opts)` — glyph chip (2–3 letter, bordered in entity color, inverts to filled when selected) + name + sub. Always a 2px colored left-block. Modes: single-select(`▸`+`SELECTED`), `multiselect`(checkbox `[✓]`/`[ ]`), badges(`FEATURED` AMBER, `★ REC` GREEN, `● DETECTED` GREEN, custom), `dim`, `large`, `extra` slot. Selected → `bg=ACCENT_FAINT, border=ACCENT`.
- `pill_toggle(rect, on)` — 30×16-equiv: `[●  ]`/`[  ●]` sliding knob, ACCENT when on, DARK track when off (animate knob position via tick if desired).
- `config_preview(rect, lines)` — `section_label("WILL WRITE TO ~/.zeus/config.toml")` + boxed TOML lines (key in DIM, value in ACCENT_BRIGHT) on `BG`.
- `detection_banner(rect, color, text)` — left-accent strip (AMBER for existing-config/workspace, CYAN for Ollama/detected) with the icon+message.

**Per-step renderers** (each a `fn render_<step>(f, rect, state)`):
- **Welcome (WLCM)**: centered. 6-line ASCII `ZEUS` logo with per-line gradient (`LOGO_COLORS`: ACCENT→ACCENT_BRIGHT→ACCENT_DIM→MUTED). Tagline `O P E R A T I N G   S Y S T E M`(ACCENT_DIM, tracked). Greeting box (`BG2` border) with Ready face + italic quote. Conditional existing-config banner (`↻ EXISTING CONFIG DETECTED · Welcome back, {name}`). INITIATE card (width ~64): header `▸ INITIATE` + `v0.4.7 · …`, body copy, 3-row stat list (`19 STEPS / 10 req,9 opt`, `~5 MIN / QuickStart`, `~25 MIN / Full`), footer `↵ Continue · N Exit · build {sha} · main`.
- **Mode (MODE)** branch point: 3 large cards side-by-side (`Layout::horizontal` thirds) — QuickStart(QS,GREEN,~3min,"1 step left"), Full(FU,ACCENT,~25min,"17 steps left"), Custom(CU,CYAN,"varies"). Each: 48-equiv glyph tile + name + sub + bottom TIME/STEPS stat pair. Selected → `▸ SELECTED` top-right, ACCENT_FAINT fill. NOTE bar: "skipped sections via `zeus onboard --resume`". Mode prunes the visited step set.
- **Provider (PROV)** 3-pane (`Layout::horizontal([Length(40), Min(0), Length(22)])`): left = StepHeader + scrolling provider Cards (12 providers; minimax FEATURED amber, ollama DETECTED) + footer "12 providers · Sorted by usage". Center = selected provider detail: glyph tile, name(large)+pills, FLAGSHIP/PRICING/KEY FORMAT row, config_preview(`model = "{id}/{flagship}"`), NEXT note. Right = HINTS rail: RECOMMENDATIONS list (Reasoning→Anthropic, Multimodal→OpenAI, Throughput→MiniMax, Local→Ollama, Speed→Groq), color-dotted.
- **Auth (AUTH)** validation-gated: heading `Authenticate with {provider}`. Mode tabs (API Key / Setup Token / Browser OAuth, underline-active). API Key: secret Field (format-validated vs `keyFmt` prefix → ✓/✕) + TEST CONNECTION button with 4 states (idle `▸ TEST CONNECTION`, `▸ TESTING…`+Thinking face "probing endpoint", `✓ AUTH OK`+Success face "● /v1/models 200 · 184ms · 47 models", `✕ AUTH FAILED`+Error face "✕ 401"). Setup Token: amber detected banner + token Field. OAuth: 4-step progress checklist (✓/▸/○). config_preview `[credentials] {id}_api_key = "***{last4}"`. `t`=Test. Valid iff api_key present.
- **Model (MODL)** validation-gated, provider-dependent: vertical radio list. Per-provider catalogs (anthropic Opus4.7★/Sonnet/Haiku, openai GPT-4o★/4o-mini/o1-pro, minimax abab-7★/6.5s; fallback=anthropic). Row: radio dot + name + model-id(DIM) + `★ RECOMMENDED` + sub; right CONTEXT(large ACCENT)+PRICING. Ollama special: CYAN `● LIVE FETCH` banner "models from localhost:11434/api/tags". Valid iff selected.
- **Fallback (FLBK)** optional, 2-pane: left AVAILABLE checklist (≤6 non-primary providers, toggled→ACCENT_FAINT). Right (width ~40) `FALLBACK CHAIN (n)` ordered list — empty: dashed "No fallbacks…"; filled: numbered ACCENT-bordered rows + up/down/✕ + reorder hint. SUGGESTED box. `[` `]` reorder.
- **Channels (CHAN)** optional multiselect: two labeled groups (Cloud APIs: telegram/discord/slack/email; Phone-paired: imessage/whatsapp/signal/matrix) each a 2-col Card grid (checkbox + glyph + name + sub + sdk metadata). Right (width ~30) `SELECTED (n)` live list; empty "console-only". Default toggled: discord+telegram. `Sp` toggle.
- **ChanConfig (CCFG)** conditional: one bordered card per selected channel (left-accent in channel color), header glyph+name+sdk+status pills. Field sets per `fieldsByChannel` (telegram: API ID/Hash/Phone; discord: Bot Token/Channel ID; slack: Bot/App Token; email: SMTP Host/Port587/User/App Password; imessage: none+cyan note; whatsapp: Phone ID/Access Token+QR warn; signal: none+QR warn; matrix: Homeserver/User/Password). SEND TEST per channel (▸ SEND TEST → SENDING… → ✓ DELIVERED).
- **Gateway (GTWY)**: BIND (Host field + Port field with live "Port 8080 in use by PID 47291" error), FEATURES (3 pill_toggles: Agent Loop on, WebUI on, MCP off), INSTALL AS SERVICE (4-col Cards: launchd MAC ★rec, systemd LIN dim, rc.d BSD dim, Manual; selected → `WILL INSTALL {path}`).
- **Agent (AGNT)** 2-pane: left persona Card grid (Coordinator/Engineer/Creative/Sysadmin/Analyst/Custom, each glyph+sub+tone) + IDENTITY Fields (Agent Name auto `zeus{tld}`, Role=persona, Tone=persona tone). Right (width ~40) live `SOUL.MD PREVIEW` markdown (`# {name}`, `## Role`, `## Tone`, `## Guiding Principles` varying by persona). Footer "writes to ~/.zeus/workspace/SOUL.md".
- **Workspace (WKSP)**: existing-workspace banner (amber, "2,847 facts · 147 sessions · last 2 min ago" + USE EXISTING / START FRESH buttons). PATHS Fields (Workspace, Sessions, Mnemosyne DB). DISK USAGE PROJECTION 3-col stat grid.
- **Security (SECR)**: 4-col large Cards (Strict STR red, Standard STD amber ★REC green-border, Permissive PRM yellow, Custom CST) — each glyph tile + name + italic sub + BLOCKED list (✕ ≤3). Note `[aegis] level = "{id}"`.
- **Features (FEAT)** optional: Talos mandatory banner (`⚠ MACOS GATE — TALOS IS MANDATORY`). Toggle list (talos force-on+FORCE-ON pill, nous, mnemosyne, hermes, athena, browser, voice, skills) — each pill_toggle + name + desc + `● ON`(GREEN)/`○ OFF`(MUTED). Mandatory rows non-clickable, accent-bordered.
- **Voice (VOIC)** 2-pane: left VOICE_PROVIDERS Cards (ElevenLabs, OpenAI TTS, Cartesia, Custom Endpoint, Skip). Right (width ~42) CREDENTIALS (API Key secret, Voice ID; Custom adds Base URL) + `▸ TEST VOICE`; none → yellow "⚠ NO VOICE CONFIGURED".
- **Images (IMGS)** 2-pane: left IMAGE_PROVIDERS Cards (OpenAI GPT Image, Google NanoBanana, BFL Flux, OpenAI-compat URL, Automatic1111 URL, Skip). Right CONFIG (Base URL, API Key, Model); a1111 special: Steps field hint "⚠ Z-Image Turbo: must be 1".
- **Orchestration (ORCH)**: 3-col Cards (All-on ALL ACCENT ★REC, Heartbeat-only HB amber, Disabled OFF dim). Conditional HEARTBEAT TIMING Fields (Interval 300s, Quiet Start 23, Quiet End 8) hidden when disabled.
- **Memory (MNEM)** 2-pane: left MEMORY_PROVIDERS Cards (Ollama local ★REC+DETECTED, OpenAI, FTS-only) + STORAGE Fields (DB Path, Embedding Model hidden for FTS). Right (width ~30) DISK PROJECTION table (1K→12MB … 1M→12GB) + cyan "● OLLAMA DETECTED".
- **Skills (SKIL)** filterable: `/` filter input + category tabs (All + Productivity/Dev/Marketing/Security/Research with counts) + `{installed} selected · {total} available`. 2-col scrolling grid of skill rows (checkbox + name + ★REC + desc + category tag). Default installed: calendar-pro, git-flow, ci-watch, secret-scan, deep-synth. Suppress Enter-advance so `/` typing works.
- **Complete (DONE)** 2-pane: header large face (Ready→Working→Success per test phase) + divider + "✓ Configuration complete". Summary list (14 rows, one per subsystem) — status dot + name + value + pill `✓ READY`/`⏭ SKIPPED`/`✕ ERROR`, left border green/dim/red. Buttons `▸ TEST ALL BACKENDS` (→ TESTING… → ✓ ALL PASSED) + `▸ AWAKEN ZEUS`(solid ACCENT). Right rail (width ~36) NEXT STEPS command list + "SUMMARY SAVED → ~/.zeus/onboarding-summary.md".

**State machine**: `advance()` adds current to `completed` set + idx++; `skip()` (optional only) adds to `skipped` + idx++; `back()` idx-- (no clear). Validation gates: only Auth (needs api_key) and Model (needs selection). Keyboard: Enter=advance (unless incomplete), Esc=back, ↑↓/←→=navigate (cyclic per-step list), Space=toggle focused (channels/features/skills/fallback), s=skip, t=test, ?=help; input-field-aware (when focus in a Field, only Enter+Tab+Esc pass through; Skills suppresses Enter).

---

## PART 3 — WORK BREAKDOWN FOR TWO TITANS

Both titans branch off `origin/TUI`. **Commit 0 (shared, whoever starts first, the other rebases):** rewrite `crates/zeus-tui/src/theme.rs` to the new palette (§2.0) + add helpers (`section_label`, `keycap`, `block_meter`, `left_border_block`, `pill_toggle` color helpers) + flip `BorderType::Rounded`→`Plain`. This is the dependency root — do it first, get it merged, then both build on it. Per the fleet workflow, titans code on feature branches; Zeus100 reviews+merges; merakizzz deploys.

### Titan A — zeus-spark → CHAT / PRODUCTION (`feat/tui-chat-prototype`)

| # | Commit | Scope |
|---|---|---|
| A1 | `feat(tui): new theme palette + helpers` | Commit 0 (if A starts). theme.rs rewrite, helpers, square borders. Cargo green. |
| A2 | `feat(tui): ZeusFace mascot component` | New `src/zeus_face.rs`: `FaceKind`, verbatim frame arrays from chat spec §7, `color()`, `render(kind, tick, label)`, `AnimatedDots`. Add `tick` clock to event loop (~6-8 Hz). Unit test: each FaceKind returns non-empty padded frames. |
| A3 | `feat(tui): 9-tab bar + topbar + hintbar` | Tab enum→9 variants (incl. Advanced+`AdvSub`), `TAB_COUNT=9`. Rewrite `render_top_bar` (ZEUS tracked, ctx block_meter, conn dot), `render_tab_bar` (glyphs, bg3 active, unread/approval pills), `render_hint_bar` (tab-specific keycaps + queue indicator + status). |
| A4 | `feat(tui): chat messages + tool_call cards` | Rewrite `render_messages`: speaker columns (`▸ user` cyan, `(◉‿◉) zeus` accent), channel/provider pills, **tool_call bordered cards** (left-block state color, `⚙ tool_call · name`, output block on BG, 5-line collapse + `▾ more` + `e` expand), error block, streaming footer (face + `cooking │ iter N/8 · M tools`). **Remove `render_sidebar`** (24-col sidebar deleted). |
| A5 | `feat(tui): non-blocking queue + slash overlay + input` | Add `queue: Vec<String>`; Enter-while-streaming pushes to queue; queue banner (amber, `📥 Queued: N`); inline slash overlay above input (`SLASH COMMANDS`, filtered cmds); rewrite `render_input` (face cell + `│` + input + `{n}/4096 ↵`, placeholder swap). `Esc`/`Ctrl+Esc` cancel logic. |
| A6 | `feat(tui): command palette + Advanced tab landing` | `Clear`-based palette overlay (accent border, type-colored rows, `Ctrl+K`/`:`). Advanced tab: 13 subsystem cards grid + breadcrumb sub-view nav. |

### Titan B — zeus-freebsd → ONBOARDING (`feat/tui-onboarding-prototype`)

| # | Commit | Scope |
|---|---|---|
| B1 | (rebase on A1 theme, or author Commit 0 if first) | Consume new theme. No duplicate palette. |
| B2 | `feat(onboarding): 19-step frame + breadcrumb + statusbar` | Rewrite `onboarding/mod.rs` step enum to **19 steps** (remove inserted QuickStart config step; Mode=branch point). New `render_top_bar` (face + step counter + code), `render_breadcrumb` (windowed ±4 + endpoints + `···`, ✓/⏭/NN chips), `render_status_bar` (keybind legend + validation dot + CONTINUE→NN/19). Drop Gauge, `⬡` titles, local color module. |
| B3 | `feat(onboarding): reusable component kit` | New `onboarding/components.rs`: `StepHeader`, `Field` (secret/valid/required/hint/error/options), `Card` (multiselect/badges/dim/large/extra + 2px left-block), `pill_toggle`, `config_preview`, `detection_banner`. Reactive `compute_face_state`. |
| B4 | `feat(onboarding): steps 01-09 (welcome..chanconfig)` | Welcome (ASCII logo gradient, INITIATE card), Mode (3 branch cards), Provider (3-pane+HINTS rail), Auth (mode tabs + TEST 4-states + config_preview), Model (radio list + Ollama LIVE FETCH), Fallback (ordered chain builder), Channels (grouped multiselect), ChanConfig (per-channel cards + SEND TEST), Gateway (toggles + port-in-use + service cards). |
| B5 | `feat(onboarding): steps 10-19 (agent..complete)` | Agent (persona + live SOUL.md preview), Workspace (existing banner + disk projection), Security (4 cards + ★REC), Features (Talos mandatory gate + toggles), Voice/Images/Orchestration/Memory (2-pane detail rails + projections), Skills (filter + category tabs), Complete (summary list + TEST ALL + AWAKEN). |
| B6 | `feat(onboarding): keyboard model + state machine + SignalPair` | advance/skip/back, validation gates (auth+model only), input-field-aware keys, Space toggle, `s`/`t`/`/` keys, mode-branch step pruning, conditional SignalPair/WhatsAppPair QR sub-screens. |

### DESIGN-FIDELITY GATE (mandatory per cut — compile+tests are NOT sufficient)

The last attempt drifted because cargo-green let a wrong design ship. Each commit must pass this gate **before** the titan claims it done, and Zeus100 re-runs it at the merge gate:

1. **Palette assertion test** (both titans): a `#[test]` asserting the exact `Color::Rgb` triples for `BG, BG2, BG3, ACCENT, ACCENT_DIM, ACCENT_BRIGHT, FG, DIM, MUTED, GREEN, YELLOW, CYAN, AMBER, RED, BLUE, PURPLE` match §2.0. This mechanically blocks reverting to the blood-red palette. No `rgb(255,0,60)` as a primary anywhere outside error semantics; grep-assert `BorderType::Rounded` count == 0 in the new screens.
2. **Glyph/string-presence test**: assert the rendered buffer (use `ratatui::backend::TestBackend` + `Terminal::draw`) contains the load-bearing literal glyphs/strings for that screen. Chat: `(◉‿◉) zeus`, `⚙ tool_call`, `▓`/`░` in the meter, `📥 Queued:` when queue nonempty, `SLASH COMMANDS`, `{n}/4096`, the 9 tab names+glyphs. Onboarding: `Step n of 19`, `WILL WRITE TO ~/.zeus/config.toml`, `›` breadcrumb separators, `✓`/`⏭` chips, `TEST CONNECTION`, `SOUL.MD PREVIEW`, `TALOS IS MANDATORY`. These are falsifiable buffer assertions, not eyeballing.
3. **TestBackend snapshot**: render each screen at a fixed size (e.g. 120×40) to a buffer and commit the snapshot string under `crates/zeus-tui/tests/snapshots/`. The reviewer diffs the snapshot against a hand-checked golden. Layout regressions (sidebar reappearing, wrong tab count, missing queue banner) surface as snapshot diffs.
4. **Prototype side-by-side checklist** (in the PR body): the titan pastes, per sub-component, a line citing the JSX prototype line range and the matching ratatui fn, with a ✓ that the structural element exists (e.g. "tool_call card left-border state colors — jsx L399-441 → render_tool_card() — ✓ 5 status colors mapped"). Zeus100 spot-checks 3 of these against `git show` of both the JSX and the rust before merge — chain-verify, no partial-grep trust.
5. **No-sidebar / tab-count invariants** (chat): assert `TAB_COUNT == 9`, assert no `render_sidebar` symbol remains. (onboarding): assert `total() == 19`.

Gate failure = the cut is held, not merged, regardless of cargo green. Per review-only rule these are titan cuts (features); Zeus100 reviews+merges, never writes the code; merakizzz deploys manually.

---

Key file paths: `/Users/mike/Zeus/crates/zeus-tui/src/theme.rs` (rewrite), `/Users/mike/Zeus/crates/zeus-tui/src/ui.rs` (chat rewrite), `/Users/mike/Zeus/crates/zeus-tui/src/zeus_face.rs` (new), `/Users/mike/Zeus/crates/zeus-tui/src/app.rs` (Tab enum L652, TAB_COUNT L1892), `/Users/mike/Zeus/crates/zeus-tui/src/onboarding/mod.rs` (rewrite to 19 steps), `/Users/mike/Zeus/crates/zeus-tui/src/onboarding/render.rs` (rewrite, drop local colors), `/Users/mike/Zeus/crates/zeus-tui/src/onboarding/components.rs` (new). Prototypes: `/Users/mike/zeus-prototypes/zeus-tui-production.jsx`, `/Users/mike/zeus-prototypes/zeus-tui-onboarding.jsx`.
