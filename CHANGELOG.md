# Changelog

All notable changes to Zeus are documented here.

---

## [Unreleased] - 2026-07-13

Soul-pipeline hardening, on-chain wallet surfaces (API + TUI), WebUI onboarding parity (both phases complete), XAI capability isolation, a hermetic installer fix, a FreeBSD fresh-install deadlock fix, the release-publish path unblocked, the missing OpenRouter/xAI/Sakana providers restored, Ollama per-model capability probing, portable TUI newline/copy handling, a WebUI build-stamp footer, and published Terms of Service + Privacy Policy.

### Added

**Ollama per-model capability probing — #357 (`be9026e4`)**
- `query_model_capabilities` now probes `POST /api/show` per Ollama model and parses the model's own `capabilities` array (`completion`, `tools`, `vision`, `thinking`) as the primary signal for `supports_tools`/`supports_vision`, falling back to the existing family-name heuristics only when the field is absent (older Ollama versions, custom modelfiles) — zero behavior change on that fallback path
- New `supports_parallel_tools` flag on `ModelCapabilities`/`DynamicModelCapabilities`, true only when a model declares tool support **and** its family is known to handle parallel calls well (qwen2.5, qwen3.5, gemma4, glm-4); conservative `false` default everywhere else
- `OllamaResolver` is now actually wired into `complete_openai` and `stream_openai` — it existed before but was never called in the request path, so per-model capabilities now really gate tool injection and `parallel_tool_calls` at runtime instead of unconditionally sending `true`
- 6 new tests covering capabilities-array parsing, embedding models, fallback behavior, and parallel-tools gating; zero behavior change for non-Ollama providers

**TUI chat newline + text selection — #369 (`0fd95371`)**
- Shift+Enter inserts a literal newline in the chat input instead of submitting, so multi-line messages are composable without leaving the TUI
- Mouse-driven text selection and copy now work in the chat pane across terminal emulators, replacing the previous copy-paste-unfriendly behavior

**WebUI build-stamp footer — #367 (`6bdd43ae`)**
- Onboarding wizard footer now shows the build's git SHA and build time (injected via a new `build.rs` at compile time), so a stale WebUI bundle is diagnosable at a glance instead of silently serving old code
- Hardcoded `"STEP {} OF 19"` replaced with a count derived from `STEPS.len()` — the step counter can no longer drift out of sync with the actual step list

**Terms of Service + Privacy Policy published — #370, #371**
- `docs/legal/TERMS_OF_SERVICE.md` and `docs/legal/PRIVACY_POLICY.md` drafted grounded in the shipped code, not boilerplate: self-hosted architecture (users run their own gateway, Zeus Lab operates nothing for them), credentials live only in the user's local `~/.zeus/config.toml`, no telemetry endpoint to Zeus Lab (OTEL/OTLP is user-configured, defaults to `localhost`), Instagram/TikTok scopes act only on the user's own connected accounts with their own tokens, wallet keys are server-side local and devnet-guarded
- Ported to the public `zeuslab.ai` site (`src/legal.rs`, commit `09d2c800`) at `/terms` and `/privacy`, replacing 176 lines of stale boilerplate that had implied a hosted-accounts model contradicting the self-hosted architecture — unblocks the TikTok app audit and Meta App Review, both of which require a public privacy-policy URL

### Fixed

**Full persona SOUL.md rendering — #358, #363**
- `render_soul_md` now detects a full persona body by content shape — `## ` sections or more than 3 non-empty lines — and renders it verbatim under a `# SOUL.md — <name>` header, regardless of what the source starts with; the generic "Core Truths" fallback template only fires for genuinely short/blank input, no longer truncating a real persona down to a boilerplate wrapper (`b125644e`)
- TUI agent screen parses personas through the shared `zeus_core::persona::Persona` parser instead of a hand-rolled frontmatter scraper, so the full `soul_body` — not just name/tagline/tone — flows straight from the persona file into what's rendered (`b125644e`)
- `personalities/leadership/the-coordinator.md` rewritten to match the operator's template verbatim: identity paragraph, "Leading your titans," and "Voice & channel discipline" sections, replacing the abridged version that was shipping before (`b125644e`)
- `soul_pipeline` test suite now seeds its persona library correctly in sandboxed test homes — closes #363, a deterministic red test dating to the #346-era persona-lib seeding gap; suite is green (3/3) for the first time (`b125644e`)
- Practical effect: onboarding and `deploy-identity.sh --with-identity` both now write and re-stamp the *actual* selected persona's soul, not a generic template — deployed rigs pick this up on their next binary update + re-onboard (or identity re-stamp)

### Added

**On-Chain Wallet Stack — #350 (API), #352 (TUI overlay)**
- `GET /v1/wallet/onchain` — address, SOL balance, token balance, cluster (`3dda0700`)
- `GET /v1/wallet/onchain/transactions` — recent tx signatures via `getSignaturesForAddress` (`3dda0700`)
- `POST /v1/wallet/onchain/transfer` — devnet-guarded SPL transfer with `build_transfer_plan` preflight, fee estimate + balance check, insufficient balance returns 402 (`3dda0700`)
- Zero key material in any response — public addresses + tx sigs only; devnet-only hard-fail guard if RPC URL isn't devnet; same auth gate as `/v1/economy/*` (`3dda0700`)
- TUI wallet view overlays live on-chain data alongside the existing off-chain economy balance (`11f54e6a`)

**Native Wallet Screens — #353 (macOS, iOS, Android)**
- Native wallet screens shipped across all three mobile/desktop app repos (separate from `main`): macOS (`33d1b3b`), iOS (`231ff21`), Android (`0accf0e`)
- Same security posture as the API/TUI surfaces — honest 402/403 rendering, no client-side key material
- Completes every on-chain wallet surface except WebUI, tracked as #351

**WebUI Wallet Page — #351 (`c26ef420`)**
- New `POST /v1/wallet/onchain/transfer/preview` endpoint: runs `build_transfer_plan` only, returns the plan (fee estimate, balance-after, ATA existence) without ever signing or submitting a transaction
- WebUI wallet page: two-step transfer flow — Preview (renders the plan) → Confirm (calls the real `/transfer` endpoint) — so nothing is signed before the operator sees the plan
- `402` (insufficient balance) and `403` (non-devnet RPC) rendered honestly in the UI, not swallowed
- Zero key material client-side, same as every other wallet surface
- **Completes the on-chain wallet stack on every surface — API, TUI, macOS, iOS, Android, WebUI — closes #190**

**WebUI Onboarding Parity Phase 1 — #356**
- Added OpenRouter, xAI, Sakana providers to WebUI wizard, matching TUI's 13-provider set (`349eda7a`)
- Reordered wizard from 14 → 20 steps to match TUI flow: instance selector, auth credential summary, fallback model chain, channel config summary, gateway host/port, workspace path, voice provider, image gen provider (`349eda7a`)
- `can_next` validation updated for all 20 steps; step counter, top bar, and launch button text updated to match (`349eda7a`)

**WebUI Onboarding Parity Phase 2 — #356**
- New `GET /v1/onboarding/personalities` endpoint: reads the full persona registry off `~/.zeus/personalities/`, returning `{id, name, tagline, description, category}` for every persona, falling back to a small hardcoded set only if the directory is empty or unreadable (`cf28e2e0`)
- Persona picker step now renders all 25 personas dynamically from that endpoint instead of a hardcoded shortlist — new personas need zero WebUI code changes to appear in the wizard (`cf28e2e0`)
- Skills step now sources the live installed skill catalog via the existing `/v1/onboarding/skills` endpoint instead of a stub list (`cf28e2e0`)
- Model-name field now auto-prefixes by provider on selection, matching TUI behavior, instead of requiring the raw provider-qualified string (`cf28e2e0`)
- 220 lines of orphaned `StepServices` code removed — dead since an earlier step-reorder left it unreachable (`cf28e2e0`)
- Landed via `b3b84e7a` (doc-comment cleanup + handler relocation on top of `cf28e2e0`)

### Fixed

**Soul Pipeline — #338, #346, #347**
- Restored two intentional `SOUL.md` writers that #326 had over-corrected into a single-writer model: onboarding writes the selected persona's SOUL on setup, and `--with-identity` re-stamps it on deploy — both route through `write_onboarding_soul`'s heal-stub/preserve-custom semantics (`63b98242`, #338)
- Onboarding tilde paths (`~/...`) now expand correctly before filesystem writes, fixing SOUL.md landing in the literal `~` directory instead of the user's home (`d42c4f03`, #347)
- Install identity fallback guarded against a bad path when no persona is selected (`057fa700`, #346)

**Installer — #349**
- Sandbox no-launch mode kept fully hermetic — no stray process launches during dry-run/sandbox installs (`e265ac29`)

**LLM Capability Routing**
- xAI Grok capabilities isolated from the shared capability table so Grok-specific limits/features no longer leak into other providers' capability checks (`3e359262`)
- Sakana auth-probe URL normalized — provider-local normalizer guarantees exactly one `/v1` segment; regression test asserts Bearer auth + no `/v1/v1` (`e5eb9671`, #355)

**Instagram + TikTok TUI surfaces — #361**
- Onboarding `ChanConfig` screen gained Instagram (7 fields: `access_token`, `account_id`, `page_id`, `app_id`, `app_secret`, `poll_interval_secs`, `auto_reply` toggle via Space) and TikTok (single access-token field) entries, secrets masked same as existing channels (`341fb65f`)
- Production Channels tab renders live Instagram + TikTok rows; TikTok correctly shown post-only with no fabricated inbound message stats (`341fb65f`)

**Instagram + TikTok WebUI surfaces — #362**
- Onboarding channel step gained the same Instagram (7 fields: `access_token`, `account_id`, `page_id`, `app_id`, `app_secret`, `poll_interval_secs`, `auto_reply`) and TikTok (`access_token` only, post-only) entries as the TUI, secrets masked with the existing pattern (`3bd31a6e`)
- Channels page's add-modal lists both as selectable types; the dynamic `/v1/channels` render handles them with no template-specific code (`3bd31a6e`)
- Additive-only diff (+16 lines) — zero changes to existing channel entries

**Instagram + TikTok: feature-complete across all surfaces**
- With #362 landed, Instagram and TikTok are now live and config-wired end-to-end: backend (#360), TUI (#361), and WebUI (#362) all ship the same field sets and behavior
- Code-complete is not the same as production-usable: TikTok apps post `SELF_ONLY` (private) until the TikTok app audit passes, and Instagram publish/comments/DMs require Meta App Review — both are operator-side platform paperwork, not code gaps

**Missing providers in `/v1/providers` — no ticket #**
- `list_providers()` was returning only 10 of 13 providers, silently dropping OpenRouter, xAI, and Sakana from the onboarding wizard's selection UI (the WebUI onboarding fetches from this endpoint and replaces its hardcoded fallback list)
- All 13 providers now match the WebUI's own hardcoded list

**FreeBSD fresh-install gateway restart deadlock — #365**
- The `$()` command-substitution restart path deadlocked on FreeBSD fresh installs — replaced with the #223 temp-file pattern
- `--update`'s stop-supervisor-first + stray-process-sweep + poll/kill ordering ported to the fresh-install launch path, ending the four-gateway boot races that pattern caused
- A failed `trunk build` is now **fatal** when `--with-webui` was requested, instead of a silent non-fatal warning that let the installer serve a stale WebUI bundle
- Fixed a latent subshell bug that was losing the "gateway was already running" flag across the restart

**Release script publish path — #366**
- `release-local.sh --version` now stamps the workspace `Cargo.toml` version (single source of truth all crates inherit) and syncs `Cargo.lock` via `cargo update --workspace`, so the release tag, artifact names, and `zeus --version` always agree; the stamp is left uncommitted for review, and `--publish` refuses to run until it's committed
- Fixed a fatal top-level `local` in the publish block — every `--publish` run had been dying on it, which explains zero releases shipped to date
- `--targets` is now validated upfront; previously an unknown target silently sailed through to a zero-artifact "success"
- Exit codes are now honest: a run that builds nothing dies, a partial failure exits 1, and `--publish` refuses a partial matrix

---

## [Unreleased] - 2026-07-09 to 2026-07-12

Four-day sprint: full X (Twitter) community management suite, wallet UI wiring, soul restoration, hardening batch (#329–#335), and 13 more messaging channels wired end-to-end.

### Added

**X (Twitter) Community Suite — #336, #339 (42 tools total)**
- **Delete** (#336): `x_delete_post`, `x_batch_delete` (per-item deleted/failed/skipped results, Retry-After backoff), legacy `x_delete` preserved (`7c140d7a`)
- **Read / listening** (#339): `x_search_recent`, `x_get_mentions`, `x_get_tweet`, `x_get_user_timeline` with `since_id` polling (`ccfce94f`)
- **Engagement** (#339): `x_like`/`x_unlike`, `x_retweet`/`x_unretweet`, `x_quote`, `x_follow`/`x_unfollow`, `x_bookmark`/`x_unbookmark`, `x_upload_media` (`224f8bdf`)
- **Moderation** (#339): `x_block`/`x_unblock`, `x_mute`/`x_unmute`, `x_report_tweet`, `x_hide_reply`/`x_unhide_reply` (`47f99635`)
- **Lists** (#339): `x_create_list`, `x_update_list`, `x_delete_list`, `x_add_list_member`, `x_remove_list_member`, `x_get_list`, `x_get_owned_lists`, `x_get_followed_lists`, `x_get_list_memberships`, `x_get_list_tweets`, `x_follow_list`, `x_unfollow_list` (`6e2f7718`)
- **DMs** (#339): `x_send_dm`, `x_send_dm_to_user`, `x_get_dm_events`, `x_get_dm_conversation_events` (`4f4c8013`)
- **Metrics** (#339): `x_account_metrics`, `x_metrics` (`a2c7799f`)
- Core `message` tool now dispatches to `x_twitter`/`x`/`twitter` channel aliases for post/reply
- Reads require bearer token / OAuth2 — clear error surfaced when only OAuth1 credentials are configured

**Wallet UI Wiring — #190**
- Wired wallet send flow: `economy_transfer` + `economy_unstake` API wrappers, Agora page now uses typed API calls (`313779a7`)
- Added `economy_transfer` + `economy_unstake` methods to the TUI API client, updated Send view (`98a3649a`)
- Fixed `economy_unstake` missing `agent_id` param (`96d9cc03`)

**13 More Messaging Channels Wired — #316 P3**
- Teams, WebChat, GoogleChat, Nextcloud (batch 1 — `0774b6c3`)
- Nostr, LINE, Feishu, Zalo (batch 2a — `fe07ce79`)
- BlueBubbles, SMS, Twilio WhatsApp, Voice (batch 2b — `cbad5332`)
- iMessage boot-banner presence, Twitch adapter construction, MQTT dedup fixed (P1 — `e7c6ceb8`)
- Mattermost + MQTT added to onboarding channel grid + config + persistence (P2③ — `ae0ad1d7`)
- `send_file`/`send_rich` channel param docs now reflect real dispatch (P2④ — `021295b3`)

**Soul Self-Rewrite Skill — #327**
- Proposal-only SOUL self-rewrite skill: an agent can draft changes to its own SOUL.md for operator review, never writes unilaterally (`45d2854e`)

**Solana Devnet Plumbing — #306**
- Airdrop, balance queries, `rpc_url` derivation (`9327086f`)

### Fixed

**Soul Restoration — #338**
- Restored two intentional SOUL.md writers (reverting #326's over-correction to a single writer): onboarding writes the selected persona's SOUL on setup, and `--with-identity` re-stamps it on deploy — both routed through the shared `write_onboarding_soul`/#202 semantics (heal stub/missing souls, preserve custom ones unless forced) (`63b98242`, `58f1f7af`)

**Onboarding Arrow-Key Routing — #337 / TUI**
- Fixed arrow keys not routing correctly during instance onboarding (`18aef4c8`)

**Hardening Batch — #329 through #335**
- `#329` runtime-independent OS-thread hard shutdown deadline (`163e7f0e`)
- `#330` re-arm heartbeat interval at head of timed branch — kills a `sleep(0)` busy-loop (`0b7b06e2`)
- `#331` macOS sleep immunity — gateway holds an IOPM `PreventSystemSleep` assertion (`0b452298`)
- `#332` five-cut logging/lifecycle pass: sink reliability (reopen-on-external-delete, WARN+ fsync, stderr outage fallback), adapter lifecycle uniformity across all connection-owning SDKs, boot fingerprint + clock-sanity WARN, named shutdown-drain tasks, per-subsystem `[logging.targets]` level knobs (`8e79de22`…`cea0695c`)
- `#333` `update.sh` Phase 7 — supervised restart or loud failure, never a silent `nohup` (`576223cc`)
- `#334` killed an env-var race class in `zeus-tui` tests (`b31db80f`)
- `#335` probe Ollama before showing "detected" badges in onboarding (`2e1dfb49`)
- `#309` guard config writes against debris preservation; detect post-nuke config debris at boot with a loud warn (no auto-repair) (`7375c20e`, `159d78d4`)
- `#328` forensics: log SIGTERM sender pid/uid via `SA_SIGINFO`; Discord shard-disconnect visibility via `serenity=warn` filter + stage-transition forensics (`1dfe2c1d`, `a95bb2f5`)
- `#322` watchdog service — FreeBSD respawn, systemd crash-loop protection, health-poll watchdog; supervisor pidfile + single-shot watchdog gate bounce fix (`2d6894f2`, `e3394191`)

### Infrastructure
- `#66` local cross-compile release pipeline with `--dry-run` mode (`bc1eb056`)
- Vendored OpenSSL via `zeus-solana` feature flag for release builds

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
