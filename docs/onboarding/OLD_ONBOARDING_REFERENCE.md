# Old TUI Onboarding — Technical Reference

**Purpose:** Authoritative, screen-by-screen reference for how the **old (proven-working) TUI onboarding** performs its tasks and writes config — so the team can bring the **new `screens/` onboarding** to functional parity without working blind.

**Source:** `crates/zeus-tui/src/onboarding/mod.rs` (preserved on branch `feat/webui-copy-polish-herald`; the new flow is `crates/zeus-tui/src/screens/*` + persist in `app.rs`).
**Gold output:** the working `zeus-spark` config (`zai/glm-5.2`, populated `[credentials] ZAI_API_KEY`, `[provider_credentials.zai]`, full `[gateway]`/`[[bindings]]`). The broken `zeus-freebsd` config (new flow) had `glm/glm-5.2`, empty `[credentials]`, no `[provider_credentials]`.

> Authored by Zeus100 (coord) 2026-06-22 from a direct read of `onboarding/mod.rs`. Line refs are to that file.

---

## 1. Flow overview — 20 steps (`OnboardingStep`, mod.rs:14)

| # | Step | Code | Title | Keybinds (`help()`) |
|---|------|------|-------|---------------------|
| 0 | Welcome | WLCM | Welcome | `Enter=Continue  N=Exit` |
| 1 | SetupMode | MODE | Setup Mode | `↑/↓=Navigate  Enter=Select  Esc=Back` |
| 2 | QuickStart | QCFG | QuickStart | `Tab=Next Field  Enter=Continue  Esc=Back` |
| 3 | Provider | PROV | Provider | `←/→=Browse  Enter=Select  Esc=Back` |
| 4 | Auth | AUTH | Auth | `Tab=Switch Mode (Key/Token/Browser)  Enter=Continue  Esc=Back` |
| 5 | Model | MODL | Model | `↑/↓=Navigate  Enter=Confirm  Esc=Back` |
| 6 | Fallback | FLBK | Backup LLMs | `↑/↓=Navigate  Space=Toggle  Enter=Continue  Esc=Back` |
| 7 | Channels | CHAN | Channels | `Space=Toggle  Enter=Continue  Esc=Back` |
| 8 | ChanConfig | CCFG | Chan Config | `Tab=Next Field  Enter=Continue  Esc=Back` |
| 8b | SignalPair | SQRP | Signal Pair | `Enter=Continue (after scan)  Esc=Skip` *(only if Signal toggled)* |
| 8c | WhatsAppPair | WQRP | WhatsApp Pair | `Enter=Continue (after scan)  Esc=Skip` *(only if WhatsApp toggled)* |
| 9 | Gateway | GTWY | Gateway | `Tab=Next Field  Enter=Continue  Esc=Back` |
| 10 | Agent | AGNT | Agent | `↑/↓=Navigate  Tab=Custom  Enter=Select  Esc=Back` |
| 11 | Workspace | WKSP | Workspace | `Enter=Generate  Esc=Back` |
| 12 | Security | SECR | Security | `↑/↓=Navigate  Enter=Confirm  Esc=Back` |
| 13 | Features | FEAT | Features | `↑/↓=Navigate  Space=Toggle  Enter=Continue  Esc=Back` |
| 14 | Voice | VOIC | Voice | `Tab=Next Field  Enter=Continue  Esc=Back` |
| 15 | Images | IMGS | Images | `Tab=Next Field  Enter=Continue  Esc=Back` |
| 16 | Orchestration | ORCH | Orchestration | `Tab=Next Field  Enter=Continue  Esc=Back` |
| 17 | Memory | MNEM | Memory | `Tab=Next Field  Enter=Continue  Esc=Back` |
| 18 | Skills | SKILLS | Skills | `Space=Toggle  A=All  N=None  Enter=Install  Esc=Back` |
| 19 | Complete | — | Complete | `↑/↓=Navigate  Enter=Launch  Esc=Back` |

**Conditional routing (`advance()`, mod.rs:1110):**
- **SetupMode "Skip" (mode 2)** → jump straight to **Complete** (reuse existing config).
- **SetupMode "Manual" (mode 1)** → skip QuickStart, go to **Provider** (configure everything).
- **SignalPair** shown only if Signal (channel idx 3) toggled; **WhatsAppPair** only if WhatsApp (idx 6) toggled; otherwise both skipped → Gateway.

---

## 2. The canonical PROVIDERS table (mod.rs:444-452)

The **single source of truth** for provider identity. `provider_id` and `env_var` frequently differ from the display name — this is the #1 thing the new flow got wrong (it used the lowercased display name).

| Display | `provider_id` (model prefix) | `env_var` (`[credentials]` key) |
|---------|------------------------------|----------------------------------|
| Anthropic | `anthropic` | `ANTHROPIC_API_KEY` |
| OpenAI | `openai` | `OPENAI_API_KEY` |
| Google | `google` | `GOOGLE_API_KEY` |
| Ollama | `ollama` | `OLLAMA_HOST` (URL, not a key) |
| Gemini CLI | `google-gemini-cli` | *(none — browser OAuth only)* |
| **Kimi** | **`moonshot`** | **`MOONSHOT_API_KEY`** |
| **GLM** | **`zai`** | **`ZAI_API_KEY`** |
| Qwen | `qwen` | `QWEN_API_KEY` |
| MiniMax | `minimax` | `MINIMAX_API_KEY` |

**Model string** = `format!("{}/{}", provider_id, model_name)` (mod.rs:1591). So GLM → `zai/glm-5.2`, Kimi → `moonshot/...`. `parse_model()` (zeus-core) maps `"zai"|"glm"|"zhipu" → Provider::Zai`, and `env_key(Zai) = "ZAI_API_KEY"` — so `[credentials] ZAI_API_KEY` is the path the gateway's S70 bridge (`zeus-api/lib.rs:849`) exports → `zeus-llm` reads `env::var(provider.env_key())`.

---

## 3. Per-screen detail

### 0. Welcome (WLCM)
Splash / "awaken" intro. Enter→continue, N→exit. No config impact.

### 1. SetupMode (MODE)
Choose **Quick / Manual / Skip** (`setup_mode` 0/1/2). Skip→Complete (reuse config); Manual→Provider (skip QuickStart prefill).

### 2. QuickStart (QCFG)
Prefill common fields (e.g. gateway port — `quickstart_fields[0]`). Skipped in Manual mode.

### 3. Provider (PROV)
Browse the PROVIDERS table (`←/→`), select one → `selected_provider`. Detection: `providers_with_detection` flags providers whose key is already in env.

### 4. Auth (AUTH) — the most complex step
On entry (`advance()`): **resets** `api_key`/`oauth_token`/`cli_cred` (prevents stale-token leak across provider changes, #8); sets default `auth_mode` per provider (`google-gemini-cli`→2 Browser; `minimax`/`qwen`→0 Key, Tab→Device Code); pre-fills Ollama URL from `OLLAMA_HOST`; detects CLI creds (`detect_cli_credential` — codex/gemini/zeus tokens, mod.rs:229).
**`Tab` cycles auth modes:** `0`=API Key, `2`=Login with Browser (OAuth via `zeus_auth::run_oauth_flow`), `3`=Device Code (Qwen/MiniMax — displays user code + verification URL, polls for token).
**Validation (advance):** proceed only if a key/token entered **OR** provider detected in env **OR** Ollama (no key). Else error "Enter an API key or OAuth token to continue."

### 5. Model (MODL)
`↑/↓` through **live-fetched** models (`fetch_models(provider_id, api_key)`, mod.rs:2481 — per-provider endpoints). **No hardcoded fallback** — `current_models()` returns guidance strings ("Press Enter on Auth step with a valid key to load models") when none fetched. `selected_model_string()` strips display metadata (e.g. `(16.8GB, …)`) to the bare model name.

### 6. Fallback (FLBK)
`Space` toggles backup LLM providers → `fallback_models[]` → written as top-level `fallback_models = [...]`.

### 7. Channels (CHAN)
`Space` toggles among **12 channels** → `channel_toggled` set: Discord, Telegram, IRC, Signal, X/Twitter, Pantheon, WhatsApp, Matrix, Slack, Email, MQTT, Mattermost.

### 8. ChanConfig (CCFG)
Per-toggled-channel field entry (flat `chan_config_fields` vec). **Required-field validation** blocks advance (mod.rs:1343): e.g. Discord needs channel_id+guild_id; Telegram bot_token; Matrix homeserver+user_id; etc. (full map in source).

### 8b/8c. SignalPair / WhatsAppPair
Conditional QR-pairing. On entry, spawns `fetch_signal_qr_uri` (signal-cli) / `fetch_whatsapp_qr` (bridge). Keeps child process alive through handshake.

### 9. Gateway (GTWY)
`gateway_fields[0]` = host (normalizes `http://h:p`→host; default `0.0.0.0`). Port from `quickstart_fields[0]` (default 8080).

### 10. Agent (AGNT)
Persona pick (`load_personalities()`, mod.rs:503) + agent name + user name/role/org. Tab order: agent_name→user_name→user_role→user_org→persona. **On leaving Agent → `generate_workspace()`** (writes AGENTS.md/SOUL.md/etc., mod.rs:1999) if not already generated.

### 11. Workspace (WKSP)
Confirms/generates the workspace dir + files. `Enter=Generate`.

### 12. Security (SECR)
Aegis level select → `[aegis] level`.

### 13. Features (FEAT)
Toggle subsystems → `[council]` (if enabled), `[nous] enable_learning=false` (if disabled), `[athena]`, `[hermes]`.

### 14. Voice (VOIC)
STT/TTS config → `[deployment]` `whisper_stt_url`/`piper_tts_url`/`elevenlabs_api_key`/`tts_provider`.

### 15. Images (IMGS)
Image-gen config → `[images]` `provider`/`url`.

### 16. Orchestration (ORCH)
`orch_fields[0]` = heartbeat enabled → drives `[prometheus].enable_heartbeat` + `[gateway].enable_heartbeat`.

### 17. Memory (MNEM)
Mnemosyne config → `[mnemosyne]` `db_path`/`enable_fts`/`embedding_provider`.

### 18. Skills (SKILLS)
`load_skills()` (mod.rs:604) list; `Space` toggle, `A`=all, `N`=none, `Enter`=install.

### 19. Complete
`Enter=Launch` → **`save_config()`** + `complete=true`. (In the new flow, AWAKEN spawns the gateway here — confirmed working on zeus-freebsd.)

---

## 4. `save_config()` — config.toml generation (mod.rs:1620-1998)

**Single source of truth = config.toml** (comment: "No env vars, no credentials.json"). Writes, in order:

```toml
model = "{provider_id}/{model}"      # NEVER writes empty/"/unknown" — falls back to existing config model, else aborts with [WARNING]
fallback_models = [...]              # only if backups configured
workspace = "..."
sessions = "..."
max_iterations = 20
onboarding_complete = true
verbosity = "normal"

[tui]            theme="dark", vim_mode=false
[auth]           use_oauth=false
[prometheus]     enable_heartbeat={hb}, heartbeat_interval_secs=300, enable_cognitive=true, max_iterations=20
[gateway]        host, port, enable_channels=true, enable_heartbeat={hb}, enable_agent_processing=true, timeout_secs=1800
```

**Channels** (per toggled, mod.rs:1701+): section name mapping is non-obvious —
- `telegram` → **`[telegram_relay]`** (Bot API; `[channels.telegram]` would trigger MTProto)
- `signal` → **`[signal_relay]`** (signal-cli HTTP daemon)
- all others → `[channels.{name}]`
- **Discord:** `token` in `[channels.discord]`; **`channel_id`+`guild_id` → `[[bindings]]`**; `role_ids` → `[gateway]`; plus `allow_bots`.
- Numeric fields (`port`, `http_port`) written as **bare integers**; IRC `channels` as a **TOML array**.

**Credentials** (mod.rs:1840+):
- Ollama → `[ollama] url = "{api_key}"` (the field holds the URL)
- else → **`[credentials]\n{env_var} = "{key}"`** ← load-bearing auth (read by S70 bridge)
- Bedrock → also `AWS_SECRET_ACCESS_KEY`, `AWS_REGION`
- **`[provider_credentials.{provider_id}]`** with `cred_type = "oauth"|"api_key"` + `token` (mod.rs:1950/1956) — `provider_id` hyphens→underscores for the TOML key.

**Subsystems:** `[aegis] level` · `[council]` (if enabled) · `[nous] enable_learning=false` (if disabled) · `[athena] vault_path` · `[hermes] default_channel` · `[deployment]` (voice) · `[images]` · `[mnemosyne]` · `[agent] persona/name`.

---

## 5. New-onboarding gap analysis + per-screen task map

The new `screens/` flow (persist in `app.rs`) diverged. Known gaps from live testing (build `a1c4f29`):

| Gap | Old behavior | New (broken) | Owner / Task |
|-----|--------------|--------------|--------------|
| **P0 model prefix** | `{canonical provider_id}/{model}` (`zai/...`) via PROVIDERS table | lowercased display name (`glm/...`) | **#254 / zeus106** |
| **P0 credentials** | `[credentials] {env_var}={key}` + `[provider_credentials.{id}]` | key written only to `.env` (empty `[credentials]`) | **#254 / zeus106** (fix `d7a755f4` writes `[credentials]`) |
| Provider step render | clean card + detail | bleed-through overlap | **#250 / zeus112** |
| Model step | **live fetch only**, no hardcoded fallback; honest empty-states | 2 hardcoded models under fake "LIVE FETCH" badge | **#251 / zeus-spark** |
| Footer keybinds | per-screen `help()` strings (above) | wrong/garbled ("← BACK rd…") | **#252 / zeus112** |
| Agent persona render | clean grid | Name/Role/Tone fields overlap cards | **#253 / zeus112** |
| Sparse sections | full set written | sparse — **OK**, all sections serde-default (verified, #254) | n/a |

**Parity checklist for the new flow (per provider, all 12 — merakizzz: "for ALL providers"):**
1. Model = `{canonical provider_id}/{model}` (PROVIDERS table / `Provider` canonical id), NOT display name.
2. `[credentials] {env_var} = {key}` keyed from the table.
3. `[provider_credentials.{provider_id}]` (`cred_type`+`token`).
4. Telegram→`[telegram_relay]`, Signal→`[signal_relay]`, Discord channel/guild→`[[bindings]]`.
5. Live model fetch only — no hardcoded models; honest empty/fallback states.
6. E2E: fresh onboard (GLM **and** Anthropic) → gateway → real chat reply.
