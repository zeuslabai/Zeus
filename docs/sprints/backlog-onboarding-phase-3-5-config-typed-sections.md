# Sprint — Onboarding Phase 3.5: Config Typed-Section Completion

**Status:** ✅ DAEMON-SIDE LANDED (typed sections in `zeus-core`); ⏳ TUI-side applier wiring deferred (see "Out of scope (deferred)" below).
**Origin:** zeus106's stop-and-ping flag in Phase 3 router wiring (`52431e45`, 2026-05-05).
**Refined scope:** zeus106's audit on `3032d5fa` corrected the original partial-grep diagnosis (see "Original diagnosis vs. reality" below).

## What's actually missing in zeus-core (verified by grep on `3032d5fa`)

The original spec claimed all 5 typed Prometheus subsections + a top-level `[voice]` section were missing. **Audit shows otherwise:**

| Section | Status | Notes |
|---|---|---|
| `[talos]` | ✅ Already typed (`TalosConfig`, lib.rs L1785) | onboarding `apply_features` can write here today |
| `[talos.image]` | ✅ Already typed (`ImageGenConfig`, lib.rs L2287, field at L638) | onboarding `apply_images` can write here today |
| `[mnemosyne]` | ✅ Already typed (`MnemosyneConfig`, lib.rs L1369) | onboarding `apply_memory` can write here today |
| `[prometheus]` (top-level) | ✅ Already typed (`PrometheusConfig`, lib.rs L1703) | … but its 5 subfields were `Option<serde_json::Value>` blobs |
| `[prometheus.scheduler]` | ❌ Untyped subfield → ✅ Typed mirror | new `PrometheusSchedulerConfig` |
| `[prometheus.autonomy]` | ❌ Untyped subfield → ✅ Typed mirror | new `PrometheusAutonomyConfig` |
| `[prometheus.learning]` | ❌ Untyped subfield → ✅ Typed mirror | new `PrometheusLearningConfig` |
| `[prometheus.monitor]` | ❌ Untyped subfield → ✅ Typed mirror | new `PrometheusMonitorConfig` |
| `[prometheus.heartbeat]` | ❌ Untyped subfield → ✅ Typed mirror | new `PrometheusHeartbeatConfig` |
| `[voice]` (top-level) | ❌ Missing — only `DiscordVoiceChannelConfig` exists nested | new `VoiceConfig` (top-level) |

**Real debt = 6 new structs in zeus-core, not 5 missing top-level sections.**

## Original diagnosis vs. reality

The original backlog doc on `.100` (now superseded by this revision) claimed:
> _Sections in `zeus-core/src/config.rs` aren't all defined yet: `[talos]`, `[voice]`, `[talos.image]`, `[prometheus.heartbeat]`, `[mnemosyne]`._

**What the grep actually showed:** 4 of those 5 were already typed. The real gap was **5 untyped subfields inside the existing `PrometheusConfig`** plus the **missing top-level `VoiceConfig`**. zeus106 caught this pre-cut per Pre-cut Discipline #3 ("Verify the model the spec assumes"); Zeus100 acknowledged + greenlit the corrected scope.

## Cross-crate sequencing (why this is daemon-side first)

`zeus-prometheus` depends on `zeus-core`, not the reverse. The canonical engine structs (`HeartbeatConfig`, `SchedulerConfig`, `AutonomyConfig`, `LearningConfig`, `MonitorConfig`) live in `zeus-prometheus` and can't be imported into `zeus-core`. Solution chosen: **typed mirrors** in `zeus-core` that are serde-shape-compatible with their engine twins.

At the boundary, `zeus-prometheus` round-trips:

```rust
let typed_in_core: &PrometheusHeartbeatConfig = …;
let json = serde_json::to_value(typed_in_core).unwrap();
let engine_struct: HeartbeatConfig = serde_json::from_value(json)?;
```

Each mirror has a `#[serde(flatten)] extra: BTreeMap<String, Value>` to tolerate engine-side fields the mirror hasn't caught up to.

## What landed on this branch

1. **`VoiceConfig` struct + field on `Config`** (`pub voice: Option<VoiceConfig>`) — provider/model/enabled + flatten-extra bag.
2. **5 typed `Prometheus*Config` mirror structs** replacing the `Option<serde_json::Value>` blobs in `PrometheusConfig`.
3. **5 zeus-prometheus call-site updates** (`crates/zeus-prometheus/src/lib.rs`) — re-serialize typed mirror → JSON → deserialize into the engine struct. Preserves all existing behavior.
4. **10 new tests** in `phase_3_5_typed_sections_tests`:
   - 6 round-trip tests (one per typed section)
   - 1 extra-field preservation test (`voice_config_extra_fields_preserved`)
   - 1 serde-compat test (`prometheus_heartbeat_serde_compat_with_engine_struct`)
   - 1 backward-compat test (`prometheus_legacy_json_value_still_parses`) — confirms config.toml files written before Phase 3.5 still parse cleanly
   - 1 unknown-field tolerance test (`prometheus_unknown_field_tolerated_via_extra`)

## Verify gate

- `cargo check -p zeus-core -p zeus-prometheus -p zeus-tui` — clean
- `cargo build -p zeus-core -p zeus-tui` — clean
- `cargo test -p zeus-core --lib` — **289 passed, 0 failed** (10 new in `phase_3_5_typed_sections_tests`)
- `cargo test -p zeus-tui --lib` — 252 passed, 2 failed (**both pre-existing on `3032d5fa`**: `app::pantheon_tests::test_office_state_initialized`, `app::tests::test_streaming_then_tool_start_finishes_previous` — confirmed by stashing this branch's changes and reproducing on clean `3032d5fa` HEAD)

## Out of scope (deferred — TUI-side follow-up)

The dispatch's item (c) — _"Wire the existing-but-no-op Phase 3 appliers in `onboarding/persist.rs` to actually write into these typed sections"_ — lives on **`origin/TUI`**, not on `origin/main`. This branch landed the daemon-side prerequisite; the TUI-side wiring is a separate cut after main → TUI sync:

- `apply_voice` → write `OnboardingState::voice` into `Config::voice` (`VoiceConfig`)
- `apply_orchestration` → write `OnboardingState::orchestration` into `Config::prometheus.heartbeat` (`PrometheusHeartbeatConfig`)
- `apply_memory` → already partially wired via `MnemosyneConfig`; verify `embedding_model` field flows
- `apply_features` → write into `TalosConfig` toggles
- `apply_images` → write into `ImageGenConfig`
- Roundtrip tests (state → config → TOML → reload → state-equivalent) per applier

**Estimated 1-2h** once branches are aligned.

## Out of scope (further future)

- Skills step's installation path (installs to `~/.zeus/workspace/skills/` rather than TOML — separate concern)
- Migration logic for existing `~/.zeus/config.toml` files written before these typed sections existed (default-fill on load is sufficient — verified via `prometheus_legacy_json_value_still_parses` test)
- TUI Settings tab editing of these typed sections post-onboarding (Phase 6 territory)

## Branch convention

`feat/onboarding-phase-3-5-config-typed-sections` off `origin/main` HEAD `3032d5fa` (NOT `origin/TUI` — daemon-side).
