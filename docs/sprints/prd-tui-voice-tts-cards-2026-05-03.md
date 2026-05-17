# PRD — TUI Onboarding: Voice / TTS Backend Selection Cards

**Date:** 2026-05-03
**Author:** Zeus100
**Status:** Draft for merakizzz design pass
**Trigger:** voice message 2026-05-03 — current TUI onboarding only supports ElevenLabs for voice/TTS; merakizzz wants a card chooser mirroring the image-gen pattern with top-3 commercial providers + Custom + Skip

---

## Problem

The TUI onboarding wizard's voice/TTS step currently:

1. **Only supports ElevenLabs.** No card chooser, no alternative provider entry.
2. **Can't capture an API key for any other provider** without hand-editing `config.toml` post-onboarding.
3. **Doesn't surface the local options** (`zeus-tts` already integrates Piper, Kokoro, macOS `say`) — so operators don't know they exist as fallbacks.

Net: every operator gets ElevenLabs by default, can't pick a competitor, and has no path to plug in a self-hosted TTS without manual config.

---

## Goals

1. **Make the provider choice explicit.** Card chooser mirroring the image-gen step's UX (also mirroring the LLM-provider and channel-provider patterns already in onboarding).
2. **Capture credentials at onboarding time.** API key field + any provider-specific config collected directly in the wizard.
3. **Cover the top-3 commercial APIs** (ElevenLabs + 2 others) plus Custom for self-hosted.
4. **Inline credential expansion:** when the user selects a card, the API key field appears immediately below the selection — no separate page navigation.

## Non-goals

- TTS invocation logic (voice cloning, prosody control, batching) — already handled in `zeus-tts` and `zeus-voice` crates.
- STT (speech-to-text) configuration — separate wizard step (Whisper / Groq / others), not in this PRD.
- Local model download / installation — Custom card covers operators who already have a self-hosted endpoint running (e.g., Piper local, Kokoro local, NovaXAI `kk.novaxai.ai`).

---

## User flow

1. User progresses through the LLM-provider + channel-provider + image-gen steps (existing + this sprint's image-gen PRD).
2. User arrives at the new **Voice / TTS Backend** step.
3. Step renders a card grid: 5 cards (3 commercial + Custom + Skip).
4. User navigates with arrow keys, selects with Enter.
5. **Selected card expands inline immediately** to show:
   - **API key field** (always required except Skip and Custom-no-auth)
   - **Optional fields** specific to that provider (default voice ID, model ID, base URL for Custom)
6. User completes the form; on submit, credentials and config persist to `config.toml` `[voice]` table.
7. Skip card sets `[voice] provider = "none"` and dismisses the step.

Web (Leptos) onboarding mirrors the same flow.

---

## Provider card list (final — per merakizzz directive 2026-05-03)

| Card | Provider | Auth | Default voice / model | Notes |
|------|----------|------|------------------------|-------|
| **ElevenLabs** | ElevenLabs | API key | `eleven_multilingual_v2` (model), Rachel (voice) | Current default; premium quality; mentioned explicitly by merakizzz |
| **OpenAI TTS** | OpenAI | API key | `tts-1-hd` (model), `nova` (voice) | Already integrated in `zeus-tts`; well-known; multi-voice support |
| **Cartesia** | Cartesia | API key | `sonic-2` | Best-in-class low-latency for realtime / streaming voice; popular for agent voice work |
| **Custom API** | self-hosted / other | base URL + optional key + voice ID | user-supplied | Covers Piper, Kokoro, macOS `say`, NovaXAI `kk.novaxai.ai`, others |
| **Skip** | — | — | — | Bypass step; no TTS configured |

**5 cards total: 3 commercial + 1 Custom + 1 Skip.** Mirrors the image-gen card count exactly.

**Note on local options:** `zeus-tts` already integrates Piper, Kokoro, and macOS `say`. Operators using those should pick the **Custom** card and point at the local endpoint (e.g., `http://localhost:8104` for the just-deployed Piper). Future iterations may surface dedicated cards for these if usage data justifies it.

---

## Card schema (per-card config fields)

```rust
struct VoiceProviderConfig {
    provider: String,          // canonical key: "elevenlabs", "openai", "cartesia", "custom", "none"
    api_key: Option<String>,   // None for self-hosted-no-auth Custom
    base_url: Option<String>,  // Required for Custom; optional override for others
    voice_id: Option<String>,  // Provider-specific default voice (Rachel, nova, sonic-2-default-voice, etc.)
    model_id: Option<String>,  // Provider's TTS model (eleven_multilingual_v2, tts-1-hd, sonic-2, etc.)
    extra: HashMap<String, String>,  // stability, similarity_boost, speed, format, etc.
}
```

Persistence target in `config.toml`:

```toml
[voice]
provider = "elevenlabs"
api_key = "sk_..."
voice_id = "21m00Tcm4TlvDq8ikWAM"     # Rachel
model_id = "eleven_multilingual_v2"
# base_url = "..."   # only for Custom
# [voice.extra]
# stability = "0.5"
# similarity_boost = "0.75"
```

---

## Functional requirements

1. **F1.** Card grid renders 5 cards with provider name, one-line description, and a placeholder for a logo/icon (graphics merakizzz assigns).
2. **F2.** Keyboard navigation matches the image-gen / LLM-provider / channel-provider steps.
3. **F3.** **Selected card expands inline immediately** to show the credential form. Per merakizzz: "if I choose option A, B, or C, the respective API key entry should be shown right away." No separate confirm-then-form transition.
4. **F4.** API key field non-empty validation (except Custom-no-auth, Skip).
5. **F5.** Custom card requires `base_url`; voice_id field is recommended but may be empty (Custom endpoint defines its own default).
6. **F6.** Optional **"Test connection"** button per card — pings `<base>/voices` (ElevenLabs), `<base>/v1/audio/voices` (OpenAI), `<base>/voices` (Cartesia), or `<base>/health` (Custom). Non-blocking.
7. **F7.** On submit, write to `config.toml` `[voice]` table; preserve credentials in adjacent tables.
8. **F8.** Web (Leptos) onboarding implements the same card list, validation, and inline expansion.

---

## Edge cases

- **API key already in `~/.zeus/credentials.json`**: pre-populate but mark editable.
- **User selects Custom but leaves base_url empty**: validation error.
- **Test connection fails**: surface error inline, allow correction without leaving the step.
- **Re-running onboarding**: pre-select currently-configured provider's card, pre-populate fields.
- **`zeus-voice` crate also has TTS code** (Twilio voice calls + ElevenLabs path): the new wizard's `[voice]` config should drive both `zeus-tts` (text-to-speech rendering) and `zeus-voice` (voice call output) consistently.

---

## Discovered constraints

(Same architectural caveats as the image-gen PRD apply here, slightly adapted.)

### 1. `zeus-tts` is the canonical TTS crate; provider plug-ins exist

Per CLAUDE.md `zeus-tts` (1,941 lines) integrates: OpenAI TTS, macOS `say`, Piper, Kokoro. ElevenLabs lives separately in `zeus-voice`. The new wizard's `[voice]` config should drive the unified provider selection — possibly requiring a small refactor to route ElevenLabs through `zeus-tts` for symmetry.

### 2. Local TTS deployments now exist on the fleet (post-2026-05-02)

Piper TTS just landed on .14 (zeus-spark), .106, .107, and is queued for .112 — all on port 8104, `0.0.0.0`. The Custom card should default-suggest `http://localhost:8104` for operators who deployed local Piper as part of fleet provisioning.

### 3. Provider-aware default voices required

Each commercial provider has different voice ID conventions. The card's "default voice" field needs sensible per-provider defaults so the wizard doesn't ship with a stale or invalid voice ID.

---

## Required pre-work before voice card chooser ships

1. **Audit `[voice]` config consumption** — confirm `zeus-tts` and `zeus-voice` both read from the unified `[voice]` table, or refactor to make them.
2. **Cartesia provider integration** — if not already present in `zeus-tts`, add a thin client wrapper. Cartesia's API is OpenAI-compatible-ish; should be ~50 LOC.
3. **Empirical TTS smoke test per card** before adding it to the chooser — call each provider's "synthesize 'hello world'" endpoint with the default voice; confirm audio output is valid (non-zero file size + correct format). Mirror the image-gen step-count gate pattern.

---

## Implementation handoff

After merakizzz signs off on the PRD + delivers layout/graphics:

- **Code dispatch target:** active fullstack agent (likely zeus106 or zeus107 in the launch sprint).
- **Estimate:** ~1-2 sessions parallel to the image-gen sprint (the patterns are identical — copy the image-gen card chooser scaffolding once it lands and adapt for voice).

---

## Status / next steps

- [x] PRD drafted (this doc)
- [ ] merakizzz design pass + open-questions resolution
- [ ] Cartesia provider integration in `zeus-tts` (if not already present)
- [ ] Card chooser implementation (TUI + Web)
- [ ] Empirical TTS smoke tests per card
- [ ] Ship as part of the 2026-05-04 launch sprint
