# Zeus TUI — Full Rebuild: Implementation Plan & Roadmap

**Branch:** `tui-rebuild` (lands to `main` ONLY on merakizzz sign-off)
**Design source of truth:** `docs/zeus-tui-onboarding.jsx` (onboarding) + `docs/zeus-tui-production.jsx` (production). 100% to these — not patched, not "inspired by," exact.
**Backend:** the gateway REST/WS API (`/v1/...`). The new UI is a thin render layer over `api.rs`.
**Scope:** BOTH the onboarding flow AND the main (production) TUI.

---

## 0. Non-negotiable principles (every failure this rebuild prevents)

1. **From scratch.** No old render code is carried over or patched. The branch starts as a compiling empty shell (Phase 0).
2. **One render contract.** Every screen is drawn by a single `frame()` helper that paints an **opaque full-screen background first** (`bg = #0a0a0f`), then chrome, then content. This structurally kills the bleed-through / transparency class of bug.
3. **The gate is merakizzz's LIVE screenshot, per screen.** Nothing is "done" until he confirms the live render matches the prototype. NO TestBackend-snapshot "verified" (snapshots render in isolation and cannot catch bleed). NO code-read "looks right."
4. **Verify the binary, not the build.** Before any "still broken" debugging: `zeus --version` SHA must equal the branch tip; `which zeus` must be `/usr/local/bin/zeus`. (Stale/shadowed binaries cost us a full day.)
5. **Substrate over worktree.** Read truth via `git show <branch>:<path>` — never a local checkout that may be stale.

---

## 1. Architecture — mirror the prototype's component structure

The JSX is already cleanly componentized. The Rust mirrors it 1:1.

- **`theme.rs`** — the exact `C` palette from the JSX:
  `bg #0a0a0f · bg2 #12100e · bg3 #1a1610 · fg #d4cfc8 · dim #5a5650 · muted #3a3632 · accent #ff3c14 (fire) · accentDim #a0301a · accentBright #ff6842 · green #22c55e · yellow #eab308 · blue #3b82f6 · cyan #06b6d4 · red #ef4444 · amber #ffa050 · purple #a855f7 · white #f0ece6` + status colors (ready/listening/thinking/tool/success/error/alert/queued/sleeping).
- **Shared widgets** (one module each, mirroring the JSX components):
  `TopBar`, `StepIndicator`, `StepHeader`, `Field`, `Card`, `StatusBar`/`HintBar`, `TabBar`, `ZeusFace`, `CommandPalette`.
- **`frame.rs`** — the render contract (opaque bg → chrome → body). Onboarding and production both go through it.
- **Onboarding:** a 19-variant step state machine + one render fn per step.
- **Production:** app state + tab dispatch (PRIMARY + ADVANCED) + one render fn per tab.
- **`api.rs`** — KEPT (gateway HTTP client). New UI calls it; no HTTP rewrite.

---

## 2. Phase 0 — Clean slate (spark, commit 1) — IN PROGRESS

Purge the old TUI implementation to a **compiling empty shell**:
- **Delete:** `onboarding/`, `ui.rs`, `screens/`, `tabs/`, `office/`, `pantheon/`, `command_palette.rs`, `slash_overlay.rs`, `markdown.rs`, `markdown_stream.rs`, `diff_viewer.rs`, `zeusface.rs`, `chat_tests.rs`, the old render loop in `lib.rs`.
- **Keep:** `Cargo.toml`, `api.rs`, `theme.rs`, `crash_log.rs`, minimal `main.rs`.
- **Done when:** `cargo build -p zeus-tui` compiles to a shell with zero old render code. Gated by Zeus100.

---

## 3. Phase 1 — Onboarding (19 screens → `zeus-tui-onboarding.jsx`)

Build order = prototype order; gate each on a live screenshot before the next. Worst-first exception: do **Channels** and **Workspace** early (merakizzz flagged them most/heavily broken).

| # | Step (code) | JSX component | Backend wiring |
|---|---|---|---|
| 1 | Welcome (WLCM) | `WelcomeStep` | none |
| 2 | Setup Mode (MODE) | `ModeStep` | none (Quick / Manual) |
| 3 | Provider (PROV) | `ProviderStep` | writes `model` to config; 12-provider list (anthropic, openai, minimax[featured], google, ollama[detected], openrouter, groq, mistral, together, fireworks, azure, custom) |
| 4 | Auth (AUTH) | `AuthStep` | credential capture + provider test |
| 5 | Model (MODL) | `ModelStep` | `GET /v1/models` (LIVE list — not hardcoded) |
| 6 | Backup LLMs (FLBK) | `FallbackStep` | **same provider list as step 3** (gaps-doc fix) |
| 7 | Channels (CHAN) | `ChannelsStep` | 8-channel two-col grid — Cloud APIs: telegram/discord/slack/email · Phone-paired: imessage/whatsapp/signal/matrix |
| 8 | Channel Config (CCFG) | `ChanConfigStep` | per-channel creds + test (conditional on toggles) |
| 8b | Signal/WhatsApp Pair | `SignalPairStep` | QR pairing (shown only when toggled) |
| 9 | Gateway (GTWY) | `GatewayStep` | host/port + service mode (launchd/rc.d/systemd) |
| 10 | Agent (AGNT) | `AgentStep` | 6 personas (coordinator/engineer/creative/sysadmin/analyst/custom) → `[agent]` persona+name |
| 11 | Workspace (WKSP) | `WorkspaceStep` | workspace + sessions paths |
| 12 | Security (SECR) | `SecurityStep` | `[aegis] level` |
| 13 | Features (FEAT) | `FeaturesStep` | subsystem toggles (platform-aware) |
| 14 | Voice (VOIC) | `VoiceStep` | `GET /v1/tts/providers` + `/v1/tts/voices` |
| 15 | Images (IMGS) | `ImagesStep` | image provider config |
| 16 | Orchestration (ORCH) | `OrchestrationStep` | prometheus/heartbeat config |
| 17 | Memory (MNEM) | `MemoryStep` | mnemosyne backend config |
| 18 | Skills (SKIL) | `SkillsStep` | `GET /v1/skills` |
| 19 | Complete (DONE) | `CompleteStep` | write `~/.zeus/config.toml` + launch choice |

UX note (merakizzz): Persona step must scroll / show all personas while keeping the aesthetic.

---

## 4. Phase 2 — Production TUI (→ `zeus-tui-production.jsx`)

PRIMARY_TABS + ADVANCED_TABS. Chrome: `TopBar` (ctx%, host, model, gw version, conn) + `TabBar` + `HintBar` + `:` `CommandPalette`.

| Tab | JSX component | Backend wiring |
|---|---|---|
| Chat | `ChatTab` | `POST /v1/chat` + `WS /v1/ws` (streaming, cooking iters, tool-call cards) |
| Office | `OfficeTab` | fleet agents (animated scene) |
| Pantheon | `PantheonTab` | `/v1/pantheon/missions` + `/rooms` |
| Tools | `ToolsTab` | `GET /v1/tools` |
| Memory | `MemoryTab` | workspace files · `/v1/sessions` · mnemosyne search (3 sub-tabs) |
| Channels | `ChannelsTab` | `/v1/channels` |
| Approvals | `ApprovalsTab` | `/v1/approvals` (+ pending badge) |
| Settings | `SettingsTab` | `GET/PUT /v1/config` |
| Advanced | `AdvancedTab` + `AdvancedSubview` | per-subview: Skills/Schedules/MCP/Extensions/Voice/ImageGen/Workflows/Economy → the `/v1/*` endpoints already mapped for the mobile parity work |

---

## 5. Process / gate (every screen)

1. Seat builds the screen to the JSX on `tui-rebuild`.
2. Push → merakizzz runs it live → screenshots.
3. Zeus100 gates: live render matches prototype (chrome, glyphs, color tokens, layout, copy) **and** the backend wiring works.
4. Only then: next screen. No skipping the live gate.

---

## 6. Team

- **spark + 107 build. Zeus100 architects + gates each screen. NOT 106.**
- titan → minimax-m3 (re-onboarding).
- Multiple seats on one branch: feature-commits per screen, Zeus100 serializes/gates; no direct overwrites.

---

## 7. Risks (today's lessons, baked in)

- **Transparency/bleed** → opaque-bg-first render contract (§0.2). A terminal with transparency will still show the wallpaper — the contract makes the TUI itself fully opaque so it doesn't matter.
- **"Verified" that isn't** → live-screenshot gate only (§0.3).
- **Stale/shadowed binary** → verify `zeus --version` SHA + `which zeus` before debugging (§0.4).
- **Stale local checkout** → `git show <branch>:` for substrate (§0.5).
- **mimo seat confabulation** → hard-gate every claim on substrate; seats verify their own state with a tool before asserting.
