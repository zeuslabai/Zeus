# Fugu Ultra — Research & Integration Plan

**Author:** zeus106 (Opus) · **Date:** 2026-06-27 · **Status:** research + plan only, no code changes
**Mandate:** unlock Fugu Ultra's real power + fix how Zeus integrates it. Scope the build from these findings.

---

## 1. What Fugu Ultra really is

**Sakana Fugu (launched 2026-06-22) is not a Japanese LLM — it's an LLM trained to call other LLMs.** It is a *multi-agent-system-as-a-model*: a single OpenAI-compatible endpoint that, under the hood, dynamically assembles, routes, and coordinates a pool of expert agents per task, instead of relying on hand-designed workflows.

- **Two ICLR 2026 papers underpin it:**
  - **TRINITY** — learned assembly/selection of expert agents.
  - **The Conductor** — learned routing/coordination across those experts inside the inference loop.
- **The roles** (the "expert pool" the orchestrator drives): **Thinker / Worker / Verifier** — a plan→execute→check loop run *inside a single API call*.
- **Behavior:** on a query, Fugu either (1) answers directly when it can, or (2) for complex/multi-step problems, spins up and coordinates multiple frontier models internally, reading back its own intermediate outputs to reach answers a single forward pass can't.
- **Why it matters:** adaptive orchestration → SOTA on established benchmarks, plus resilience (no single-model dependency), flexibility (swap frontier models without retraining), and "AI sovereignty" (one stable interface over a churning model landscape).
- **`fugu-ultra`** = the higher-performance tier of the Fugu family (vs. a lighter/cheaper tier).

**Takeaway:** Fugu is a *meta-model*. Its value is the orchestration happening server-side — Zeus should treat it as a high-context, high-latency, high-quality "hard-problem" model, **not** as a drop-in chat model with default knobs.

---

## 2. API surface

**It is OpenAI-compatible by design** — "most SDKs work by just swapping the base URL." That's the headline and the trap: it *looks* like plain OpenAI chat, so a generic OpenAI client mostly works, which is exactly why Zeus's under-configuration goes unnoticed until a long prompt 400s.

Confirmed spec (OpenRouter `sakana/fugu-ultra`):

| Property | Value | Zeus currently has |
|---|---|---|
| **Context window** | **1,000,000 tokens** | **128,000** ❌ |
| **Max output** | **128,000 tokens** | hard-coded **4,096** ❌ |
| Input price | $5 / M tokens | (cost table — verify) |
| Output price | $30 / M tokens | (cost table — verify) |
| API format | OpenAI chat completions | OpenAI ✅ |
| Tools | yes | declared ✅ |

**Beyond plain OpenAI chat — what to expect / probe (Sakana direct API `https://api.sakana.ai/v1`):**
- The orchestration (Thinker/Worker/Verifier pool, agent assembly) is **server-side and largely implicit** — you get the benefit by just calling the model; there is no documented requirement to hand-author the agent graph.
- **Open questions to confirm against Sakana's own API docs (OpenRouter only exposes the OpenAI-compatible subset):**
  1. **Agent-pool opt-out / "direct" mode** — is there a param to force single-pass (skip orchestration) for cheap/fast calls? (Cost + latency control.)
  2. **Coordination / role controls** — any exposed knobs for effort, max internal agents, role weighting, or which frontier models are eligible?
  3. **Reasoning/trace surfacing** — does it return intermediate Thinker/Verifier traces (à la `reasoning_content`) that Zeus would need to capture/replay across turns? (This is a known multi-turn 400 footgun — see §3.)
  4. **Sampling constraints** — does it reject `temperature`/`top_p` like reasoning models do, or accept them?
- **Latency:** orchestration means **higher, more variable latency** than a normal model — streaming + generous timeouts matter.

> Action item: pull Sakana's first-party API reference (not just the OpenRouter OpenAI-compat view) before the build, to lock params #1–#4. OpenRouter and Vercel AI Gateway both proxy the OpenAI-compatible surface; the *fugu-specific* params (if any) live in Sakana's own docs.

---

## 3. Zeus's current gap + the 400 root cause

### How Fugu is wired today
- **Provider:** `Provider::Sakana` exists (`zeus-core/src/lib.rs`), env `SAKANA_API_KEY`, base URL `https://api.sakana.ai/v1` (`zeus-llm/src/lib.rs` ×4), id `sakana`, aliases `"sakana" | "fugu"`.
- **Capabilities** (`zeus-llm/src/capabilities.rs:331`): Sakana is lumped into the generic OpenAI bucket with **`context_window: 128_000`**, `supports_thinking: false`, `skip_temperature: false`.
- **Routing:** generic OpenAI chat path (`complete_openai`-style in `lib.rs`).

### The 400 — two independent, compounding causes

**Cause A — context trimming math is wrong (the "prompt too long" 400).**
`lib.rs:2764` and `:2922` compute the trim target from the capability:
```rust
let max_tokens = caps.context_window;          // 128_000 for Sakana (WRONG — real = 1_000_000)
let target = (max_tokens as f64 * 0.8) as usize; // 102_400
// estimate tokens at len/4; if over target, trim middle history
```
This trimmer is supposed to keep prompts under the model's real limit. Because Sakana is mis-declared at **128k instead of 1M**, the trimmer is calibrated to the wrong ceiling:
- It will **trim history that Fugu could happily accept** (capability loss — the under-utilization Zeus100 called out), **and**
- The estimator is a crude `chars/4` heuristic with no headroom for tool schemas / system-prompt growth, so near the boundary it can both over-trim *and* still mis-fit. On titan/freebsd (larger fleet system prompts + tool sets), the real prompt rides near the *misconfigured* 128k line → intermittent **`400 prompt too long`** when the estimate undershoots actual server-side tokenization. **Fix the ceiling → the trimmer stops fighting a phantom limit.**

**Cause B — output cap + sampling/params (the "invalid arguments" 400).**
`lib.rs:2816–2820` hard-codes the output budget:
```rust
body["max_tokens"] = serde_json::json!(4096);   // Fugu supports up to 128_000 output
```
- Fugu can emit up to **128k** output; pinning **4096** silently truncates long orchestrated answers (capability loss, not a 400 by itself).
- `inject_openai_sampling` (`lib.rs:1218`) sends `temperature: 0.3` for any non-reasoning, non-Moonshot provider — which includes Sakana. **If Fugu (an orchestration model) rejects `temperature`/`top_p` the way reasoning models do, that's a direct `400 invalid arguments`.** This is unconfirmed but high-probability given Fugu's reasoning-like nature, and it matches the "invalid arguments" half of the symptom.
- **Multi-turn trace footgun:** the codebase already documents (XiaomiMimo Bug-4/5) that models which emit server-side reasoning content cause **multi-turn 400s** when Zeus can't capture/replay it. If Fugu returns Thinker/Verifier traces and Zeus drops them, the *next* turn's message sequence can be malformed → 400. Must verify whether Fugu surfaces traces.

**Verdict:** the intermittent 400 is **primarily Cause A** (wrong 128k ceiling driving the trimmer into the wall on big-prompt hosts), with **Cause B** (sampling params and/or dropped traces) as the "invalid arguments" co-factor. Both stem from the single root cause: **Fugu is mis-modeled as a generic 128k OpenAI provider instead of a 1M-context orchestration model with its own param profile.**

---

## 4. Recommendations

### 4a. Proper zeus-llm Fugu provider profile

**Split Sakana out of the shared XAI/Cerebras/DeepSeek/XiaomiMimo bucket** into its own `ProviderCapabilities` arm:

```rust
Provider::Sakana => ProviderCapabilities {
    api_format: ApiFormat::OpenAI,
    auth_methods: &[AuthType::ApiKey],
    supports_tools: true,
    supports_vision: true,            // verify against Sakana docs
    supports_thinking: true,          // orchestration ≈ reasoning; gate sampling accordingly
    supports_streaming: true,
    supports_parallel_tools: true,
    supports_audit_logging: false,
    supports_mid_loop_interrupt: false,
    bot_sender_min_iterations: 5,
    skip_temperature: true,           // PROBABLE — confirm; kills the "invalid arguments" 400
    skip_v1_prefix: false,
    skip_parallel_tool_calls: false,
    context_window: 1_000_000,        // THE fix for the "prompt too long" 400
},
```

Plus, in the request-body path:
- **Output budget:** stop hard-coding `4096` for Sakana — raise to a Fugu-appropriate cap (e.g. configurable, default 16k–32k, ceiling 128k). Don't blanket-128k every call (cost: $30/M output).
- **Sampling:** route Sakana through the reasoning-style branch in `inject_openai_sampling` (skip `temperature`/`top_p`) **if** confirmed rejected; otherwise leave at 0.3. One-line gate.
- **Trace handling:** if Fugu returns reasoning/orchestration traces, add capture/replay (mirror the XiaomiMimo Bug-4 lesson) to prevent multi-turn 400s. If it doesn't, no-op.
- **Cost table:** add Fugu pricing ($5 in / $30 out per M) to `cost.rs` so the fleet's budget accounting is real — Fugu is expensive; it must be visible.
- **Timeouts:** orchestration latency is high/variable — ensure Sakana uses generous request timeouts + streaming, not the default short timeout.

**Sequencing for the build:** (1) bump `context_window` → 1M (kills the dominant 400 immediately, lowest risk); (2) gate sampling + raise output cap; (3) confirm/implement trace replay; (4) add cost entries. Step 1 alone should stop the titan/freebsd bleeding.

### 4b. Leveraging Fugu's multi-agent power across the fleet

Fugu is a **server-side orchestrator** — Zeus already *is* a client-side orchestrator (spawn, sub-Titans, Prometheus). The win is using each where it's strong, not duplicating:

1. **"Hard problem" escalation tier.** Wire Fugu as a **fallback/escalation target** in `fallback.rs` for tasks that stall or fail verification on a normal model — let Fugu's internal Thinker/Worker/Verifier crack the genuinely hard ones, while cheap models handle the routine 95%. (Cost-gated: $30/M output means it's a scalpel, not a default.)
2. **1M-context consolidation jobs.** Use Fugu for whole-repo audits, long-session memory consolidation, multi-doc synthesis — work that benefits from the full 1M window that no other fleet model offers.
3. **Don't nest orchestrators blindly.** Avoid Zeus spawning N sub-Titans that *each* call Fugu (N× orchestration cost + latency). Prefer: Zeus picks the strategy, Fugu does the heavy single-call reasoning, Zeus integrates the result.
4. **Verifier-as-a-service pattern.** Fugu's internal Verifier role makes it a strong **final-check / adjudicator** model — e.g. gate-review of another agent's output before it lands. Natural fit for the coordinator review lane.
5. **Resilience routing.** Because Fugu abstracts over frontier models, it's a good **provider-outage fallback** — if Anthropic/OpenAI are down, a Fugu route keeps the fleet answering.

---

## Open items before the build (confirm, don't assume)
- [x] Pull Sakana's **first-party API docs** (beyond OpenRouter's OpenAI-compat subset) for: agent-pool opt-out, role/coordination params, trace surfacing, sampling acceptance.
- [x] Confirm whether Fugu **rejects `temperature`** (decides Cause-B fix).
- [x] Confirm whether Fugu **returns reasoning/orchestration traces** (decides multi-turn trace replay).
- [x] Verify Fugu **vision** support before flipping `supports_vision`.
- [x] Verify exact **pricing** + add to `cost.rs`.

---

## CONFIRMED FINDINGS (2026-06-27, zeus106 — vs Sakana first-party + published API spec)

Verified against **sakana.ai/fugu** (first-party pricing + FAQ) and the published
`sakana/fugu-ultra` model spec (modality, context, supported params). Console API
reference is login-gated; the model-spec surface is authoritative for the four caps below.

1. **Vision — CONFIRMED SUPPORTED.** ✅ Modality is `text+image->text` (input
   modalities: `text`, `image`). **We shipped `supports_vision: false` (fail-safe) —
   this was wrong; flipped to `true`.** → patch `fix/fugu-vision-confirm`.

2. **Temperature/sampling — skip was CORRECT.** ✅ The published `supported_parameters`
   are `include_reasoning, reasoning, structured_outputs, tool_choice, tools,
   web_search_options` — **no `temperature`/`top_p`**. Confirms `skip_temperature: true`
   was the right call (the Cause-B fix is validated, not just probable). No change.

3. **Reasoning-trace surfacing — CONFIRMED, our model is right.** ✅ Exposes
   `include_reasoning` + `reasoning` params → it *does* surface reasoning. We already
   set `supports_thinking: true`. No caps change needed; capturing/replaying the
   `reasoning` field across turns remains the fast-follow noted in §3.

4. **Pricing — base rate CORRECT, but a long-context tier was MISSED.** ⚠️ First-party:
   base (ctx ≤ 272K) **$5/M in · $30/M out · $0.50/M cache-read**; long-context (> 272K)
   **$10 / $45 / $1.00**. Our `$5/$30` flat matches the base/common case. The **>272K tier
   (2× in, 1.5× out) is a known under-estimate** — documented in `cost.rs`, tracked as a
   fast-follow until `cost.rs` is context-length-aware. Also note a **$0.50/M cache-read**
   discount exists (we fall back to input rate — minor over-estimate, safe direction).

**Bonus findings (fleet-integration relevant):**
- **Agent-pool opt-out does NOT apply to Fugu Ultra** — its pool is **fixed** (full pool
  is what delivers its performance). Opt-out is a plain-**Fugu** console setting only. So
  there's no "direct/cheap mode" param to expose for Ultra.
- **Web search is a first-class billed feature** (`web_search_options` param, $0.01/call) —
  a potential capability to expose later.
- **EU/EEA:** not yet available (GDPR compliance pending) — relevant for any EU-hosted seat.
- **Model cadence:** Sakana retrains/rolls updated Fugu models ~2 weeks after a new
  frontier model ships — expect the underlying pool (and our pinned `fugu-ultra-20260615`)
  to version forward periodically.

**Net patch from this loop-closure:** one real correctness fix (`supports_vision → true`)
+ pricing-tier documentation in `cost.rs`. The other three caps we shipped were confirmed
correct by first-party docs.

## TL;DR
Fugu Ultra = a 1M-context, OpenAI-compatible *meta-model* that orchestrates an expert agent pool (Thinker/Worker/Verifier via TRINITY + Conductor) server-side. Zeus mis-models it as a generic **128k** provider, which (A) starves its context **and** miscalibrates the history trimmer → the intermittent **`400 prompt too long`** on big-prompt hosts (titan/freebsd), while (B) a hard-coded 4096 output cap + a possibly-rejected `temperature` param drive the **`invalid arguments`** half. Fix: give Sakana its **own capability profile** (1M context, gated sampling, real output cap, cost entries, trace handling) and use Fugu as the fleet's **escalation / 1M-context / verifier tier** — a cost-gated scalpel, not a default.
