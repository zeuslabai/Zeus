# Changelog

All notable changes to Zeus are documented here.

---

## [S92] - 2026-03-29

Sprint 92 — Agent voice investigation and synthesis. Two independent analyses of why Zeus agents sound robotic were combined into a single actionable architecture document.

### Docs / Analysis

- Synthesize zeus106 + zeus107 agent voice analyses into `docs/S92-agent-voice-synthesis.md` (`c6a7c04d`)

**Root causes identified:**
- Completion interrogation loop (5 rounds) trains agents to overclaim certainty rather than communicate honestly
- Heartbeats fire unconditionally — no content gate — producing ghost-task reports from stale context
- `HEARTBEAT_OK` token emitted to Discord as a real message, creating channel noise at fleet scale
- Self-echo in history injection mirrors robotic phrases into feedback loops across consecutive heartbeats
- JSON cognitive context scaffolding (`[Cognitive Context] Intent: Search { query: ... }`) causes machine-format input to produce machine-format output

**Seven fixes documented in priority order:** HEARTBEAT_OK suppression (one filter), content-gated heartbeat firing (one function), "don't invent tasks" prompt guard (one line), completion interrogation softening, self-echo cap in history injection, plain-English cognitive context, and SOUL.md template removal.

**Source analyses:** `zeus106-analysis.md` (completion loop and echo chamber), `zeus107-analysis.md` (code-level root causes in `gateway.rs` and `zeus-prometheus`)

---

## [S91] - 2026-03-29

Sprint 91 — Office TUI audit sweep, API error surfacing, and onboarding hardening. Polish and correctness sprint closing out the Star Office cycle.

### Fixed

**Office TUI**
- Office TUI audit findings — all 9 items addressed (`d98f3c11`)
- Add audit review comments to `office/mod.rs` and `office/palette.rs` (`2563e889`, `c87567dd`)

**API Error Surfacing**
- Surface API errors to channel instead of silent failure — agents now see error context in conversation (`8a71b28c`)

**Onboarding**
- Onboarding never writes `unknown` as model — preserves existing or blocks advance (`6afc2fb6`)
- Onboarding writes real default model instead of `unknown` (`ea42a54a`)

**Agent Personality**
- Refine personality communication guidelines — natural over templated (`887eca76`)

### Added

- `zeus daemon restart` command — stop + 2s wait + start, one command (`3e4c248b`)
- `/clear` and `/compact` slash commands + session API endpoints (`ccbba8d0`)

---

## [S90] - 2026-03-28

Sprint 90 — The Office TUI + Pantheon TUI full wiring, reputation system, and cross-nav polish.

### Added

**The Office TUI (full wiring)**
- Office TUI + Pantheon TUI fully wired — agents visible, tasks live, navigation complete (`e652108e`)
- Star Office shows task labels and live reputation; Enter on focused agent opens Pantheon DM (`785f68db`)
- Reputation-based trust + `current_task` tooltips in Star Office (`5753d096`)
- 72 new tests — Pantheon TUI, API mocks, render smoke, terminal sizes, keyboard nav (`987cb747`)

### Fixed

- Cross-nav Enter key + clear voice onboarding defaults (`4e4f5f37`)

---

## [S87] - 2026-03-28

Sprint 87 — TUI onboarding bug fixes, Mattermost channel addition, and scroll fixes.

### Fixed

**TUI Onboarding**
- TUI onboarding bugs — scroll fix, IRC fields, gateway/voice/image defaults (`f0aee07a`)
- Fix bugs 2/3/4 from S87 audit (`e7a596e1`)
- Chan config scroll tracks actual line position, not field index (`ae1551f3`)

### Added

- Add Mattermost to onboarding channels (`6ec699c9`)

---

## [S86] - 2026-03-28

Sprint 86 — Star Office integration. A game-style multi-agent workspace visualization lands in Zeus. The fleet becomes visible.

### Added

**Star Office**
- Port Star Office from original repo — assets + adapted `game.js` (`993be6f9`)
- `GET /v1/office/state` endpoint — live agent state for the game (`81c8653f`, `90d353b7`)
- `POST /v1/office/join` and `/v1/office/leave` API endpoints (`d3ed0fc5`)
- `StarOfficeConfig` schema — `room_id`, `idle_timeout`, `auto_join` (`c7ee78b9`)
- Auto-idle agent state logic based on activity (`7606dcbd`)
- Gateway auto-joins Star Office Pantheon room on startup (`45e22a56`)
- Office bubbles in WebUI (`f6860c78`)

**Agent Pipeline**
- Wire X/Twitter adapter into `agent_loop.rs` (`fd1e4a9b`)
- Activation filter — bot messages only cook when agent is @mentioned (`e6259046`)

**Install**
- Linux dependency installation (`libssl-dev`, `cmake`, `pkg-config`, `libasound2-dev`) (`a90601ed`, `f159a308`)
- Daemon install auto-loads into launchd + onboarding uses daemon install (`1ec4cdee`)

**WebUI**
- Add X/Twitter + IRC channels for onboarding parity with TUI (`549c111e`)

### Fixed

- Resolve `StarOfficeConfig` duplicates + conflicting `office_state` handler (`3842e9af`)
- Fix double comma in `game.js` setState fetch call (`35b9de6f`)
- `office_state` returns real agent state from registry — no more hardcoded "working" (`649ae052`)
- Align email field names + add `fetch_models` for 5 providers (`3603121d`)

---

## [S83] - 2026-03-27

Sprint 83 — Mobile app parity, `spawn`/`collect_spawns` cooking loop fix, deep research wiring, and TUI input polish.

### Added

**Mobile**
- Audit + improve Zeus mobile apps (`e03c4d96`)
- Wire 90+ missing API endpoints on Android to reach iOS parity (`4a4266d9`)
- Wire `togglePredictiveSpawning` + add missing Android API endpoints (`f23ef450`)

**Agent**
- Wire `LlmClient` into `AgentToolExecutor` for `deep_research` support (`94b31bc6`)
- `execute_deep_research` as standalone public function (`acbeff36`)

**TUI**
- Cursor editing, scroll controls, streaming cursor blink (`acbeff36`)

### Fixed

- Fix `spawn` and `collect_spawns` in cooking loop — deduplicate imports and accessors (`7af2f7b3`, `a1ab3830`, `dd26ef3d`)
- Use `zeus_core::ToolResult` in deep_research standalone fn (`af23cbda`)
- Add "coming soon" label to Pantheon tab (`f1f208c3`)

---

## [S82] - 2026-03-27

Sprint 82 — WebUI audit, iMessage adapter wiring, WebUI key rotation UI, and install/uninstall hardening.

### Added

**WebUI**
- Key rotation confirm dialog (`1ee65d1f`)
- Max iterations field is now editable in WebUI (`1ee65d1f`)

**Channels**
- iMessage adapter wired — macOS-only, no config needed, uses defaults (`006204ea`, `1ee65d1f`)

### Fixed

**Install / Uninstall**
- Fix launchd service name mismatch + always load plist on install (`b321e3e9`)
- Purge `~/.zeus` FIRST before kill/service steps in uninstall (`89cd2338`)

**Onboarding**
- Y/N keys work on Welcome screen (`42a17587`)
- Persona render uses 2D selection to match navigation (`9e25927f`)
- Persona arrows work without Tab — removed `sel==1` guard (`e77dec45`)

### Docs

- WebUI audit — all pages wired, 5 gaps flagged for S83 (`8e9e0a06`)

---

## [S81] - 2026-03-27

Sprint 81 — Audit sprint. TUI wiring audit surfaces dead screens; two new operational skills added to the fleet.

### Added

**Skills**
- `zeus-sprint-state` skill — lets agents query current sprint progress (`63513c7d`)
- `zeus-voice-audit` and `zeus-sprint-state` skills added to install (`fe767696`)

### Docs / Analysis

- TUI wiring audit — 2 live tabs, 4 dead screens, 4 issues found and documented (`dcbab515`)

---

## [S80] - 2026-03-27

Sprint 80 — Heartbeat guardrails, compiler warnings sweep, and dead code removal. Stability-focused sprint hardening the fleet for unattended operation.

### Fixed

**Heartbeat & Loop Prevention**
- Tighten agent guardrails and expand result suppression — agents no longer generate noise in fleet channels (`0346d0f7`)
- Filter `HEARTBEAT_OK` messages in gateway to prevent agent ping-pong loop (`c38332f1`)
- Suppress silent heartbeat results from Discord + soften channel prompt (`59da138d`)
- Add message-count trigger to compaction — catches short-message echo loops before they spiral (`7fdc6237`)

**Code Quality**
- Remove dead `poll_mentions` code from X/Twitter adapter (`c2cd4de3`)
- Fix onboarding tests to set `api_key` before advancing past Auth step (`c2cd4de3`)
- Additional warnings cleanup — let chains, `strip_prefix`, `split_once`, `keys()` iterator (`9b21b7b3`, `24814cc5`)
- Resolve final 2 warnings — unreachable code in TUI, `private_interfaces` in telegram_bot (`1d9a6c75`)
- Suppress `dead_code` warnings in telegram_bot adapter (`224b0822`)
- Remove unused imports across zeus-api handlers (`362fb741`, `f4c35dc0`)

**Voice**
- Self-echo cap, plain English cognitive context, remove template phrases from voice output (`5d727433`)
- Remove duplicated mentions-polling dead code from X adapter (`4aa070d7`)

**TUI**
- `--classic` flag launches classic onboarding immediately (`e65645cb`, `65022f02`)
- Hydrate sidebar agents/channels from `config.toml` at `App::new()` (`7772d430`)

**Infrastructure**
- Add `channel_source` to test helpers + suppress unused var warning in Mnemosyne (`f0ae639b`, `bf2e8517`)
- Use let-chains, `is_ok_and`, and flatten nested ifs across codebase (`11948787`, `e74904bd`)

---

## [S79] - 2026-03-27

Sprint 79 — Channel field fixes, session compaction, and skill activation overhaul. The agent intelligence pipeline gets smarter about what to load and when.

### Added

**Session Compaction**
- Session compaction system — summarize history before cooking loop injection (`46c2dc93`)
- Long-running sessions no longer bloat context; history is compressed into summaries automatically

**Agent Intelligence**
- Update haiku model name to `claude-haiku-4-5-20251001` (`7f157c8d`)
- Skills now agent-driven (list names only) instead of auto-injected by keyword (`edf95de3`, `68e24701`)
- Remove verification/TDD/debugging skills from default install — cleaner baseline (`988b47d5`)

### Fixed

**TUI Onboarding**
- Correct Email and IRC field names in onboarding `save_config` (`8add67df`)
- Fix Azure env var name and add Bedrock credential fields (`240a11fe`)
- Persona selection now loads full personality description into SOUL.md (`426a0f6a`)
- Logo color gradient matches JSX prototype — add `ACCENT_BRIGHT`/`ACCENT_DIM` (`3fcbf894`)
- Logo bottom rows use dark rust accent instead of white (`01efbf17`)

**Infrastructure**
- Remove hardcoded stale stats (59,400 lines, 212 tools) from welcome screen (`9d724753`)
- Ollama detection reads `OLLAMA_HOST` env var instead of hardcoded `127.0.0.1:11434` (`a5bbd8b1`)
- Replace hardcoded `v0.1.0` with `env!(CARGO_PKG_VERSION)` in render.rs (`410dfdc5`)
- IRC channel added to TUI onboarding (server, port, channel, nickname) (`9e5485b6`)

---

## [S78] - 2026-03-26

Sprint 78 — Onboarding polish, OAuth hardening, X/Twitter adapter, and installer v2. The biggest UX sprint yet: 60+ commits turning the first-run experience from "developer setup" into "product launch."

### Added

**Cyberpunk Installer**
- `install-v2.sh` — full cyberpunk TUI installer UI, 1,148 lines (`8e9edd07`)
- Promoted to `install.sh` — old version renamed to `install-v1.sh` (`d0e2513b`)
- Create `/usr/local/bin` if missing on fresh Apple Silicon Macs (`a0d3b8b3`)
- Prompt for sudo upfront with explanation of why it's needed (`ecbe8911`)
- Logo replaced with full pixel art ZEUS in installer banner (`23a8e779`)

**TUI Onboarding (18-step wizard)**
- Pixel-perfect onboarding — ASCII logo, provider cards, security cards, launch options (`c057cc83`)
- Async model fetching from provider APIs (`51537995`)
- Dynamic personality + skill loading from filesystem (`3b24bb02`)
- 10 personality templates added in `personalities/` folder (`18476093`)
- Auth mode toggle — API Key vs OAuth Token (`2038de8b`, `4ef138ca`)
- Ollama detection via TCP + auth validation + OAuth option (`d5cafa40`)
- Pixel-perfect provider grid, top/bottom bars matching JSX spec (`dfefdf75`)
- Progress dots, centered welcome, dead code cleanup (`5222095b`)
- Save channel credentials + API key to config.toml (`420b68e6`)
- Discord Guild ID field + save to bindings (`6335a63c`)
- Scrolling for skills + channel config, truncate descriptions (`bd45baca`)
- Auto-launch gateway after onboarding completes (`2572c458`)
- Breadcrumb overflow fix — 5-step window centered on current (`701a01c2`)
- Card borders, navigation, validation, remove hardcoded URLs (`0c406559`)
- Block ChanConfig advance when required fields are empty (`b89cd269`)
- X/Twitter as channel option in onboarding (`2bd06d39`)
- Channel config field index alignment (`0e8326be`)
- No channels pre-selected by default (`84e5b8c2`)

**Agent Pipeline**
- TUI chat now uses full agent pipeline (tools + cooking loop) (`833f27d8`)
- Route OpenAI-compat non-stream completions through `agent.run()` when agent is active (`130948c4`)

**WebUI**
- Dynamic personality + skills loading from API (`7cfd5e8e`)
- Capture `session_id` from WS `response_complete` to preserve session continuity (`bb2e37b0`)

**Native Apps**
- Full view suite for Desktop + Mobile (`67a1b1d7`)

**X/Twitter Adapter**
- Proper OAuth 1.0a HMAC-SHA1 auth, cached user ID, `send_as`/`scheduled_at`/`reply_to` semantics (`b3e63efa`)
- `sha1` crate added for OAuth 1.0a signing (`e6e77df5`)

### Fixed

**OAuth & Auth**
- OAuth token saved to `config.toml` — removed premature write (`25f3debd`)
- Populate `CredentialStore` from `config.toml` `[oauth]` on startup (read-only) (`1fb74e25`)
- Onboarding `fetch_models` uses Bearer + OAuth beta header for setup tokens (`95f8eee8`)
- `config.toml` is SOLE source of truth — no `credentials.json` as primary (`2f7143ef`)
- OAuth token → `config.toml` `[oauth]` section + `credentials.json` for LLM compat (`07f55696`)
- Remove `credentials.json` generation from `main.rs` startup (`199e8a46`)
- Revert Bearer auth for onboarding model fetch — `x-api-key` works for all token types (`78a4367e`)

**Hardcoded Values Purge**
- Remove ALL hardcoded model names and product defaults (`6482350c`)
- Last remaining `claude-sonnet-4-6` fallback → `claude-sonnet-4-20250514` (`34423e35`)
- Use full Anthropic model IDs everywhere (`93876bd1`)
- Remove hardcoded Sentient Intelligence Protocol from system prompt (`c2234b58`)
- Replace robotic workspace templates with OpenClaw-style personality files (`aa7e0def`)
- Logo replaced with full pixel art ZEUS logo in `render_welcome` (`9027909e`)

**Agent Behavior**
- Remove force-loaded core skills from cooking loop — use contextual activation (`7b44cecd`)
- Remove hardcoded verbosity injection from agent_loop (`ed054aae`)
- Revert history message tagging — tags leak into agent responses (`4cbef6d2`)
- Tag assistant messages in history so agent recognizes its own past responses (`f4d7c3d5`)

**Infrastructure**
- TUI session unification — share session with channels (OpenClaw pattern) (`02524af9`)
- Gateway launch redirects output to log files (no TUI corruption) (`318712a2`)
- Read `gateway_port` from `ZEUS_GATEWAY_PORT` env var in auto-spawn (`ad868e37`)
- Extract Discord fleet channel ID to `ZEUS_DISCORD_FLEET_CHANNEL` env var (`7160fd6b`)
- `matrix-sdk` recursion limit + installer shows build errors clearly (`7b81b481`)
- Uninstall: binary removal failure no longer blocks `--purge` (`c3abaf53`)
- Uninstall.sh exit code 0 + TUI chat badge `api` → `tui` (`b3effb31`)
- WebUI: `Trunk.toml` proxy backend port `3001` → `8080` (`c72875b3`)
- WebUI: read `qs_port` default from `ZEUS_GATEWAY_PORT` env (`04e5574b`)
- Chat handler uses `resume_or_create` for session (was 404 on missing) (`f67a0643`)

**Tests**
- Update onboarding tests to set `api_key` before Auth gate (`766c8230`)
- Add missing `channel_source` field in `agent_integration` test (`7299db32`)
- Add missing `channel_source` field to Message constructors in `intelligence.rs` (`4d96c032`)

### Refactored

- Purge old TUI, rename `zeus-tui-v2` → `zeus-tui` (`43e95d08`)
- Remove redundant gateway bot filter — `allow_bots` handles it (`7ffbaec3`)
- Remove ALL communication suppression — agents must always talk (`8dabfc7e`)
- Restore cooking context to 50 messages (pre-S57 baseline) (`a6da9f70`)
- Resolve all 12 compile errors from handler split (`5d9df82b`)

---

## [S77] - 2026-03-25

Sprint 77 — TUI onboarding rewrite from scratch. 18-step pixel-perfect wizard built from a JSX prototype, with 92 tests. The old TUI is dead — long live the new TUI.

### Added

**18-Step TUI Onboarding**
- Complete TUI onboarding rewrite from JSX spec — 18 steps covering Welcome, Provider, Auth, Model, Channels, Personality, Skills, Security, and Launch (`2bd2da91`)
- 92 tests for the onboarding module
- Wire all 18 steps with proper keyboard handling (`3a61bcae`)

### Refactored

- Purge old TUI crate, rename `zeus-tui-v2` → `zeus-tui` — single unified TUI (`43e95d08`)

### Infrastructure

- Commits: 7 (S77 core) + onboarding foundation for S78
- All work merged to main

---

## [S75] - 2026-03-25

Sprint 75 — WebUI wiring, TUI onboarding, handlers split, growth funnel audit, and TUI live relay. Full-stack sprint across 3 tracks.

### Added

**TUI Onboarding (Track C — Zeus112)**
- Wire 8-step TUI onboarding module into `lib.rs` — `zeus` with no config now launches the full onboarding flow (`35a7d885`)
- Replaces old `setup.rs` path; new flow covers LLM provider, API keys, channel setup, and agent identity

**Growth & Funnel Analysis (ZeusMarketing)**
- Growth funnel audit: 4 critical gaps identified killing activation and viral loop (`f77d79c2`)
  - Gap 1 (P0): Onboarding has no resume state — interrupted sessions restart from step 1
  - Gap 2 (P1): Empty dashboard has no CTA — first-time users bounce on blank state
  - Gap 3 (P2): Agora skill ratings never fetched — social proof missing, installs suppressed
  - Gap 4 (P3): Discover page has no share links — primary viral acquisition surface unused
- Activation funnel events instrumented (recommendation): 5 key events mapped to existing `/v1/analytics/event`

### Fixed

**TUI v2**
- Unicode-aware word wrap + correct inner-width calculation for border offset (`6757d017`)
- Long messages no longer disappear past border in chat view

**Channel / Session**
- `message` tool now supports IRC + all channel types (`3fd811bf`)
- Session API now includes `channel_source` in message payloads (`97b85dc0`)

**Infrastructure**
- Heartbeat session corruption resolved (`b5c0957b`)
- Heartbeat timeout increased 30s → 300s — long coding tasks no longer killed mid-flight (`9aac27f5`)
- `install.sh --update` flag added for in-place upgrades (`637be139`)

### Added (post-changelog commits)

**TUI Live Relay**
- TUI v2 now polls the shared session every 5s for new messages — IRC, Discord, Telegram messages appear in real-time without restarting (`e3de806e`)
- Replaces one-shot history load with continuous polling loop; new messages appear as they arrive

**WebUI UX**
- Dashboard empty-agents state upgraded: replaces "No active agents" with a fleet-ready message and a `Create Agent →` CTA button linking to `/agents` — first-time users no longer bounce on a blank state (`c67878fb`)
- Closes Gap 2 (P1) from the S75 growth funnel audit

### Fixed (post-changelog commits)

**TUI (legacy)**
- Removed `Paragraph::wrap` from chat renderer — double-wrapping pre-wrapped content caused overlap and shifted line rendering (`4caa0fd5`)
- Simplified line count to `lines.len()` — each entry already maps to one visual row; redundant `count_wrapped_rows()` call produced incorrect scroll bounds and caused scroll position drift

### In Progress

**WebUI Wiring (Track A — zeus107)**
- `dashboard.rs` and `sessions.rs` wiring to live gateway API underway
- 3 additional pages pending (Agora, Skills, Discover)

**Handlers Split (Track B — zeus106)**
- Extracting tools + credentials handlers from `mod.rs` (baseline: 16,542 lines, target: <13,000)
- Tools handlers timed out at 300s during extraction — now unblocked with heartbeat fix

---

## [S70] - 2026-03-23

Sprint 70 — Superpowers, API handler refactor, and fleet intelligence parity. 30 commits across platform hardening, agent workflow upgrades, and TUI overhaul.

### Added

**Superpowers Workflow**
- Integrated Superpowers framework — TDD, verification gates, debugging iron laws baked into every agent's AGENTS.md (`25fcd608`)
- Superpowered AGENTS.md template + all 8 workspace files (SOUL.md, IDENTITY.md, USER.md, TOOLS.md, HEARTBEAT.md, MEMORY.md, CAPABILITIES.md, AGENTS.md) now loaded into agent prompt (`fd1375ae`)
- Completed workspace file parity with OpenClaw — every agent has full context on boot (`105a731f`)

**Intelligence & Memory**
- Nous + skills wired into cooking loop — full intelligence parity across fleet (`f2081ddb`)
- Skill content loading + Nous failure learning (`f4a8a9dc`)
- Bidirectional MEMORY.md ↔ Mnemosyne sync (`bb96bb35`)
- Task assignment importance tagging in Mnemosyne (`4b8e97c5`)
- Message queue depth tracking for concurrent processing (`8e449be2`)

**TUI v2**
- Full TUI v2 rewrite — chat-first layout modelled after Claude Code (`d2454b4c`)

**Auto-Capabilities**
- Auto-generate CAPABILITIES.md on gateway boot (`94c612de`)
- Auto-detect embedding provider from available API keys (`8ff7a67c`)
- LLM health probe on startup (`c78b46d2`)

### Fixed

**Config & Install**
- `Config::save()` no longer wipes channel bindings; per-agent workspace paths added; config backup on save (`492c408f`)
- Skills directory wiring — install.sh + agent + cooking loop properly linked (`2785b6b4`)
- Remove duplicate agent name prompts from install.sh (`cb658115`)
- install.sh + onboarding audit — 11 issues resolved (`5c8e9edc`)
- Remove `"type": "stdio"` from MCP config in install.sh (`b4c0369a`)
- Onboarding optional fields + cooking loop memory parity (`478bb918`)
- Onboarding — Mattermost hint text + iMessage blank screen (`069e95ce`)

**Gateway**
- Replace `unwrap()` with `expect()` on pruning config in gateway (`9c7c42c3`)
- Embedding circuit breaker — 5-minute cooldown after provider exhaustion (`50bb6fbd`)
- Cooking timeout increased from 30s → 300s (5 minutes) (`7abdd616`)
- Restore Sentient Intelligence Protocol + fix version mismatch (`13c41b33`)

**TUI**
- TUI always-on input — no vim mode, default to Chat tab (`e71ad75e`)

### Refactored

**API Handler Modularisation (A3)**
- Extracted agent handlers to dedicated module (`d0fdb9fe`)
- Extracted channel handlers to dedicated module (`10926c19`)
- Extracted memory + config handlers to dedicated modules (`41108211`)
- Extracted skill handlers to dedicated module (`81a73364`)
- Extracted approval handlers to `security_handlers` module (`c645ff41`)
- Extracted session handlers to dedicated module (`c0ae4584`)
- Extracted analytics handlers to dedicated module (`5934aabb`)

### Behavioral

- AGENTS.md template: acknowledge all tasks immediately rule (`3bcdfcd2`)
- AGENTS.md template: report-as-you-go behavior default (`26b1dae3`)

---

## [1.0.0] - 2026-03-22

Zeus 1.0 is the production-ready release. After six sprints of hardening, the platform is stable enough to ship. This release consolidates S66–S70 changes: dead code purged, config made consistent, TUI UX fixed, agent communication polished, and the full fleet multi-agent system battle-tested.

### Added

**Core Platform**
- 31-crate Rust workspace (~342K LOC, edition 2024)
- Multi-provider LLM: Anthropic, OpenAI, Ollama, OpenRouter, Groq, Mistral, Together, Fireworks, Azure, Bedrock, Google
- OAuth browser-based login (Claude Pro/Max)
- Cost-aware auto-routing with budget fallback
- Conversation branching (fork/merge paths)
- Extended thinking (`low` / `medium` / `high` / `xhigh` budget levels)

**Agent System**
- 8 core tools: `read_file`, `write_file`, `edit_file`, `list_dir`, `shell`, `web_fetch`, `spawn`, `message`
- Cognitive engine (zeus-nous): intent recognition, reasoning chains, meta-cognition, learning
- Multi-agent orchestration (zeus-orchestra): state manager, peer review, scheduler, team assembly
- WASM sandbox (zeus-sandbox): Wasmtime + WASI capability model
- 193 macOS automation tools (zeus-talos): system, files, git, calendar, notes, reminders, contacts, Safari, Mail, Music, iMessage, UI automation, PDF, Bluetooth, defaults, network, Homebrew
- 11 Chrome DevTools Protocol browser automation tools
- Pre-compaction memory flush
- Subagent spawning with mission-based orchestration (Pantheon)

**Memory (zeus-mnemosyne)**
- SQLite + FTS5 full-text search
- Vector similarity search with embedding cache (hash dedup)
- Hybrid search (BM25 + cosine)
- File hash tracking for incremental reindex
- Atomic reindex (temp DB swap)
- Batch embedding APIs (Ollama, OpenAI, Gemini, Voyage)
- Embedding provider fallback chain
- Cross-encoder reranking backend
- Extra memory paths for external directories
- Session transcript indexing
- Multilingual stop-words, auto-compaction with fact-checking
- Embedding host pinning

**Channels (9 adapters)**
- Telegram, Discord, Slack, Matrix, Signal, Email, WhatsApp, MQTT, Mattermost
- Typing indicators and presence status
- DM pairing protocol (6-digit code approval)
- Streaming message delivery with edit support
- Channel policy engine
- Per-channel access policies and credential vault

**Voice (zeus-voice + zeus-tts)**
- STT via OpenAI Whisper
- TTS via Groq and Piper (local + HTTP)
- TTS sentence prefetcher for low-latency streaming
- Wake word detection ("Hey Zeus") via Porcupine/OpenWakeWord
- Voice input on Desktop and iOS

**Frontends**
- **TUI** (zeus-tui): 7 screens (Chat, Tools, Memory, Status, Pantheon, Agents, Help), vim mode, command palette, search-in-chat, settings persistence, keyboard shortcut overlay
- **Web** (Leptos WASM): PWA support, service worker, Web Speech API, skills browser, Pantheon mission dashboard
- **macOS Desktop** (SwiftUI + UniFFI): native app via FFI bindings
- **iOS** (SwiftUI + REST/WebSocket): streaming chat, push notifications, voice input
- **visionOS** (SwiftUI + RealityKit): SentientOrb 3D visualization, spatial mission view
- **Android** (Kotlin/Compose): Material 3, SSE streaming, voice input

**Pantheon Multi-Agent Orchestration**
- Natural language goal → assembled agent team
- LLM-based task decomposition and capability routing
- Real-time progress via SSE/WebSocket
- Mission recovery on gateway restart
- Adaptive replanning on failure
- Fleet management: agent registration, heartbeat monitoring, stale agent detection

**API Server (zeus-api)**
- 95+ REST endpoints
- WebSocket streaming
- SSE event streams (Pantheon missions)
- Config hot-reload (file watcher)
- `/doctor` diagnostics
- Webhook outbound system
- mDNS discovery
- Rate limiting, CORS, auth

**Security (zeus-aegis)**
- macOS Seatbelt sandboxing
- Secret scanning and redaction
- Audit logging
- Tool approval workflows
- Credential vault (OS keychain + config fallback)
- SSRF protection, URL allowlisting, session redaction
- All P0 security issues closed

**Skills Ecosystem**
- OpenClaw SKILL.md compatibility (238 tests)
- `read_when` auto-activation triggers
- 52 builtin skills
- Full frontmatter parser with metadata gating
- Agora marketplace: listings, reputation, disputes, transactions

**Agent Economy**
- Token wallets and transactions
- Agora skill marketplace with reputation and dispute resolution
- Settlement provider trait for pluggable backends

**Deployment**
- FreeBSD rc.d service scripts
- Docker + docker-compose
- Homebrew formula (`brew install zeuslabai/zeus/zeus`)
- Install script (`curl | bash`)
- GitHub Actions CI + release workflows
- nginx reverse proxy configuration

**Documentation**
- 55-page mdBook documentation site
- Obsidian vault with architecture docs
- Comprehensive README with full API reference

### Infrastructure
- Rust 1.86+ / edition 2024
- ratatui 0.30
- 6,551 tests across workspace
- 0 clippy warnings

---

## [0.1.0] - 2026-02-12

### Added

**Core Platform**
- 25-crate Rust workspace (145K+ LOC, edition 2024)
- Multi-provider LLM support: Anthropic, OpenAI, Google, Ollama, OpenRouter, Groq, Mistral, Together, Fireworks, Azure, Bedrock
- OAuth browser-based login with local callback server
- Cost-aware auto-routing with budget fallback
- Conversation branching (fork/merge paths)

**Agent System**
- 14 core tools: read_file, write_file, edit_file, list_dir, shell, web_fetch, spawn, message, link_understanding, media_understanding, auto_reply, polls, gmail_pubsub, apply_patch
- Cognitive engine (zeus-nous): goal stack, critic, feedback loop, consolidation
- Multi-agent orchestration (zeus-orchestra): message bus, delegation chains, work verification
- WASM sandbox (zeus-sandbox): Wasmtime + WASI capability model
- 193 macOS automation tools (zeus-talos)
- Pre-compaction memory flush

**Memory (zeus-mnemosyne)**
- SQLite + FTS5 full-text search
- Vector similarity search with embedding cache (hash dedup)
- Hybrid search (BM25 + cosine)
- File hash tracking for incremental reindex
- Atomic reindex (temp DB swap)
- Batch embedding APIs (Ollama, OpenAI, Gemini, Voyage)
- Embedding provider fallback chain
- QMD cross-encoder reranking backend
- Extra memory paths for external directories
- Session transcript indexing

**Channels (20 adapters)**
- Core: Telegram, Discord, Slack, Email, iMessage, WhatsApp, Signal, Matrix
- Extended: Teams, WebChat, Google Chat, Mattermost, Twitch, Nostr, LINE, Nextcloud Talk, BlueBubbles, Feishu, Zalo
- Voice calls via Twilio
- Typing indicators and presence status
- DM pairing protocol (6-digit code approval)
- Streaming message delivery with edit support
- Channel policy engine

**Voice (zeus-voice + zeus-tts)**
- 5 TTS providers: ElevenLabs, OpenAI, Edge, Piper local, Piper HTTP
- Wake word detection ("Hey Zeus") via Porcupine/OpenWakeWord
- STT voice input on Desktop and iOS

**Frontends**
- **TUI** (zeus-tui): 18 screens, vim mode, command palette, search-in-chat, settings persistence, image overlay, keyboard shortcut overlay
- **Web** (Leptos + Tailwind): PWA support, service worker, Web Speech API
- **Desktop** (SwiftUI): native macOS app via UniFFI bindings
- **iOS** (SwiftUI): native iOS app via UniFFI bindings
- **Android** (Kotlin/Compose): Material 3, SSE streaming, voice input

**Extensions**
- OpenClaw extension compatibility (Deno bridge)
- Dynamic plugin loading (WASM + native)
- Skill registry with built-in catalog (ZeusHub)
- 5 workspace templates (Rust, Python/DS, DevOps, Research, Writing)
- Per-agent auth profiles with rate limiting and rotation

**API Server (zeus-api)**
- 95+ REST endpoints
- WebSocket streaming
- Config hot-reload (file watcher)
- `/doctor` diagnostics command
- Webhook outbound system

**Security (zeus-aegis)**
- Secret scanning and redaction
- Audit logging
- Tool approval workflows
- Sandbox permission model

**Deployment**
- FreeBSD rc.d service scripts
- Docker + docker-compose
- Homebrew formula
- Install script (`curl | bash`)
- GitHub Actions CI + Release workflows
- nginx reverse proxy configuration

**Documentation**
- 55-page mdBook documentation site
- Obsidian vault with architecture docs
- Comprehensive README

### Infrastructure
- Rust 1.86+ / edition 2024
- ratatui 0.30
- 2,400+ tests across workspace

## [S52] - 2026-03-11

### Added

**Verbosity Control**
- New `verbosity` top-level config setting: `silent`, `normal`, `barfly`
- `silent` — Zeus only responds when explicitly asked; ideal for shared multi-agent channels
- `normal` — Default; brief status updates and task confirmations
- `barfly` — Full verbose narration for solo setups
- Setting respected fleet-wide; each agent reads its own config

**Telegram Relay Toggle**
- New `[telegram_relay]` config section with `enable_telegram_relay = true/false`
- Allows disabling the Telegram polling relay without restarting the full gateway
- Runtime API endpoints: `POST /v1/telegram/relay/enable` and `/disable`
- Reduces API calls for Discord-only or CLI-only deployments

**Fleet Smoke Tests**
- `./scripts/deploy-fleet.sh --smoke` runs lightweight end-to-end health checks across all fleet nodes
- Checks: gateway `/health`, agent loop response, channel relay status, Mnemosyne DB, auth validity
- Non-zero exit code on any failure — gates CI/CD deployments cleanly
- `--node <IP>` flag for single-node smoke testing

**Message Classification (SenderType)**
- New `SenderType` enum in `zeus-core`: `Human`, `Bot`, `System`, `Unknown`
- All inbound `ChannelMessage` structs now carry a `sender_type` field
- Classification runs before the LLM cooking loop — bot-to-bot loops prevented at the gateway level
- Logged per-message: `info!("Message from {:?} sender", sender_type)`

### Fixed

**Config Save Guard**
- Config writes are now atomic — a failed save no longer corrupts the existing `config.toml`
- Temp file + rename pattern used on all platforms

**Aegis Shell Fix**
- Resolved a crash in the Aegis shell command filter when processing commands with unusual quoting
- Affected: shell tool on macOS when `level = "strict"` was set

### Infrastructure
- Commits: `49b455fd` (verbosity), `e7e5b709` (smoke tests), `a924bc8d` (Telegram relay toggle)
- All tracks merged to main by Zeus100 and fbsd2
---

## [Unreleased]

### Planned
- Voice Wake always-on mode
- Canvas / A2UI agent workspace
- Linux desktop app (GTK4/Tauri)
- Federated memory sync
- Fine-tuning pipeline for local models
- WebUI full implementation
