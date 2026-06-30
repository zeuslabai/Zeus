# PRD — TUI Onboarding (Comprehensive, All Sections)

**Date:** 2026-05-03
**Author:** Zeus100
**Status:** Draft for merakizzz design pass
**Trigger:** merakizzz directive 2026-05-03 — *"give me a PRD for absolutely every single section of our onboarding... we'll do some design improvements"*
**Scope:** TUI onboarding wizard (`crates/zeus-tui/src/onboarding/mod.rs`). WebUI onboarding mirrors most patterns; touched only where TUI/Web diverge.
**Source of truth for current state:** `crates/zeus-tui/src/onboarding/mod.rs:14-39` (`OnboardingStep` enum, S76 8-step state machine grown to 22 steps).

---

## Table of contents

1. [Goals + UX principles](#goals--ux-principles)
2. [Step inventory (22 steps)](#step-inventory-22-steps)
3. [Per-step PRDs](#per-step-prds)
4. [Cross-cutting design decisions](#cross-cutting-design-decisions)
5. [Discovered constraints](#discovered-constraints)
6. [Design improvement opportunities](#design-improvement-opportunities)
7. [Open questions for merakizzz](#open-questions-for-merakizzz)

---

## Goals + UX principles

1. **Every step has a card-chooser pattern** where the user picks among 3-7 named options + an optional "Custom" + an explicit "Skip." Mirror the existing LLM-provider + channel-provider patterns (which already work well).
2. **Inline expansion on selection** — when the user picks a card, the credential/config form for that card expands inline below the grid. No page transitions for sub-flows. (merakizzz directive, 2026-05-03 voice: *"if I choose option A, B, or C, the respective API key entry should be shown right away."*)
3. **Skip is always allowed** — never force a choice. Onboarding should land users at a working baseline regardless of which steps they skip. (merakizzz directive, 2026-05-03: *"Don't force any choices."*)
4. **Re-run safe** — running `zeus onboard` over an existing config preserves credentials and pre-selects the currently-configured choices. No silent overwrites.
5. **Visual consistency** — Orbitron font, fire-orange accent, dark theme, consistent keybinds per step (`↑/↓ navigate, Enter select, Esc back, Tab next-field`).
6. **Real-time validation** — API keys: format pattern check; URLs: parseable; ports: numeric in range. Catch errors before the user advances.
7. **Optional connection test** — each credential step exposes an opt-in "Test connection" button that hits the provider's lightweight endpoint and surfaces the result inline.

## Step inventory (22 steps)

| # | Step | Const | Code symbol | Required? | UX pattern |
|---|------|-------|-------------|-----------|------------|
| 0 | Welcome | WLCM | `Welcome` | ✓ first | Splash + Enter to continue |
| 1 | Setup Mode | MODE | `SetupMode` | ✓ | 2-card chooser: QuickStart vs Full |
| 2 | QuickStart | QCFG | `QuickStart` | conditional | Single-form quick path |
| 3 | LLM Provider | PROV | `Provider` | ✓ | Card chooser (11 providers) |
| 4 | Auth | AUTH | `Auth` | ✓ | Mode tabs (Key / Token / Browser) |
| 5 | Model | MODL | `Model` | ✓ | List picker |
| 6 | Backup LLMs | FLBK | `Fallback` | ⏭ | Multi-select cards |
| 7 | Channels | CHAN | `Channels` | ⏭ | Multi-select cards (8 channels) |
| 8 | Channel Config | CCFG | `ChanConfig` | conditional | Per-channel credential forms |
| 8b | Signal Pair | SQRP | `SignalPair` | conditional | QR pairing screen |
| 8c | WhatsApp Pair | WQRP | `WhatsAppPair` | conditional | QR pairing screen |
| 9 | Gateway | GTWY | `Gateway` | ✓ | Form (port, host, daemon mode) |
| 10 | Agent / Persona | AGNT | `Agent` | ✓ | Form (name, role, SOUL.md tone) |
| 11 | Workspace | WKSP | `Workspace` | ✓ | Form (paths) |
| 12 | Security | SECR | `Security` | ✓ | 3-card chooser: Strict / Standard / Permissive |
| 13 | Features | FEAT | `Features` | ⏭ | Toggle grid (Nous / Hermes / Athena / Mnemosyne / etc.) |
| 14 | Voice | VOIC | `Voice` | ⏭ | Card chooser (5 cards — see voice PRD) |
| 15 | Images | IMGS | `Images` | ⏭ | Card chooser (6 cards — see image-gen PRD) |
| 16 | Orchestration | ORCH | `Orchestration` | ⏭ | Form (heartbeat, cron, watchdog) |
| 17 | Memory | MNEM | `Memory` | ⏭ | Form (Mnemosyne, FTS, embeddings) |
| 18 | Skills | SKIL | `Skills` | ⏭ | Multi-select grid |
| 19 | Complete | DONE | `Complete` | ✓ last | Summary + launch button |

Legend: ✓ required, ⏭ skippable.

---

## Per-step PRDs

### 0. Welcome (`OnboardingStep::Welcome`)

**Purpose:** orient the operator. Set tone (Orbitron, fire-orange, low-key sci-fi vibe).

**Current state:** splash screen with ASCII art + "Awaken Zeus" tagline.

**Fields:** none. Just `Enter` to continue, `N` to exit.

**Persistence:** none.

**Design improvements:**
- (a) Detect existing config and show "Welcome back, <agent name>" if re-running.
- (b) Add a one-line build version + commit SHA so operators know what they're onboarding into.

---

### 1. Setup Mode (`OnboardingStep::SetupMode`)

**Purpose:** route between **QuickStart** (sane defaults, 1 LLM provider, 1 channel, skip everything else) and **Full** (every section).

**Current state:** 2-card chooser. `setup_mode: 1=Full, 2=QuickStart`. QuickStart routes to `Complete` after a single form; Full routes to `Provider` and walks every step.

**Fields:** none beyond the choice itself.

**Persistence:** runtime-only (selects which steps to walk).

**Design improvements:**
- (a) Add a third card: **Custom** — opt into a subset of the 22 steps via checkboxes. Power users want this; current Full mode forces walking every step.
- (b) Show the QuickStart vs Full step count next to each card ("QuickStart — 1 step left" vs "Full — 17 more steps").

---

### 2. QuickStart (`OnboardingStep::QuickStart`)

**Purpose:** zero-to-running in one screen. Pick one provider, paste one API key, name the agent, done.

**Current state:** single form with `quickstart_fields` array. Fields: provider, api_key, agent_name.

**Persistence target in `config.toml`:**
```toml
model = "<provider>/<default-model>"
[credentials]
<provider>_api_key = "..."
[agent]
name = "..."
```

**Design improvements:**
- (a) Add a "Test connection" button before allowing Complete — catches typos in the API key before the user is dropped into a broken Zeus.
- (b) After completion, show a 2-line summary of what got configured + what's at default + a hint that `zeus onboard` re-run unlocks more.

---

### 3. LLM Provider (`OnboardingStep::Provider`)

**Purpose:** pick the primary LLM provider.

**Current state:** card chooser, 11 providers (Anthropic, OpenAI, Google, Ollama, OpenRouter, Groq, Mistral, Together, Fireworks, Azure, Bedrock).

**Persistence target:**
```toml
model = "<provider>/<model-id>"
```

**Design improvements:**
- (a) Show real-time availability hint per card (e.g., dim "Ollama" if no local Ollama detected at `localhost:11434`).
- (b) Each card should display the provider's flagship model + per-token pricing (when known) so the user can make an informed pick.
- (c) Add "Custom OpenAI-compatible" card with base URL field — covers vLLM, LM Studio, internal proxies, etc. Mirrors what we did for image-gen + voice cards.

---

### 4. Auth (`OnboardingStep::Auth`)

**Purpose:** capture credentials for the chosen provider.

**Current state:** mode-tab UI with three paths: API Key, Setup Token (paste), Browser OAuth.

**Persistence target:**
```toml
[credentials]
<provider>_api_key = "..."     # or
<provider>_oauth_token = "..."
use_oauth = true               # if OAuth path
```

**Design improvements:**
- (a) Surface the **paste-token first-run detection** prominently — currently buried. If a setup token is detected on the clipboard or in `~/.zeus/setup-token`, pre-populate.
- (b) Add **inline validation** of the key format (e.g., `sk-...` for OpenAI, `sk-ant-...` for Anthropic, `gsk_...` for Groq). Catch obvious typos before the connection test.
- (c) The Browser OAuth path needs a clear state indicator: "Opening browser... waiting for callback... received... validating..." so the user isn't confused if the browser doesn't auto-open.

---

### 5. Model (`OnboardingStep::Model`)

**Purpose:** pick a specific model from the chosen provider's catalog.

**Current state:** list picker. Per S55, models are curated per-provider (not all 200+ — just the ones we actively support).

**Persistence target:**
```toml
model = "<provider>/<model-id>"
```

**Design improvements:**
- (a) For providers that expose a dynamic model list (OpenRouter, Ollama, Together): fetch the live list from the provider's API at this step rather than relying on a hardcoded curated set.
- (b) Show context window + pricing per model in the picker (e.g., `claude-opus-4-7 (1M ctx, $15/$75 per Mtok)`).
- (c) Mark "Recommended" on the flagship for each provider so first-time users have a default to pick.

---

### 6. Fallback / Backup LLMs (`OnboardingStep::Fallback`)

**Purpose:** select 0-N backup providers for failover.

**Current state:** multi-select. When primary fails, the agent loop tries fallbacks in order.

**Persistence target:**
```toml
[fallback_models]
chain = ["openai/gpt-4o", "groq/llama-3.3-70b-versatile", ...]
```

**Design improvements:**
- (a) Default-suggest a sensible chain based on the primary (e.g., if primary is Anthropic, suggest OpenAI + Groq as cheap-fast fallbacks).
- (b) Reorder controls (drag-up/down or `[`/`]` keys) — chain order matters.
- (c) Surface "this fallback also needs an API key — go back to Auth?" if the user picks a provider whose creds aren't already on file.

---

### 7. Channels (`OnboardingStep::Channels`)

**Purpose:** multi-select messaging channels (Telegram, Discord, Slack, Email, iMessage, WhatsApp, Signal, Matrix).

**Current state:** card multi-select, `channel_toggled: HashSet<usize>`.

**Persistence target:**
```toml
[[channel_bindings]]
type = "discord"
# (per-channel sub-tables filled in ChanConfig)
```

**Design improvements:**
- (a) Each card shows: enabled/disabled toggle + a one-line "what this channel does" + a hint about pairing requirements (e.g., "WhatsApp requires QR scan from your phone").
- (b) Group cards: "Cloud APIs" (Discord/Slack/Telegram/Email) and "Phone-paired" (WhatsApp/Signal/iMessage/Matrix) — different setup complexity.

---

### 8. Channel Config (`OnboardingStep::ChanConfig`)

**Purpose:** capture per-channel credentials for each toggled channel.

**Current state:** form with `chan_config_fields`, walks each toggled channel sequentially.

**Persistence target (per channel):**
```toml
[channels.discord]
token = "..."
[[bindings]]
channel_id = "..."

[channels.telegram]
api_id = 12345
api_hash = "..."
phone = "..."
# ... etc per channel
```

**Design improvements:**
- (a) Per-channel form should expand **inline below the channel card grid** instead of replacing the grid (current TUI flow walks one channel at a time, hiding others). Keep all picked channels visible with their forms stacked.
- (b) Add **per-channel test buttons** (e.g., "Send test message" for Discord — picks a channel, sends "Zeus connected ✅", confirms reception).
- (c) Pull credentials from environment if present (e.g., `DISCORD_BOT_TOKEN` already in shell env → pre-populate).

---

### 8b. Signal Pair (`OnboardingStep::SignalPair`)

**Purpose:** QR-based device linking for Signal (CLI-based, not Cloud API).

**Current state:** fetches QR from `signal-cli` subprocess, displays as ASCII or terminal pixel art. User scans with phone.

**Design improvements:**
- (a) Add a "Why is the QR unreadable?" link to docs explaining iTerm vs Apple Terminal pixel rendering quirks.
- (b) After successful pair, show "Connected to Signal as <phone-number>" confirmation with the actual phone number from `signal-cli`.

### 8c. WhatsApp Pair (`OnboardingStep::WhatsAppPair`)

**Purpose:** QR-based device linking for WhatsApp Cloud API or Web mode.

**Current state:** parallel to SignalPair, different backend.

**Design improvements:** same as SignalPair (a)+(b).

---

### 9. Gateway (`OnboardingStep::Gateway`)

**Purpose:** configure the gateway daemon (port, host, optional WebUI co-host).

**Current state:** form with `gateway_fields`. Fields: port (default 8080), host (default 127.0.0.1), webui_enabled, daemon_install (yes/no).

**Persistence target:**
```toml
[gateway]
port = 8080
host = "127.0.0.1"
enable_agent_processing = true
```

**Design improvements:**
- (a) Detect if port 8080 is already in use (likely zeus already running). Surface clearly: "Port 8080 in use by PID <X> (zeus). Pick a different port, or stop the existing instance."
- (b) Add an **"Install as service"** card chooser at the bottom: launchd / systemd / rc.d / "Skip — I'll start manually." Currently just a yes/no toggle, but the OS-specific service-name + log path differs and operators don't know what's being installed.
- (c) WebUI co-host toggle should explain the bootstrap-mode trick (port 8081 if 8080 is taken).

---

### 10. Agent / Persona (`OnboardingStep::Agent`)

**Purpose:** name the agent + capture personality / role hints.

**Current state:** form. Fields: agent_name, role, soul_tone (used to seed SOUL.md).

**Persistence target:**
```toml
[agent]
name = "Zeus100"
role = "Coordinator"
[identity]
soul_tone = "professional, direct"
```
+ written into `~/.zeus/workspace/SOUL.md`, `AGENTS.md`, `IDENTITY.md` via `deploy-identity.sh` patterns.

**Design improvements:**
- (a) Card chooser for **persona archetype**: Coordinator / Engineer / Creative / Sysadmin / Analyst / Custom. Each card pre-fills a tone + sample SOUL.md snippet. Custom = free-form.
- (b) Live preview pane showing what SOUL.md will look like, updating as the user edits.
- (c) Auto-suggest agent name if it's a fleet host (e.g., on `.107` suggest `zeus107`).

---

### 11. Workspace (`OnboardingStep::Workspace`)

**Purpose:** confirm or override the workspace and sessions paths.

**Current state:** form with editable paths. Defaults: `~/.zeus/workspace`, `~/.zeus/sessions`.

**Persistence target:**
```toml
workspace = "~/.zeus/workspace"
sessions = "~/.zeus/sessions"
```

**Design improvements:**
- (a) Detect existing workspace at default path; offer "Use existing" vs "Start fresh (will rename old to `<path>.bak`)" if files are present.
- (b) Show estimated disk usage projection (workspace memory grows; sessions JSONL grows; `mnemosyne.db` can be GBs).

---

### 12. Security (`OnboardingStep::Security`)

**Purpose:** pick the Aegis sandbox level.

**Current state:** 3-card chooser: Strict / Standard / Permissive.

**Persistence target:**
```toml
[aegis]
level = "standard"  # or "strict" / "permissive"
```

**Design improvements:**
- (a) Each card shows **what's blocked** at that level (e.g., Strict = no `shell`, no `web_fetch`, no `apply_patch`).
- (b) Add a 4th card: **Custom** — opens a per-tool allowlist editor.
- (c) Hint about what's appropriate for the use case: "Strict for shared-machine fleet bots; Standard for personal coding assistant; Permissive for sandbox/research."

---

### 13. Features (`OnboardingStep::Features`)

**Purpose:** toggle subsystems (Nous, Hermes, Athena, Mnemosyne, Talos, Browser, Voice, etc.).

**Current state:** toggle grid. Each toggle ON enables the corresponding crate's runtime path.

**Persistence target:**
```toml
[nous]
enable_learning = true
[mnemosyne]
db_path = "~/.zeus/mnemosyne.db"
[talos]    # ← THIS IS THE GATE that's been biting us today
# (presence-only — empty block enables the crate)
[hermes]
default_channel = "console"
[athena]
vault_path = "~/Obsidian/Zeus"
```

**Design improvements (THIS STEP IS WHERE TODAY'S BUGS LIVED):**
- (a) **Talos toggle is mandatory-on for macOS** — without `[talos]` block, image-gen + system-info + AppleScript tools all silently absent. Either remove the toggle (always on) OR add a strong warning.
- (b) **Per-feature explanation card** — operators don't know what "Nous" or "Athena" does. One-line plain-English: "Nous = cognitive learning. Captures intent + improves over time. Optional but recommended."
- (c) **Detect platform** and gray out incompatible features (Talos on Linux/FreeBSD = no AppleScript path → most features no-op).

---

### 14. Voice (`OnboardingStep::Voice`)

**Purpose:** pick TTS backend.

**Current state:** form (currently ElevenLabs-only).

**Per the voice PRD** (`docs/sprints/prd-tui-voice-tts-cards-2026-05-03.md`): convert to a 5-card chooser (ElevenLabs, OpenAI TTS, Cartesia, Custom, Skip) with inline credential expansion.

**Design improvements:** see voice PRD.

---

### 15. Images (`OnboardingStep::Images`)

**Purpose:** pick image-gen backend.

**Current state:** form (defaults to GPT Image, no API key entry).

**Per the image-gen PRD** (`docs/sprints/prd-tui-image-generator-cards-2026-05-02.md`): convert to a 6-card chooser (OpenAI GPT Image, Google NanoBanana, BFL Flux, OpenAI API custom URL, Automatic1111 API custom URL, Skip).

**Design improvements:** see image-gen PRD. Plus: this step writes `[talos.image]` block (replacing orphan `[images]`) per the constraint.

---

### 16. Orchestration (`OnboardingStep::Orchestration`)

**Purpose:** configure the orchestration layer (heartbeat, cron, watchdog).

**Current state:** form with `orch_fields`. Fields: orchestration_mode (enable_all / disable / custom), heartbeat_interval, cron_enabled.

**Persistence target:**
```toml
[prometheus]
heartbeat_interval_secs = 300
[prometheus.heartbeat]
event_driven_only = false
safety_net_interval_secs = 3600
quiet_hours_start = 23
quiet_hours_end = 8
```

**Design improvements:**
- (a) **3-card chooser** for orchestration_mode: All-on / Heartbeat-only / Disabled.
- (b) Surface the heartbeat structured-task editor — let users define their own per-task intervals (push-work 30m, report 1h, etc.) instead of accepting defaults.
- (c) Show the watchdog interval (10-min default) with explanation of what triggers Discord alerts.

---

### 17. Memory (`OnboardingStep::Memory`)

**Purpose:** configure Mnemosyne (memory backend).

**Current state:** form. Fields: db_path, enable_fts, embeddings_provider.

**Persistence target:**
```toml
[mnemosyne]
db_path = "~/.zeus/mnemosyne.db"
enable_fts = true
embeddings_provider = "ollama"  # or "openai"
embeddings_model = "nomic-embed-text"
```

**Design improvements:**
- (a) Card chooser for **embedding provider**: Ollama (local, free) / OpenAI (cloud, paid) / Skip embeddings (FTS-only).
- (b) Disk-usage estimate based on expected message volume.
- (c) Auto-detect Ollama at `localhost:11434` and pre-select if available.

---

### 18. Skills (`OnboardingStep::Skills`)

**Purpose:** install initial skills (SKILL.md plugins).

**Current state:** multi-select grid sourced from `zeus-skills` registry.

**Persistence target:** skills installed to `~/.zeus/skills/<skill-id>/`.

**Design improvements:**
- (a) Categorize skills (Productivity / Dev / Marketing / Security / etc.) instead of one flat grid.
- (b) Show "Recommended" badge on a starter set (5-10 most-useful skills) so first-time users have an obvious default.
- (c) Skill cards should show what tools they grant (e.g., "OpenClaw Compat" → adds `claw_*` tools).
- (d) Search/filter input — skill catalogs grow, flat grid won't scale past ~20.

---

### 19. Complete (`OnboardingStep::Complete`)

**Purpose:** summary + launch.

**Current state:** summary screen showing what was configured. Enter to launch the gateway.

**Design improvements:**
- (a) Show a **green/red status indicator** per section (green = configured, gray = skipped, red = error to fix). Operators know exactly what's ready.
- (b) Add a "Test all configured backends" button — one-shot connection test across LLM, channels, image-gen, TTS. Surface any failures.
- (c) Save the summary to `~/.zeus/onboarding-summary.md` so re-runs can diff against it ("you previously had X configured; this run will change Y").

---

## Cross-cutting design decisions

### Card chooser pattern (mandatory across steps)

When a step has a discrete-choice with 2-7 options, render as a **horizontal card grid** (not a vertical list, not a dropdown).

```
┌────────────┐  ┌────────────┐  ┌────────────┐
│   ICON     │  │   ICON     │  │   ICON     │
│            │  │            │  │            │
│  Title     │  │  Title     │  │  Title     │
│  one-line  │  │  one-line  │  │  one-line  │
│  desc      │  │  desc      │  │  desc      │
│            │  │            │  │            │
│  [SELECT]  │  │  [SELECT]  │  │  [SELECT]  │
└────────────┘  └────────────┘  └────────────┘
```

Selection state: subtle fire-orange border + slight inner glow on the selected card. Inline form expands BELOW the grid.

### Re-run behavior (mandatory across steps)

When `zeus onboard` runs over an existing `~/.zeus/config.toml`:
1. **Pre-populate every form field** from the existing config.
2. **Pre-select the currently-configured card** in every chooser.
3. **Show a per-step "no change" indicator** if the user advances without editing — minimizes accidental overwrites.
4. **Diff summary at Complete step** — show what's about to change vs. what stays the same.

### Validation (mandatory across credential steps)

- Format check: regex per provider (sk-..., sk-ant-..., gsk_..., etc.).
- "Test connection" button: opt-in, hits a lightweight provider endpoint (`/v1/models` for OpenAI-likes, `/health` for self-hosted), surfaces ✅/❌ inline.
- Real-time feedback as the user types — don't wait for submit to surface "key looks invalid."

### Credential storage

All credentials land in `~/.zeus/config.toml` `[credentials]` block. Path: 0600 perms. Backups via `config-guard.sh`. **Never write secrets to `~/.zeus/.env`** — that pattern was deprecated 2026-03-14 (see `feedback_no_deploy.md`).

### Skip semantics

- "Skip" is always available except on `Welcome`, `SetupMode`, `Provider`, `Auth`, `Model`, `Gateway`, `Agent`, `Workspace`, `Security`, `Complete` (10 required steps).
- Skipping writes `<step>_configured = false` to a runtime state file (not config.toml — that's only for actual configured values). This lets `zeus onboard --resume` jump back to skipped steps.

---

## Discovered constraints (from today's work)

### 1. `[talos]` config gate (CRITICAL — biggest discovery 2026-05-03)

The Features step (#13) MUST write a `[talos]` block (even if empty) for macOS hosts. Without it, **all 193 Talos tools silently fail to register** in the gateway/CLI/MCP runtimes (gate at `src/main.rs:925`, `gateway.rs:417/441/643`, `mcp/server.rs:118/441`). zeus106 verified this empirically yesterday on .106.

The current onboarding either skips writing `[talos]` on Mac or has a bug — fleet shakedown showed every reporting host (.100, .106, .107, .226-effective) was missing the block.

**Fix path:** Features step always writes `[talos]` block on macOS, never as a user-facing toggle on that platform. On Linux/FreeBSD, the block is opt-in (limited utility without AppleScript).

### 2. Orphan `[images]` block

Onboarding currently writes `[images]` (`provider = "gpt-image-1.5"`, OpenAI URL) but no tool reads it. The Images step (#15) should write `[talos.image]` instead (per image-gen PRD), and the existing `[images]` writer should be removed from the wizard.

### 3. Heartbeat config didn't reach config.toml on past deploys

The `[prometheus.heartbeat]` block was missing on every reachable fleet host today, even after redeploy. The Orchestration step (#16) needs to ensure this block is written + the deploy-identity script needs to NOT strip it.

### 4. Native Claude vs MCP path inconsistency

The MCP gate uses `[mcp_server] enable_talos = true` while the CLI/gateway gate uses `config.talos.is_some()`. Two separate switches for the same effective control. The Features step should expose ONE toggle that flips both, hiding the underlying duplication.

---

## Design improvement opportunities (prioritized for this sprint)

| # | Improvement | Impact | Effort |
|---|------------|--------|--------|
| 1 | Apply card chooser pattern to Memory (#17), Voice (#14), Images (#15), Security (#12) — currently inconsistent | High UX consistency | Medium |
| 2 | Inline credential form expansion (no page transition) on every chooser | Matches merakizzz directive | Medium |
| 3 | Always-write `[talos]` block on Mac (Features #13) — fix the gate | Unblocks 190+ tools fleet-wide | Low |
| 4 | "Test connection" button on every credential step | Catches bad keys before launch | Medium |
| 5 | Re-run pre-population from existing config | Operator-friendly, avoids overwrites | Medium |
| 6 | Persona archetype card chooser (Agent #10) | Faster onboarding, better SOUL.md seeds | Low |
| 7 | Live disk-usage estimate (Workspace #11, Memory #17) | Prevents disk-full surprises | Low |
| 8 | Provider-aware validation regex (Auth #4) | Catches typos | Low |
| 9 | Categorize Skills (#18) + Recommended badge | Better first-run experience | Medium |
| 10 | Complete step (#19) full backend test | Confidence the install works | Medium |

---

## Open questions for merakizzz

1. **Step ordering:** current order walks `Provider → Auth → Model → Fallback → Channels → ChanConfig → ...`. Should Workspace/Security/Features come BEFORE LLM provider so the operator sets up the host environment first? Or current order (start with the LLM choice) is the right hook?
2. **QuickStart scope:** is the current "1 provider + 1 channel" QuickStart the right minimal? Or should it be even smaller (LLM only, channels optional)?
3. **Custom mode** (Setup Mode #1 sub-card): worth building, or stick with QuickStart vs Full?
4. **Persona archetype cards (#10):** which archetypes do we ship? My proposal: Coordinator / Engineer / Creative / Sysadmin / Analyst / Custom. Add/drop?
5. **Skill recommendations (#18):** which 5-10 skills should be the "Recommended" starter set? Need a curated list — currently each agent picks differently.
6. **Service install (#9):** should daemon installation be a card chooser (launchd/systemd/rc.d/Skip) or auto-detect platform + just confirm?
7. **Test connection** scope: opt-in (one button per step) or default-on (every credential step auto-tests on advance)? Default-on is safer but slows the flow.

---

## Status / next steps

- [x] PRD drafted (this doc)
- [ ] merakizzz design pass + open-questions resolution
- [ ] Layout/graphics for each step (your lane)
- [ ] Per-step implementation dispatch (Track A: card chooser pattern conversion / Track B: validation + re-run / Track C: discovered-constraints fixes)
- [ ] Estimate: full implementation = 5-8 sessions split across 2-3 fullstack agents. Pre-launch: discovered-constraints fixes only (#3 above) — that's ~1 session.
