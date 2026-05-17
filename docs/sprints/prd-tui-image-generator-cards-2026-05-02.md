# PRD — TUI Onboarding: Image Generator Selection Cards

**Date:** 2026-05-02
**Author:** Zeus100
**Status:** PRD locked — merakizzz signed off 2026-05-02 (voice). Now in design phase.
**Trigger:** voice message 2026-05-02 — current TUI onboarding has a hardcoded default of `gpt-image-1` for image generation with no UI affordance to enter an API key or pick an alternative provider

---

## Problem

The TUI onboarding wizard's image-generator step currently:

1. **Defaults silently to OpenAI `gpt-image-1`** without making the choice explicit
2. **Has no UI surface to enter an API key** for that provider
3. **Offers no alternative providers** — even though several popular image-gen APIs exist with different pricing, latency, and quality tradeoffs
4. **Has no path for self-hosted / custom endpoints** — operators running their own image-gen (e.g., NovaXAI's `zimage.turbo`) cannot wire it through onboarding

Net: every operator gets the same opinionated default, can't authenticate it, and has no way to plug in a different provider without hand-editing `config.toml` post-onboarding.

---

## Goals

1. **Make the provider choice explicit.** First-class card chooser (mirroring the existing LLM-provider step) instead of a hidden hardcoded default.
2. **Capture credentials at onboarding time.** API key field + any provider-specific config (base URL for custom, model ID, region, etc.) collected directly in the wizard.
3. **Cover the major commercial APIs** plus a Custom API card for self-hosted/proprietary endpoints.
4. **Match the existing onboarding UX patterns** — same card grid layout, same keyboard navigation, same persistence behavior as the LLM-provider step.

## Non-goals

- Image generation invocation logic (model wrappers, prompt formatting, output handling) — already exists in `zeus-talos` / `zeus-prometheus`. This PRD is **purely** the onboarding-UI affordance.
- Provider arbitrage / fallback chaining — single provider per workspace at onboarding time. Multi-provider fallback is a separate sprint.
- Offline/local model setup (e.g., bundling Stable Diffusion weights) — out of scope for the wizard. Custom API card covers users who have already set up their own endpoint.

---

## User flow

1. User progresses through the LLM-provider step (existing behavior, unchanged).
2. User arrives at the new **Image Generator** step.
3. Step renders a card grid: 7-9 cards (final list per research below).
4. User navigates with arrow keys, selects with Enter (matches LLM-provider step's keybinds).
5. Selected card expands inline to show:
   - **API key field** (always required except where the provider is keyless or self-hosted-with-no-auth)
   - **Optional fields** specific to that provider (model ID, base URL, region, default size)
6. User completes the form; on submit, credentials and config persist to `config.toml` `[image_generator]` table.
7. **Skip** option: a "Skip — configure later" card that bypasses the step entirely (sets `image_generator = "none"`).

The Web onboarding (Leptos) mirrors the same flow with the same card list.

---

## Provider card list (final — per merakizzz directives 2026-05-03)

Two directives shape the card list:
1. *"top three only. We're not going OpenClaw-wide."*
2. *"Fooocus is ancient and we don't need it. We need GPT Image and others, plus **OpenAI API custom URL** and **Automatic1111 API custom URL** which is what we use for z-image-turbo."*

**Fooocus is dead — drop entirely.** The existing `image_generate` Fooocus client in `zeus-talos` should be **deleted, not adapted**, since merakizzz confirms it was never actually used (Fooocus has no API surface). New chooser starts clean.

**Two distinct "custom URL" cards** — OpenAI-compatible vs Automatic1111. Different protocols, can't share one card.

| Card | Provider | Auth | Model ID | Notes |
|------|----------|------|----------|-------|
| **OpenAI GPT Image** | OpenAI | API key | `gpt-image-1` | Most recognized brand |
| **Google NanoBanana** | Google (Gemini) | API key | `gemini-2.5-flash-image-preview` | Named by merakizzz; strong cost/quality |
| **BFL Flux** | Black Forest Labs | API key | `flux-pro` (default), `flux-dev`, `flux-schnell` | Top-tier developer favorite |
| **OpenAI API (custom URL)** | any OpenAI-compatible endpoint | API key + base URL | user-supplied | Anthropic-style proxies, vLLM gateways, local OpenAI-compat servers, fal.ai (OpenAI mode), etc. |
| **Automatic1111 API (custom URL)** | A1111 / forks (Vladmandic, ComfyUI w/ A1111 plugin) | optional API key + base URL | user-supplied | **The actual path for Z-Image Turbo on DGX `:7860`** (verified by zeus-spark + zeus107 today). Also covers self-hosted SDXL, Flux-on-Comfy via A1111-compat. |
| **Skip** | — | — | — | Bypass step; no image gen configured |

**6 cards total: 3 commercial + 2 custom-URL (OpenAI-compat / A1111) + 1 Skip.**

**Why two custom-URL cards instead of one generic Custom:** the protocols differ. OpenAI-compatible posts to `/v1/images/generations` with `{prompt, model, size}`. A1111 posts to `/sdapi/v1/txt2img` with a different payload shape (steps, sampler, cfg_scale, seed, etc.). Conflating them under "Custom API" forces the user to know which protocol the wizard speaks. Two cards = clear UX.

---

## Card schema (per-card config fields)

Each provider card collects:

```rust
struct ImageProviderConfig {
    provider: String,         // canonical key: "openai", "google", "bfl", "stability", "fal", "replicate", "recraft", "ideogram", "custom"
    api_key: Option<String>,  // None for self-hosted-with-no-auth Custom card
    base_url: Option<String>, // Required for Custom; optional override for others
    model_id: Option<String>, // Defaults to provider's flagship model; can override
    extra: HashMap<String, String>, // Provider-specific knobs (region, default_size, etc.)
}
```

Persistence target in `config.toml`:

```toml
[image_generator]
provider = "openai"
api_key = "sk-..."
model_id = "gpt-image-1"
# base_url = "https://..."  # optional override
# [image_generator.extra]
# default_size = "1024x1024"
```

---

## Functional requirements

1. **F1.** Card grid renders 7-9 cards (final per research) with provider name, one-line description, and a placeholder for a logo/icon (graphics merakizzz will assign post-PRD).
2. **F2.** Keyboard navigation matches the LLM-provider step (arrow keys to navigate, Enter to select, Esc to back).
3. **F3.** Selected card expands inline to a credential form. API key field is `prop:type="password"` masking on Web; toggleable masking on TUI.
4. **F4.** Validation: API key non-empty (except Custom-no-auth), base_url is a valid URL on Custom card.
5. **F5.** Optional **"Test connection"** button on each card — pings the provider's lightweight endpoint (e.g., OpenAI `/v1/models`, Custom card hits `<base_url>/health`). Surface success/failure inline. Non-blocking — user can skip the test.
6. **F6.** On submit, write to `config.toml` `[image_generator]` table; preserve any existing credentials in adjacent tables (`[channels]`, `[mnemosyne]`, etc.).
7. **F7.** Skip card sets `[image_generator] provider = "none"` and dismisses the step.
8. **F8.** Web (Leptos) onboarding implements the same card list and validation.

---

## Edge cases

- **API key already in `~/.zeus/credentials.json`**: pre-populate the field but mark it editable. Don't silently use a stale key.
- **User selects Custom but leaves base_url empty**: validation error, blocks step completion.
- **User picks a meta-platform card (Fal/Replicate)**: model_id field is required — show a default suggestion but force selection.
- **Provider's API responds with auth failure during Test connection**: show the error inline, allow user to correct and retry without leaving the step.
- **Re-running onboarding** (`zeus onboard` over an existing config): pre-select the currently-configured provider's card and pre-populate fields.

---

## Research deliverable (Zeus100 produces before dispatch)

Before any code is written, Zeus100 produces a research doc covering:

1. **Provider verification** — confirm each candidate provider on the table above is currently active, has a public API, and has a stable model ID.
2. **Auth patterns** — exact header format, base URL, any quirks (e.g., Google's per-project auth).
3. **Pricing snapshot** — per-image cost at the cheapest tier, for the README.
4. **Model ID stability** — note any providers that frequently rev model names (operators will need to override).
5. **Final card list** — the 7-9 cards that ship in v1, with a short rationale for any candidates dropped from the starting set.

Saved as `docs/sprints/research-image-gen-providers-2026-05-02.md`. Reviewed by merakizzz before code dispatch.

---

## Implementation handoff (post-PRD approval, post-research)

After merakizzz signs off on the PRD + research:

- **Layout / graphics:** merakizzz designs the card layout and assigns icons/logos per card.
- **Code dispatch target:** active fullstack agent (Zeus112 has heartbeat-rewrite load; zeus106 likely picks this up).
- **Estimate:** ~2-3 sessions — TUI cards (1 session), Web cards (1 session), config persistence + credential storage wiring (1 session).

---

## Decisions (merakizzz, 2026-05-02 voice)

1. **Don't force any choice.** Skip card is allowed; user picks whatever they want. (Closed.)
2. **Card design = same as LLM-provider + channel-provider steps.** Mirror the existing card UI components verbatim — same grid, same keybinds, same expand-on-select pattern, same persistence shape. No new layout invention. (Closed.)
3. Other open questions (test-connection opt-in/default, Custom card empty-key pattern, NovaXAI dedicated card vs Custom, replacement-vs-overlay vs `zeus-talos` tools) — deferred to implementation; engineer judgment, with the constraint that the design must match the LLM/channel card pattern.

## Design phase (merakizzz)

merakizzz takes the layout and graphics from here. PRD is locked. Research deliverable from Zeus100 is **paused** — will produce it if/when the implementation phase needs a finalized provider list. For now, the proposed card list in the table above stands as the working set.

---

## Discovered constraints (fleet shakedown findings, 2026-05-02)

A fleet shakedown of the existing image-gen tools (Zeus100 dispatch, executed by zeus106 + zeus-spark + fbsd2) surfaced three architectural realities that shape the implementation:

### 1. Talos registration is gated on `config.talos.is_some()`

**Finding** (zeus106, empirically verified on .106): the CLI / gateway HTTP / MCP layers all skip registering Talos tools (including `image_generate`, `fooocus_*`) when `[talos]` is missing from `~/.zeus/config.toml`. The gate is presence, not content — an empty `[talos]` block is enough.

**Gate locations:**
- CLI: `src/main.rs:925` (`run_tool` fn)
- Gateway HTTP: `src/gateway.rs:417, 441, 643`
- MCP: `crates/zeus-mcp/src/server.rs:118, 441` (uses separate `[mcp_server] enable_talos` boolean)

**Source code is fine** — all 5 image-gen tools register correctly via `TalosRegistry::with_defaults()` at `crates/zeus-talos/src/lib.rs:180-184`. No feature flag, no `cfg`, no missing export. Just the runtime config gate.

**Empirical proof** (.106):
```
# without [talos]: Error: Not found: Unknown tool: image_generate
# with empty [talos] appended: Error: Tool error: Fooocus request failed (tool registers + invokes)
```

**Confirmed across hosts:** .100, .106, .226 (fbsd2), .14 (zeus-spark) — none have `[talos]` post-onboarding. Universal Mac gap.

### 2. The `[images]` config block is orphan

**Finding:** `[images]` appears in `~/.zeus/config.toml` (written by onboarding) but is **read by no tool currently**. The shipped `image_generate` (Fooocus client) ignores it.

**Implication:** the existing config schema does not actually drive provider selection. Implementing the new onboarding card chooser is also an opportunity to **retire `[images]` in favor of `[talos.image]`** — a sub-block that the Talos-registered tool actually consumes.

### 3. Existing `image_generate` is Fooocus-specific — DELETE

**Finding:** `image_generate` (tool name) → `fooocus::ImageGenerateTool` → calls a local Fooocus server on `:8888`. Not a generic provider router; just a Fooocus client.

**merakizzz directive (2026-05-03):** *"Fooocus stuff is ancient and we don't need it. Fooocus was never used as Fooocus itself doesn't support API image generation."*

**Resolution: DELETE the Fooocus implementation entirely.**

- Remove `crates/zeus-talos/src/fooocus.rs`
- Remove `fooocus_*` tool registrations from `TalosRegistry::with_defaults()`
- Replace `image_generate` with a new generic provider abstraction routing to: OpenAI / Google / BFL / OpenAI-compatible custom URL / Automatic1111 custom URL.
- The new abstraction reads `[talos.image]` config (provider + base_url + key + model) and dispatches to the chosen client.

---

### 4. Per-provider step-count defaults are empirical, not theoretical

**Finding** (zeus107, empirical sweep on .107):

| Steps | CFG | Z-Image Turbo result |
|-------|-----|----------------------|
| 1 | 1.0 | ✅ Valid 2.2 MB image, 91% non-black |
| 2 | 1.0 | ❌ All black, 3.4 KB |
| 4 | 1.5 | ❌ All black |
| 4 | 7.5 | ❌ All black |
| 8 | 1.0 | ❌ All black |

**Z-Image Turbo only produces output at exactly 1 step.** Multi-step inference returns a tiny all-black PNG. This is consistent with SDXL/SD-Turbo and Flux-Schnell being 1-step-distilled models — the inference schedule is incompatible with multi-step sampling.

**Implication:** the card chooser cannot ship with a single global "steps" default. Per-card defaults must be empirically validated:

- **Turbo / Schnell / Lightning models:** `steps = 1`
- **Base SD / SDXL / SD3:** `steps = 20-30` (community standard)
- **Flux Pro / Dev:** `steps = 20-50` (BFL recommendation)
- **OpenAI GPT Image / Google NanoBanana / Recraft / Ideogram:** N/A (closed APIs, model handles internally)

**Validation gate** in implementation: for each card added to the chooser, run a quick smoke generation with the provider's published default settings. If the output is broken (all-black, all-white, etc.), the card's default config is wrong and needs an empirical fix before the card ships.

---

### Required pre-work before image-gen onboarding ships

These constraints generate concrete implementation work BEFORE the new card chooser is meaningful:

1. **Fix the `[talos]` write path on Mac onboarding** — ensure the block lands in `config.toml` post-onboarding. Either fix the writer or remove the gate.
2. **Migrate `[images]` → `[talos.image]`** (or rename/repurpose) so the config schema matches what tools actually consume.
3. **Build a generic provider layer** in `zeus-talos` so `image_generate` can dispatch to OpenAI / Google / BFL / etc. instead of just Fooocus.
4. **Empirical step-count smoke test per card** before adding it to the chooser — Turbo/Schnell variants need `steps=1`, base models need 20-30+. Don't assume.

The card chooser PRD assumes these foundations exist. Without them, the onboarding writes config that nothing reads — and even when wired, picks settings that silently produce broken output.

---

## Status / next steps

- [x] PRD drafted (this doc)
- [ ] merakizzz review + open-questions resolution
- [ ] Zeus100: image-gen provider research deliverable
- [ ] merakizzz: layout + graphics
- [ ] Code dispatch (TUI + Web + config wiring)
- [ ] Ship
