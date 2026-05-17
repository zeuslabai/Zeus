# Zeus Backlog

Issues logged here are confirmed bugs or improvements not yet assigned to an active sprint.

---

## ✅ SHIPPED

### [BUG] OpenAI: Orphaned tool_calls corrupt session history

**Reported:** 2026-04-12
**Severity:** P0 — every subsequent request fails once triggered
**Reporter:** merakizzz
**Resolved:** 2026-04-27 (audit by zeus106)

**Symptom (historical):** OpenAI returned 400 Bad Request when an assistant
message with `tool_calls` was not followed by matching tool result messages.

**Resolution:** Provider-aware orphan sanitizer shipped in the LLM layer:

- `crates/zeus-llm/src/lib.rs:1195` and `:2877+` — bidirectional sanitizer.
  Strips orphaned `tool_calls` for providers that reject synthetic results
  (Moonshot/Kimi, MiniMax); injects synthetic `tool_result` messages for
  others (OpenAI, Anthropic, etc.).
- `crates/zeus-agent/src/intelligence.rs` — `ContextGuard` calls
  `repair_orphaned_tool_calls` upstream as a second line of defense.

**Relevant commits:**
- `e617054e` fix: sanitizer handles orphaned tool RESULTS + tool_calls bidirectionally
- `1b3d787c` fix(llm): strip orphaned tool_calls for MiniMax like Moonshot/Kimi
- `58ec7293` fix: proper Kimi K2.6 tool_call handling — position-aware sanitizer + strip orphans
- `b6e1604a` fix: GPT-5.5 rejects both temperature AND reasoning_effort with tools

---

## OPEN

_No open backlog items at this time._
