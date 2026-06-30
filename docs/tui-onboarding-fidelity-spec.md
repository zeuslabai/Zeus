# TUI Onboarding — 1:1 Fidelity Spec (rebuild → prototype)

**Base:** `feat/tui-gateway-integration` (the rebuild). Goal: each onboarding screen **1:1 with `docs/zeus-tui-onboarding.jsx`**.
**Allowed diffs only:** providers (the confirmed 12 set), channels (supported set). Everything else matches the prototype exactly.
**Gate:** RENDER-DIFF — diff the rendered TestBackend buffer (or live screenshot) against the prototype, NOT token/palette match.

## Behavior / keybindings (ALL screens)
- **ESC = back one step** (previous screen). On Welcome (step 0): ESC stays put (does NOT quit).
- **Ctrl+C / Ctrl+Q = quit** (everywhere).
- **↵ Enter** = continue/advance. Per-screen nav (↑/↓, ←/→, letter keys) per the prototype's footer hints.

## Per-screen JSX reference (the SoT — match each 1:1)
| # | Screen | JSX lines | rebuild file |
|---|--------|-----------|--------------|
| 01 | Welcome | 446–508 | screens/welcome.rs |
| 02 | Mode | 509–576 | screens/mode.rs |
| 03 | Provider | 577–648 | screens/provider.rs |
| 04 | Auth | 649–790 | screens/auth.rs |
| 05 | Model | 791–869 | screens/model.rs |
| 06 | Fallback | 870–950 | screens/fallback.rs |
| 07 | Channels | 951–1006 | screens/channels.rs |
| 08 | ChanConfig | 1007–1126 | screens/chanconfig.rs |
| 09 | Gateway | 1191–1262 | screens/gateway.rs |
| 10 | Agent | 1263–1308 | screens/agent.rs |
| 11 | Workspace | 1309–1352 | screens/workspace.rs |
| 12 | Security | 1353–1409 | screens/security.rs |
| 13 | Features | 1410–1466 | screens/features.rs |
| 14 | Voice | 1467–1520 | screens/voice.rs |
| 15 | Images | 1521–1564 | screens/images.rs |
| 16 | Orchestration | 1565–1608 | screens/orchestration.rs |
| 17 | Memory | 1609–1653 | screens/memory.rs |
| 18 | Skills | 1654–1737 | screens/skills.rs |
| 19 | Complete | 1738–1800 | screens/complete.rs |

(SignalPair sub-step @1127–1190 shows if Signal channel selected.)
**Method per screen:** read the JSX ref + the rebuild screen → make the rebuild render **1:1** (chrome/layout/glyphs/colors/component shapes/copy) → RENDER-DIFF gate. Welcome + Provider below are the depth templates.

---
## Screen 01/19 — Welcome (WLCM) · JSX 446–508
**Target (1:1):**
1. ASCII ZEUS logo — 6 rows; colors [accent,accent,accentBright,accentBright,accentDim,muted]; rows 0–1 bold; centered by display width. ✅
2. `O P E R A T I N G   S Y S T E M` — accentDim bold, wide tracking. ✅
3. `Autonomous AI agents on your hardware` — dim. ✅
4. **ZeusFace box** — `(◉‿◉)` + italic dim `"Let's wake the fleet. This won't take long."`; 1px muted border, bg2. ✅ static layout matches. **Animation = cross-cutting FOLLOW-UP** (no tick/frame infra in App yet — a separate task adds it once for all faces; per 106 2026-06-18). Ship static 1:1 now.
5. **INITIATE card** — 480w, 1px muted border. ❌ MISSING → ADD: header `▸ INITIATE` (accent) · right `v0.4.7 · 391,269 LOC · 365 tools`; body blurb + 3 rows (`19 STEPS`/`10 req,9 opt` · `~5 MIN`/`QuickStart` · `~25 MIN`/`Full`); footer `↵ Continue` · `N Exit` · right `build <sha> · main`.
6. **existing-config box** — ❌ FIX COPY: amber box, header `↻ EXISTING CONFIG DETECTED`, body "Welcome back, Zeus100. Re-running will pre-populate fields from your current config." (rebuild wrongly says "…will overwrite config.toml").
7. `Press ↵ Enter to begin` — muted. ✅
**Verdict** — add INITIATE card + fix existing-config copy/box → Welcome at static-1:1. **Cross-cutting follow-up:** ZeusFace animation (App tick/frame plumbing, applies to all faces).

---
## Screen 03/19 — Provider (PROV) · JSX 577–648
**Layout (1:1) — 3 columns:**
- LEFT 360w (right border): StepHeader "Pick your LLM provider" / "Primary model that powers agent reasoning"; scrollable provider Cards (glyph badge + name + sub); footer "{N} providers · Sorted by usage frequency".
- CENTER flex: selected detail — 56×56 glyph badge (provider color, bg text); name (18 bold) + FEATURED (amber pill) / `● DETECTED` (green pill); sub (dim); row FLAGSHIP / PRICING / KEY FORMAT (labels accentDim bold, values fg, keyFmt accentBright); box `WILL WRITE TO ~/.zeus/config.toml` → `model = "{id}/{flagship}"`; NEXT hint.
- RIGHT 200w (left border, bg2): HINTS blurb + RECOMMENDATIONS list.
**Allowed diff — provider LIST = the confirmed 12** (NOT the JSX/rebuild stale list):
Anthropic(ANT,`claude-opus-4-8`) · OpenAI(OAI,`gpt-4o`) · Google(GCP,`gemini-2.5-pro`) · Ollama(OLM,local,`● DETECTED`) · Gemini CLI(GMC,OAuth) · Kimi(KMI,`kimi-k2.7-code`) · GLM(GLM,`glm-5.2`) · Qwen(QWN) · MiniMax(MNX,`MiniMax-M3`) · MiMo(MMO) · OpenRouter(OR,`auto`) · Grok(GRK).
**Gaps:** rebuild `PROVIDERS` = stale JSX list → replace with the 12; Anthropic flagship `claude-opus-4-7`→`claude-opus-4-8`; update RECOMMENDATIONS to the 12; render-verify the 3-col chrome vs JSX.

---
## Screen 02/19 — Mode (MODE) · JSX 509–576
**Target (1:1):** header "Choose your setup mode" (16 bold) + sub (re-run zeus onboard anytime). **3-column GRID** of mode cards: QuickStart(QS, green, "1 LLM, 1 channel, sane defaults", ~3min, "1 step left") · Full Setup(FU, accent, "Walk every section", ~25min, "17 steps left") · Custom(CU, cyan, "Pick which steps", varies, "you choose"). Each card: 48×48 glyph badge (filled when selected), name (16 bold), sub (dim), TIME/STEPS rows, `▸ SELECTED` top-right badge when selected, left-border = mode color, minHeight 200. NOTE box: "Skipped sections … zeus onboard --resume / edit config.toml".
**Gaps/watch:** cards are HORIZONTAL → nav MUST be **←/→** (earlier rebuild bug: only ↑/↓ wired → cards un-selectable). Render-verify the 3-col grid + selected accent-fill.

## Screen 05/19 — Model (MODL) · JSX 791–869
**Target (1:1):** header "Pick a model" + sub "From {provider}'s catalog … zeus config set model". Vertical model-card list: radio (●) + name + dim `id` + `★ RECOMMENDED` (green) + sub; right CONTEXT (accent) + PRICING. ollama: `● LIVE FETCH` box "localhost:11434/api/tags · N models".
**Allowed diff — per-provider catalogs = CURRENT/the 12:** anthropic→`claude-opus-4-8` (NOT 4-7) / sonnet-4-6 / haiku-4-5; kimi→`kimi-k2.7-code` (+highspeed); GLM→`glm-5.2`; + qwen / minimax(`MiniMax-M3`) / mimo / google(gemini-2.5-pro) / openai(gpt-4o) / grok / openrouter(auto). LIVE FETCH for ollama + zai(`api.z.ai`) + moonshot(`api.moonshot.ai`) — mirror the on-main GLM-5.2/kimi fixes.
**Gaps:** rebuild model catalogs stale (opus-4-7, missing kimi/glm/qwen/etc) → update to current + add zai/moonshot live-fetch.

## Screen 04/19 — Auth (AUTH) · JSX 649–790
**Target (1:1):** header "Authenticate with {provider}" (16 bold) + sub "Credentials persist to ~/.zeus/config.toml [credentials] with 0600 permissions." **Mode tabs** (active = 2px accent bottom-border): API Key / Setup Token / Browser OAuth.
- KEY mode: `API KEY` label + Field (placeholder = provider keyFmt, secret, validity check + hint "Expected format: {keyFmt}") + `▸ TEST CONNECTION` button (states TESTING…/success/test) → success line "● /v1/models returned 200 · <ms> · <N> models available".
- TOKEN mode: `SETUP TOKEN` + `↻ Detected` (amber) + Field "Token" (secret).
- BROWSER mode: `OAUTH FLOW` + browser-callback UI (anthropic/openai/google/**gemini-cli** OAuth).
- `WILL WRITE TO ~/.zeus/config.toml [credentials]` box → `{id}_api_key = "***{last4}"`.
**Gaps (106 substrate-walk 2026-06-18):** (1) provider propagation **ALREADY FIXED** (app.rs drives AuthScreen from `provider_display(provider_selected)`; the "Anthropic" literal was a test fixture — NOT a live bug; my earlier spec was wrong). (2) masking is char-safe but **wrong SHAPE** — current `chars().take(8)+"*"×n`; JSX 784 wants `***{last4}` → fix via `chars().rev().take(4)` (stays panic-safe). (3) 3 mode tabs + OAuth flow + TEST CONNECTION all **already present** — just verify the gemini-cli OAuth path. So Auth = a one-line masking-shape fix + a gemini-cli check.

## Screen 06/19 — Fallback (FBCK) · JSX 870–950
**Layout (1:1) — 2 columns:**
- LEFT flex: header "Backup LLM chain" (16 bold) + sub "If your primary provider fails, the agent loop tries each fallback in order. Pick 0-3 backups." Then `AVAILABLE` label (accentDim bold, tracking 3). List of candidate providers (= all providers except the primary), first 6 shown: each row = checkbox (✓ filled accent bg when in chain) + glyph badge (provider color) + name (fg) + flagship (dim, right). Row chrome: 1px muted border, **left-border 2px provider color**, bg = accentFaint when in chain else bg2, opacity 0.7 when not in chain.
- RIGHT 360w (left border): `FALLBACK CHAIN ({N})` label + hint "Reorder with `[` / `]`" (brackets accent). Empty: dashed muted box "No fallbacks selected. / Primary failures will fail the agent loop." Chain rows: numbered badge (1/2/3, accent bg, bg text) + name (white 12 bold) + flagship (dim 9) + `↑ ↓ ✕` controls (dim). When chain>0: SUGGESTED box (bg2, muted border, accentDim label) suggesting cheap-fast backups.
**Allowed diff:** candidates = the 12 (minus primary). **SUGGESTED copy** hardcodes "OpenAI + **Groq**" in JSX — Groq is NOT in our 12 → adapt to our set (e.g. "OpenAI + Ollama" / pick 2 cheap-fast from the 12). **Nav:** ↑/↓ focus AVAILABLE, Space/Enter toggle into chain, `[`/`]` reorder, `✕`/Del remove. ESC=back.
**Gaps:** verify rebuild candidate list = the 12; fix the Groq suggestion; render-verify 2-col + checkbox/numbered-badge shapes.

---
## Screen 07/19 — Channels (CHAN) · JSX 951–1006
**Layout (1:1) — 2 columns:**
- LEFT flex: header "Pick messaging channels" (16 bold) + sub "Select which channels Zeus should bridge. Per-channel credentials collected next." Two GROUPS, each: group-header row = label (accentDim bold tracking 3, UPPER) + 1px muted rule + right note ("API key auth" / "QR pairing required"), then a **2-col GRID** (1fr 1fr, gap 6) of multiselect channel Cards (checkbox + glyph + name + desc + sdk; toggled/focused states).
- RIGHT 280w (left border): `SELECTED ({N})` label + selected list (glyph + name + dim "next: config"). Empty: dashed box "No channels selected. / Zeus will run console-only."
**Channel set = the SUPPORTED 8 (NO diff — these 8 match zeus-channels' adapters exactly):** Cloud APIs: Telegram(TG,blue,"grammers MTProto") · Discord(DC,purple,"Serenity gateway") · Slack(SL,green,"Socket Mode + Web API") · Email(EM,amber,"lettre SMTP + IMAP"). Phone-paired: iMessage(iM,cyan,"AppleScript bridge") · WhatsApp(WA,green,"Cloud API") · Signal(SG,blue,"signal-cli JSON-RPC") · Matrix(MX,accent,"matrix-sdk v0.16").
**Nav:** ↑/↓/←/→ move focus across the grid, Space/Enter toggle. ESC=back. **Gaps:** verify the 8-card 2-col grid + 2 group sections + SELECTED side panel render 1:1.

---
## Screen 08/19 — ChanConfig (CCFG) · JSX 1007–1126
**Layout (1:1):** header "Configure {N} channel{s}" + sub "All channels visible — fill in any order. Test buttons send a \"Zeus connected ✅\" message to verify." Then a vertical stack (gap 14) of one config card **per SELECTED channel**:
- Card: 1px muted border, left 2px channel color, bg2. **Header row:** glyph badge (32×18, channel-color bg, bg text) + name (white 13 bold) + sdk (dim italic) + spacer + state badge: `QR PAIRING` (amber, signal/whatsapp) · `APPLESCRIPT` (cyan, imessage) · `✓ TESTED` (green, after success).
- **Body fields (fieldsByChannel):** telegram→API ID(req)·API Hash(secret req)·Phone(req); discord→Bot Token(secret req)·Default Channel ID(opt); slack→Bot Token(secret req)·App Token(secret req); email→SMTP Host(req)·SMTP Port(def 587 req)·Username(req)·App Password(secret req); whatsapp→Phone Number ID(req)·Access Token(secret req); matrix→Homeserver URL(req)·Username(req)·Password(secret req); **imessage→none** (AppleScript info line "● Uses native macOS bridge. No credentials needed. …Messages permission on first use."); **signal→none** (QR info "⚠ Requires phone-side QR scan. Pairing screen will display after this step.").
- Field: label left (≈152w gutter for button alignment) + input (secret masked) + placeholder. When fields>0: `▸ SEND TEST` button (states ▸ SEND TEST / ▸ SENDING… / ✓ DELIVERED) + success line "● Test message delivered to {name}".
**Secret masking:** reuse Auth's char-safe `***{last4}` (`chars().rev().take(4)`) — consistency across screens, no byte-slice. **Nav:** Tab/↑↓ between fields, Enter triggers focused channel's Test. ESC=back. **(SignalPair sub-step @1127–1190 renders after this if Signal selected.)** **Gaps:** verify per-channel field sets exact + QR/AppleScript info lines + Test button states render 1:1.

---
## Screen 09/19 — Gateway (GTWY) · JSX 1191–1262
**Layout (1:1):** header "Configure gateway" + sub "The gateway hosts the API, WebUI, and agent processing loop." Three sections:
- **BIND:** Field Host (default `127.0.0.1`, required, hint "Use 0.0.0.0 to expose on LAN") + Field Port (default `8080`, required) with **error line when port in use** (JSX: "Port 8080 in use by PID … Pick a different port or stop the existing instance").
- **FEATURES:** 3 pill-toggle rows (30×16 toggle, label + desc): Agent Processing Loop (**default ON**, "Background heartbeat + cron + watchdog") · WebUI Co-host (**default ON**, "Serves Leptos frontend on the same port (or 8081 if 8080 is taken)") · MCP Server (**default OFF**, "Model Context Protocol endpoint for Claude Desktop / cursor").
- **INSTALL AS SERVICE:** 4-col grid of Cards: launchd(MAC,"macOS native (recommended)",`~/Library/LaunchAgents/ai.zeuslab.gateway.plist`) · systemd(LIN,"Linux native",`/etc/systemd/system/zeus-gateway.service`) · rc.d(BSD,"FreeBSD native",`/usr/local/etc/rc.d/zeus_gateway`) · Manual(—,"I'll start zeus manually",no path). Selected w/ path → "WILL INSTALL {path}" box (accentBright path).
**Allowed diff / gaps:** (1) **port-in-use must be a REAL check** — JSX hardcodes "PID 47291"; wire to an actual bind probe. (2) **service auto-select by platform** — JSX dims non-launchd cards (mac mock); on the running platform highlight the native service (launchd/systemd/rc.d) instead of hardcoding launchd-default. **Nav:** Tab fields/toggles, grid-select service. ESC=back.

---
## Screen 10/19 — Agent (AGNT) · JSX 1263–1308
**Layout (1:1) — 2 columns:**
- LEFT: header "Agent persona" + sub "Pick an archetype to seed your agent's `SOUL.md`. Customize freely after onboarding." **2-col GRID** of persona Cards (the 6): Coordinator(COO,accent,"Orchestrates the fleet") · Engineer(ENG,cyan,"Writes and reviews code") · Creative(CRT,purple,"Marketing and content") · Sysadmin(OPS,green,"Monitors and maintains") · Analyst(ANL,amber,"Research and synthesis") · Custom(CST,dim,"Define your own"). Then IDENTITY: Field Agent Name (suggested `zeus{hostname-last}` else `Zeus100`, required, hint "Auto-suggested from hostname: {hostname}") · Field Role (default = persona name) · Field Tone (default = persona tone, hint "Used in SOUL.md prompt seed").
- RIGHT 360w (left border): **SOUL.MD PREVIEW** live box — `# {name}` (accent) / `## Role` {role} / `## Tone` {tone} / `## Guiding Principles` (per-persona bullets: coordinator/engineer/creative defined in JSX). Footer "Live preview · writes to `~/.zeus/workspace/SOUL.md`".
**Allowed diff / gaps:** persona set = the 6 (keep). **Name auto-suggest from REAL hostname** (not mock). Guiding-principles bullets only defined for coordinator/engineer/creative in JSX — extend to sysadmin/analyst or leave empty (decide; minor). **Nav:** grid-select persona, Tab fields. ESC=back.

---
## Screen 11/19 — Workspace (WKSP) · JSX 1309–1352
**Layout (1:1):** header "Workspace paths" + sub "Where Zeus stores your agent's working memory, sessions, and journal."
- **existing-detected amber box** (when `~/.zeus/workspace` exists): "↻ EXISTING WORKSPACE FOUND" + "{N} memory facts, {N} sessions, last modified {time}" + 2 buttons `USE EXISTING` (accent) / `START FRESH (BACKUP OLD)` (outline).
- **PATHS:** Field Workspace (default `~/.zeus/workspace`, hint "AGENTS.md, SOUL.md, journals, daily notes") · Field Sessions (default `~/.zeus/sessions`, hint "Per-conversation JSONL logs (grows ~5MB/day per active agent)") · Field Mnemosyne DB (default `~/.zeus/mnemosyne.db`, hint "SQLite + vector embeddings (can grow to GBs)").
- **DISK USAGE PROJECTION** box: 3-col (Workspace ~50MB "after 30 days" · Sessions ~150MB "@5MB/day for 30d" · Mnemosyne ~800MB "after 1000 sessions").
**Allowed diff / gaps:** existing-workspace counts (JSX hardcodes 2847/147/2min) **must be REAL** — scan the dir for fact/session counts + mtime. **Nav:** Tab fields, USE/FRESH buttons. ESC=back.

---
## Screen 12/19 — Security (SCTY) · JSX 1353–1409
**Layout (1:1):** header "Aegis security level" + sub "Sandbox aggressiveness for tool execution. Approval pipeline is always active regardless of level." **4-col grid** of level cards (minHeight 200): Strict(STR,red,"Shared-machine fleet bots"; BLOCKED: shell·web_fetch·apply_patch·fs_write outside workspace) · Standard(STD,amber,"Personal coding assistant",**★REC**; BLOCKED: shell w/ sudo·fs_write outside workspace+home) · Permissive(PRM,yellow,"Sandbox / research"; no blocked) · Custom(CST,dim,"Per-tool allowlist"). Card: glyph badge (36×36, level-color bg, bg text) + name (white 14) + sub (dim italic) + BLOCKED list (red label, `✕ {item}`, first 3). Selected → `▸ SELECTED` badge; recommended+unselected → `★ REC` (green) + green border. Selected → "SELECTED: {NAME}" box → `Will write [aegis] level = "{id}" to ~/.zeus/config.toml`.
**Allowed diff / gaps:** 4 levels (keep). **Nav:** ←/→ or grid-select. ESC=back. Render-verify the 4-col grid + BLOCKED lists + ▸ SELECTED/★ REC badge precedence.

---
## Screen 13/19 — Features (FEAT) · JSX 1410–1466
**Layout (1:1):** header "Enable subsystems" + sub "Toggle which Zeus crates are active in this deployment. Disabled crates compile but don't load."
- **Talos warning banner** (accent border): "⚠ MACOS GATE — TALOS IS MANDATORY" + body "the `[talos]` block must be present (even if empty) or 193 tools — incl. image-gen, AppleScript, system-info — silently fail to register. Talos is force-enabled here regardless of toggle."
- **Feature toggle list (the 8):** Talos (macOS automation, **FORCE-ON on macOS**, 193 tools) · Nous (cognitive learning) · Mnemosyne (memory, three-layer) · Hermes (channels) · Athena (research/Obsidian) · Browser (Chrome CDP, 11 tools) · Voice (TTS/STT, Twilio+Whisper) · Skill marketplace (plugins). Each row: pill toggle + name + desc + right `● ON`/`○ OFF`; mandatory (talos@macOS) → `FORCE-ON ON {PLATFORM}` pill + amber warning line + non-toggleable (opacity 0.7).
**Allowed diff / gaps:** **Talos FORCE-ON gated to macOS ONLY** — on Linux/BSD it's a normal toggle (don't hardcode mandatory). Feature set = the 8. **Nav:** ↑/↓ + Space/Enter toggle (skip mandatory). ESC=back.

---
## Screen 14/19 — Voice (VOIC) · JSX 1467–1520
**Layout (1:1) — 2 columns:** LEFT: header "Voice / TTS provider" + sub "Powers `voice_say`, `voice_call`, and Twilio outbound calls." Vertical Card list. RIGHT 380w: when a provider≠Skip selected → 42×42 glyph badge + name(14)+sub; **CREDENTIALS:** Field API Key (secret req) + Field Voice ID (default "default"); custom → Field Base URL (req, `http://localhost:5000`); `▸ TEST VOICE` button. When Skip → yellow box "⚠ NO VOICE CONFIGURED / Voice tools will be unavailable. Re-run `zeus onboard --resume voice` later."
**Allowed diff / gaps (CORRECTED 2026-06-18 — verified vs zeus-tts source):** real zeus-tts providers = **OpenAI · ElevenLabs · Edge · Piper/Local** (files: openai.rs/elevenlabs.rs/edge.rs/local.rs+piper.rs; NO Cartesia; Kokoro served via the custom OpenAI-compat endpoint, `kk.novaxai.ai`). So the ONLY diff vs JSX is **Cartesia → Edge TTS**; **ElevenLabs IS supported (keep it)**. Final set: ElevenLabs(11L) · OpenAI TTS(OAI) · Edge TTS(EDG) · Custom Endpoint(API, Piper/Kokoro) · Skip(—). Keep JSX layout/credentials shape; secret masking char-safe `***{last4}`. **Nav:** ↑/↓ select, Tab fields. ESC=back. (Prior note wrongly claimed ElevenLabs unsupported + listed macOS `say` which is not a zeus-tts provider.)

---
## Screen 15/19 — Images (IMGS) · JSX 1521–1564
**Layout (1:1) — 2 columns:** LEFT: header "Image generator" + sub "Powers `image_generate`, `image_edit`. Writes to `[talos.image]`." Vertical Card list: OpenAI GPT Image(OAI,"gpt-image-1") · Google NanoBanana(GCP,"gemini-2.5-flash-image") · BFL Flux(BFL,"flux-pro / dev / schnell") · OpenAI compat URL(API,"vLLM, fal.ai, proxies") · Automatic1111 URL(A11,"Z-Image Turbo path") · Skip. RIGHT 380w: selected≠Skip → glyph badge + name + sub; **CONFIG:** openai-custom/a1111 → Field Base URL (req; a1111 placeholder `http://dgx-spark:7860`); Field API Key (secret, req unless a1111); Field Model (req, placeholder=sub); a1111 → Field Steps (placeholder "1", hint "⚠ Z-Image Turbo: must be 1 (multi-step returns black PNG)").
**Allowed diff / gaps (CORRECTED 2026-06-18 — verified vs `ImageGenProviderType` in zeus-core + talos image_provider.rs):** real backends = **OpenAI · Automatic1111 · ComfyUI · Fooocus · OpenAI-compatible**. JSX's **Google NanoBanana + BFL Flux are NOT distinct talos backends** → swap to **ComfyUI + Fooocus**. Final set: OpenAI GPT Image(OAI,`gpt-image-1`) · Automatic1111(A11, Z-Image Turbo) · ComfyUI(CMF) · Fooocus(FOO, SDXL Turbo) · OpenAI compat URL(API) · Skip. The **a1111 Z-Image Turbo @ `dgx-spark:7860` + Steps=1 is our REAL DGX Spark path** (keep verbatim). Fields per provider: Base URL (a1111/comfyui/fooocus/compat), API Key secret `***{last4}` (openai/compat), Model, Steps (a1111). **Nav:** ↑/↓ select, Tab fields. ESC=back. (merakizzz may opt to feature NanoBanana/BFL as preset compat-URL cards — pending his call.)

---
## Screen 16/19 — Orchestration (ORCH) · JSX 1565–1608
**Layout (1:1):** header "Orchestration mode" + sub "How Zeus runs background work — heartbeat, cron, watchdog." **3-col grid** (minHeight 130): All-on(ALL,accent,"Heartbeat + cron + watchdog","Full autonomous operation. Recommended for fleet agents.",**★REC**) · Heartbeat-only(HB,amber,"Wake events only, no scheduled tasks","Reactive only…") · Disabled(OFF,dim,"Manual invocation only","No background activity…"). Card: glyph badge(36×22) + name(14) + sub(italic) + desc; ▸ SELECTED / ★ REC badges. When selected≠disabled → **HEARTBEAT TIMING:** Field Interval (default "300", hint "Seconds between heartbeat ticks (default 300 = 5 min)") + Field Quiet Start (default "23") + Field Quiet End (default "8").
**Allowed diff / gaps:** none (3 modes keep; matches our heartbeat/cron/watchdog). **Nav:** ←/→ grid-select, Tab fields. ESC=back.

---
## Screen 17/19 — Memory (MMRY) · JSX 1609–1653
**Layout (1:1) — 2 columns:** LEFT: header "Memory backend" + sub "Mnemosyne — semantic search over agent history. Pick embedding provider." Card list (large): Ollama(OLM,cyan,"Local, free, private",`nomic-embed-text`,**★REC**,**● DETECTED**) · OpenAI(OAI,green,"Cloud, paid, fast",`text-embedding-3-small`) · FTS-only(FTS,amber,"No embeddings, full-text search only"). **STORAGE:** Field DB Path (default `~/.zeus/mnemosyne.db`) + (selected≠none) Field Embedding Model (default per provider). RIGHT 280w: **DISK PROJECTION** (1K→~12MB · 10K→~120MB · 100K→~1.2GB · 1M→~12GB) + **OLLAMA DETECTED** cyan box "Found local Ollama at localhost:11434 with nomic-embed-text available…".
**Allowed diff / gaps:** **Ollama detection must be REAL** (probe localhost:11434 for nomic-embed-text — don't hardcode the green box). Embedding defaults (nomic-embed-text / text-embedding-3-small) match our mnemosyne — keep. **Nav:** ↑/↓ select, Tab fields. ESC=back.

---
## Screen 18/19 — Skills (SKIL) · JSX 1654–1737
**Layout (1:1):** header "Install starter skills" + sub "SKILL.md plugins from the registry. Each grants a set of tools." + **filter input** (`/` prefix, "filter…"). **Category tabs:** All + the SKILLS keys (Productivity · Dev · Marketing · Security · Research) w/ per-cat counts; right "N selected · M available". **2-col grid** of skill cards: checkbox + name + ★REC + desc + category tag (right). Toggle install.
**Allowed diff / gaps:** skills set should reflect the **real zeus-skills registry** (not mock entries). Categories = the 5. **Nav:** `/` filter, ←/→ or tab to switch category, ↑/↓/←/→ grid, Space/Enter toggle. ESC=back.

---
## Screen 19/19 — Complete (CMPL) · JSX 1738–1810
**Layout (1:1) — 2 columns:** LEFT: **ZeusFace** (state ready/working/success — ANIMATED, ties to the carved follow-up) + divider + "✓ Configuration complete" (18 bold) + sub "Review your setup before launch. All settings persist to `~/.zeus/config.toml`." **Summary list:** per-section rows (status dot green/muted/red + name + value + `✓ READY`/`⏭ SKIPPED`/`✕ ERROR` badge). Buttons: `▸ TEST ALL BACKENDS` (testing→passed states) + `▸ AWAKEN ZEUS` (accent). RIGHT 320w: **NEXT STEPS** — `$ zeus start` (Launches gateway + agent loop) · `$ zeus chat` (Interactive chat) · `$ zeus pantheon` (Multi-agent coordination chat) · …
**Allowed diff / gaps:** (1) **`AWAKEN ZEUS` must do REAL work** — write config.toml + launch the gateway (not mock); ties to the WebUI/bootstrap launch path. (2) **`TEST ALL BACKENDS` = real connectivity checks** per configured provider/channel. (3) **Summary reflects real configured state** (not the JSX mock summary). (4) ZeusFace animation = the cross-cutting follow-up. **Nav:** Tab buttons, Enter=AWAKEN. ESC=back.

---
**✅ SPEC COMPLETE — all 19 screens detailed.** Phase 2 (06–19) ready to fan. Cross-cutting follow-ups tracked: ZeusFace animation (App tick/frame); live build-SHA Welcome footer (build.rs/vergen); the duplicated 12-provider list → ONE shared source (Provider/Model/Fallback); REAL probes for port-in-use (Gateway), existing-workspace counts (Workspace), Ollama detection (Memory), and AWAKEN/TEST wiring (Complete).
