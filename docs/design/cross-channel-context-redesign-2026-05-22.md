# Cross-Channel Context Unification — Design Document

**Author:** zeus-spark  
**Date:** 2026-05-22  
**Status:** PROPOSED  
**Sprint:** #86  
**Reviewer:** Zeus100, merakizzz  

---

## 0. Problem Statement

> *"The same Titan on two channels of Discord doesn't know what's going on on the other channel, unless it stores the file and then reads from it. There should be a central way, central brain, where Titans gather all their information, their memory, and regardless of where you're talking to it, it will know the context of every other channel."*
> — merakizzz, 2026-05-22

A Zeus agent running on `agent:discord:1488620262676238426` and the same agent on `agent:telegram:-100123456` are **epistemically isolated**. They share a process and a binary, but their session contexts, message histories, and injected memories never cross the channel boundary. A conversation on TUI does not carry to Discord. A task kicked off on Telegram is invisible on Slack.

This was first formally documented in `docs/sprints/backlog-prepare-cook-context-extraction-2026-05-06.md` (authored by Z112, 2026-05-06):

> "A conversation with an agent on TUI does not carry over to the same agent on Telegram, Discord, or Slack. Each surface is an island."

This document proposes a concrete architecture to solve this — using infrastructure that already exists in the codebase, extended minimally and surgically.

---

## 1. Current State — Substrate Map

Every claim in this section is verified against source. File:line references are exact.

### 1.1 Channel Session Router

**File:** `crates/zeus-session/src/channel_router.rs`

The `ChannelSessionRouter` derives deterministic per-channel session keys:

```
agent:{channel_type}:{chat_id}
```

Examples:
- `agent:discord:1488620262676238426`
- `agent:slack:C0123456789`
- `agent:telegram:-1001234567890`

This is **correct and intentional** — it prevents cross-contamination of conversation history within a session. The isolation is at the session/message level. The problem is that nothing reads *across* these keys.

### 1.2 Mnemosyne — The Existing Central Brain

**File:** `crates/zeus-mnemosyne/src/lib.rs`

Mnemosyne is a SQLite-backed store with FTS5 full-text search and vector similarity search (embeddings via Ollama or other providers). It already stores messages across all sessions. The `store_with_embedding` method (line 4136) writes every agent response into Mnemosyne:

```rust
pub async fn store_with_embedding(&self, session_id: &str, message: &Message) -> Result<i64>
```

**Crucially:** the signature accepts `session_id: &str` but stores no `channel_kind` or `chat_id` alongside the message. Mnemosyne *has* all the data from all sessions — it just has no channel provenance metadata on those entries. This is the primary gap.

**Write site:** `src/gateway.rs:2299`

```rust
let _ = mnemosyne.store_with_embedding(&session_id, &user_msg).await;
let _ = mnemosyne.store_with_embedding(&session_id, &assistant_msg).await;
```

The `session_id` here is the full `agent:discord:...` key — so channel identity is *embedded in the session_id string* but not as a structured field. Semantic search queries cannot filter by channel or exclude-by-channel without string parsing.

### 1.3 Memory Injector

**File:** `crates/zeus-prometheus/src/memory_injector.rs`

`MemoryInjector` queries Mnemosyne semantically and formats results for system prompt injection. It accepts a query string and returns a formatted block of memories. Currently it performs no channel-awareness — it queries all memories matching the query regardless of which channel they came from. This means:

- It *could* already return cross-channel context.
- But it has no mechanism to label context as "from Discord #devs" vs "from Telegram" vs "from TUI".
- And it doesn't inject a structured "cross-channel context" section — it's a flat memory block.

### 1.4 Channel History Injector

**File:** `src/gateway_consumer.rs:274`

```rust
pub async fn inject_channel_history(
    final_content: String,
    channel_type: &str,
    chat_id: &str,
    ...
    discord_history: &zeus_api::handlers::discord_history::DiscordHistoryStore,
) -> String
```

This function injects the last ~10 minutes of Discord history as a context preamble. Key constraints (verified):
- **Discord-only**: returns early if `channel_type != "discord"` (line ~300)
- **Session-gated**: skips if `session_message_count > 20` — only fires on session cold-start
- **Time-windowed**: fetches messages from last 600 seconds only
- **4000 char cap** on the injected block

This is a narrow bootstrap mechanism, not a cross-channel awareness system.

### 1.5 Fleet Session Resolver

**File:** `crates/zeus-prometheus/src/session_resolver.rs`

`FleetSessionAlias` and `session_resolver` are the Lane 3a/3b stub from the backlog sprint. The resolver signature is locked (Lane 3a, commit `c`), but the body is `unimplemented!()` — Lane 3b was never delivered. It takes `(agent_id, human_id, channel_kind, now)` and is supposed to return a stable cross-channel alias for the same human across channels. Currently always returns `FleetSessionAlias::unaliased(agent_id)`.

**Gateway callsites** (all 5 confirmed in `src/gateway.rs`):
- Line 1162: inbox consumer path
- Line 2098: main channel consumer path  
- Line 2393: internal API path
- Line 2865: autonomous task path

### 1.6 Context Journal

**File:** `crates/zeus-session/src/context_journal.rs`

Captures structured task state (todos, goals, in-progress work) before compaction and re-injects after. Session-scoped only. Not channel-aware.

### 1.7 Context Manager

**File:** `crates/zeus-session/src/context_manager.rs`

Handles session compaction when token limits are approached. Not channel-aware — operates on a single session's message history.

### 1.8 Channel Kind Enum

**File:** `crates/zeus-prometheus/src/channels.rs:49`

`ChannelKind` is a comprehensive enum covering all 25+ supported channels (Discord, Telegram, Slack, TUI, WhatsApp, Signal, Matrix, etc.). It already has `FromStr` and `Display` impls. This is the right type to use for channel metadata.

### 1.9 MemoryStore Schema Gap

**File:** `crates/zeus-mnemosyne/src/lib.rs:4136`

The `store_with_embedding` call stores `session_id` (a string like `agent:discord:CHANNELID`) but the underlying `MemoryStore` schema has no dedicated `channel_kind` or `chat_id` column. Channel identity is recoverable by parsing `session_id` but not queryable as a first-class field.

---

## 2. Architecture Diagram — Current State

```
┌─────────────────────────────────────────────────────────────────┐
│                        Zeus Gateway Process                      │
│                                                                   │
│  ┌──────────────┐   ┌──────────────┐   ┌──────────────────────┐ │
│  │   Discord    │   │   Telegram   │   │        TUI           │ │
│  │  Consumer    │   │   Consumer   │   │      Consumer        │ │
│  └──────┬───────┘   └──────┬───────┘   └──────────┬───────────┘ │
│         │                  │                       │             │
│         ▼                  ▼                       ▼             │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │              ChannelSessionRouter                         │   │
│  │  agent:discord:ID  │  agent:telegram:ID  │  agent:tui:X  │   │
│  └──────────────────────────────────────────────────────────┘   │
│         │                  │                       │             │
│         ▼                  ▼                       ▼             │
│  ┌─────────┐        ┌─────────┐            ┌─────────┐         │
│  │Session A│        │Session B│            │Session C│         │
│  │ (file)  │        │ (file)  │            │ (file)  │         │
│  └────┬────┘        └────┬────┘            └────┬────┘         │
│       │                  │                      │               │
│       └──────────────────┴──────────────────────┘               │
│                          │                                       │
│                          ▼  store_with_embedding(session_id,msg) │
│                   ┌─────────────┐                               │
│                   │  Mnemosyne  │  ← ALL channels write here    │
│                   │  (SQLite +  │  ← BUT: no channel metadata   │
│                   │  FTS5 +     │  ← reads are unfiltered       │
│                   │  vectors)   │  ← no cross-channel injection │
│                   └─────────────┘                               │
│                                                                   │
│  MemoryInjector: queries Mnemosyne → injects into system prompt  │
│  (currently: no channel labels, no cross-channel section)        │
└─────────────────────────────────────────────────────────────────┘

  PROBLEM: Session A cannot see what happened in Session B or C.
  The data IS in Mnemosyne. The wiring to surface it is missing.
```

---

## 3. Architecture Diagram — Proposed State

```
┌─────────────────────────────────────────────────────────────────────┐
│                          Zeus Gateway Process                        │
│                                                                       │
│  ┌──────────────┐   ┌──────────────┐   ┌────────────────────────┐  │
│  │   Discord    │   │   Telegram   │   │          TUI           │  │
│  │  Consumer    │   │   Consumer   │   │        Consumer        │  │
│  └──────┬───────┘   └──────┬───────┘   └──────────┬─────────────┘  │
│         │                  │                       │                 │
│  ┌──────▼──────────────────▼───────────────────────▼──────────────┐ │
│  │                   ChannelSessionRouter                          │ │
│  │  agent:discord:ID  │  agent:telegram:ID  │  agent:tui:local    │ │
│  └──────────────────────────────────────────────────────────────┬─┘ │
│                                                                  │   │
│         ┌────────────────────────────────────────────────────┐  │   │
│         │  [NEW] ChannelContextBridge                        │  │   │
│         │                                                    │  │   │
│         │  On WRITE: tag Mnemosyne entries with:            │  │   │
│         │    channel_kind: "discord" | "telegram" | ...     │  │   │
│         │    chat_id: "1488620262676238426" | ...           │  │   │
│         │                                                    │  │   │
│         │  On READ (per-request):                           │  │   │
│         │    1. Semantic search Mnemosyne (existing)        │  │   │
│         │    2. Filter results by channel_kind != current   │  │   │
│         │    3. Format as "Cross-channel context" block     │  │   │
│         │    4. Inject into system prompt (≤800 tokens)     │  │   │
│         └────────────────────────────────────────────────────┘  │   │
│                                                                  │   │
│                          ▼ write with channel metadata           │   │
│                   ┌─────────────┐                               │   │
│                   │  Mnemosyne  │  ← channel_kind, chat_id      │   │
│                   │  (SQLite +  │     stored as metadata        │   │
│                   │  FTS5 +     │  ← semantic search returns    │   │
│                   │  vectors)   │     labelled, filterable rows │   │
│                   └─────────────┘                               │   │
│                          │                                       │   │
│                          ▼                                       │   │
│         ┌────────────────────────────────────────────────────┐  │   │
│         │  [NEW] AmbientSummarizer (Phase 3, optional)       │  │   │
│         │  Rolls 5-min channel summaries → stores to         │  │   │
│         │  Mnemosyne as high-importance episodic memories    │  │   │
│         └────────────────────────────────────────────────────┘  │   │
└─────────────────────────────────────────────────────────────────────┘

  RESULT: Agent on Discord sees labelled context from Telegram/TUI.
  No new service. No cross-session writes. Mnemosyne is the brain.
```

---

## 4. Design Principles

### 4.1 Mnemosyne IS the central brain — extend it, don't replace it

The codebase already has a semantically-searchable store that receives writes from every channel. The gap is not architecture — it's two missing wires:
1. Channel provenance not stored on write
2. Cross-channel results not injected on read

Fix those two wires. No new service, no new database, no new IPC.

### 4.2 Session isolation is correct — keep it

`ChannelSessionRouter` exists for good reason: conversation history (message ordering, tool call/result pairing, compaction state) must remain per-channel. We are NOT merging sessions. We're adding a cross-channel *awareness layer* on top of isolated sessions.

### 4.3 Prior art anti-patterns to avoid

| System | Pattern | Why it fails |
|---|---|---|
| ChatGPT Memory (early) | Inject full memory verbatim | Token explosion; irrelevant context poisons response |
| Naive global session | Single session for all channels | Tool call/result pairing breaks; compaction races; privacy contamination |
| LangGraph naive cross-thread | Full history from all threads | No relevance filter; O(n·channels) token cost per message |
| Discord history injector (current) | Inject last 10min raw history | Discord-only; time-bounded; not semantic; cold-start only |

### 4.4 The right model: semantic + labelled + token-budgeted

Pull cross-channel context **semantically** (vector similarity to current query), **label it by source channel**, and **cap it at a fixed token budget** (≤800 tokens). This gives the agent peripheral awareness without poisoning the context window.

This is the pattern used by Claude Projects (knowledge base semantic retrieval) and LangGraph's namespace-keyed cross-thread memory — proven at scale.

---

## 5. Detailed Design

### 5.1 Phase 1 — Channel Metadata on Mnemosyne Writes (Sprint 1)

**What:** Add `channel_kind` and `chat_id` fields to Mnemosyne message storage.

**Schema change** (`crates/zeus-mnemosyne/src/db.rs`):

```sql
-- Additive migration — nullable columns, zero breaking change
ALTER TABLE messages ADD COLUMN channel_kind TEXT;
ALTER TABLE messages ADD COLUMN chat_id TEXT;
```

**API change** (`crates/zeus-mnemosyne/src/lib.rs:4136`):

Extend `store_with_embedding` to accept optional channel metadata:

```rust
pub async fn store_with_embedding_with_channel(
    &self,
    session_id: &str,
    message: &Message,
    channel_kind: Option<&str>,   // e.g. "discord", "telegram"
    chat_id: Option<&str>,        // e.g. "1488620262676238426"
) -> Result<i64>
```

Keep the existing `store_with_embedding(session_id, message)` as a shim calling `store_with_embedding_with_channel(..., None, None)` — zero callsite diff for non-channel paths.

**Write site update** (`src/gateway.rs:2299`):

```rust
// Before:
let _ = mnemosyne.store_with_embedding(&session_id, &user_msg).await;
let _ = mnemosyne.store_with_embedding(&session_id, &assistant_msg).await;

// After:
let _ = mnemosyne.store_with_embedding_with_channel(
    &session_id, &user_msg,
    Some(channel_kind.as_str()), Some(chat_id.as_str())
).await;
let _ = mnemosyne.store_with_embedding_with_channel(
    &session_id, &assistant_msg,
    Some(channel_kind.as_str()), Some(chat_id.as_str())
).await;
```

`channel_kind` and `chat_id` are already available at the write site — the gateway message source carries both. This is a 2-line change per callsite.

**Seam points:**
- `crates/zeus-mnemosyne/src/lib.rs:4136` — extend `store_with_embedding`
- `crates/zeus-mnemosyne/src/db.rs` — schema migration (additive ALTER TABLE)
- `src/gateway.rs:2299` — update write call with channel metadata
- `src/gateway.rs:2393` — internal API path (same pattern)

### 5.2 Phase 2 — Cross-Channel Injection on Read (Sprint 2)

**What:** Query Mnemosyne for memories from *other* channels and inject as a labelled block in the system prompt.

**New method on MemoryInjector** (`crates/zeus-prometheus/src/memory_injector.rs`):

```rust
/// Query Mnemosyne for semantically relevant memories from OTHER channels.
/// Returns a formatted "Cross-channel context" block, or None if nothing relevant.
pub async fn inject_cross_channel(
    &self,
    mnemosyne: &Mnemosyne,
    query: &str,
    current_channel_kind: &str,
    current_chat_id: &str,
    token_budget: usize,        // hard cap, default 800
) -> Option<String>
```

Implementation sketch:
1. Run semantic search on Mnemosyne with `query`
2. Filter results to rows where `channel_kind != current_channel_kind OR chat_id != current_chat_id`
3. Sort by relevance score descending
4. Format with source label: `[From telegram, 2026-05-22 09:14]: ...`
5. Truncate to `token_budget` (≈ `token_budget * 4` chars)
6. Return as optional block

**System prompt injection** (`src/gateway.rs` — main channel consumer, ~line 2050):

```rust
// Existing: memory injection block
if let Some(memory_ctx) = memory_injector.inject_memory(&mnemosyne, &query).await {
    system_prompt.push_str(&memory_ctx);
}

// New: cross-channel awareness block
if let Some(cross_ctx) = memory_injector.inject_cross_channel(
    &mnemosyne, &query,
    channel_kind.as_str(), chat_id.as_str(),
    800,
).await {
    system_prompt.push_str("\n\n## Cross-channel context\n");
    system_prompt.push_str(&cross_ctx);
}
```

**Token budget rationale:** A typical system prompt is 2000-4000 tokens. 800 tokens for cross-channel context is ~20% of that — enough for 5-10 meaningful cross-channel memories without crowding the conversation window.

**Seam points:**
- `crates/zeus-prometheus/src/memory_injector.rs` — add `inject_cross_channel` method
- `src/gateway.rs` — inject cross-channel block into system prompt (after existing memory injection)
- Feature-flag: `config.memory.cross_channel_injection: bool` (default `false` until validated)

### 5.3 Phase 3 — Ambient Channel Summaries (Sprint 3, Optional)

**What:** A background task that rolls 5-minute summaries of each active channel and stores them as high-importance episodic memories in Mnemosyne.

**Why:** Raw message storage works for recent/active conversations. But for context older than the semantic search window, or for cross-channel awareness when the exact query doesn't match the topic discussed elsewhere, a pre-summarized "what's happening on each channel" entry provides ambient peripheral awareness.

**Design:**

```rust
// New: crates/zeus-prometheus/src/ambient_summarizer.rs

pub struct AmbientSummarizer {
    summary_interval_secs: u64,   // default 300 (5 min)
    max_summary_tokens: usize,    // default 200 per channel
}

impl AmbientSummarizer {
    /// For each active channel with new messages since last summary,
    /// generate a 1-3 sentence summary and store to Mnemosyne
    /// as a high-importance episodic memory tagged with channel metadata.
    pub async fn tick(&self, mnemosyne: &Mnemosyne, sessions: &SessionManager) -> Result<()>
}
```

This is architecturally independent of Phases 1-2 and can be delivered separately or skipped if the semantic search approach (Phase 2) proves sufficient.

**Seam points:**
- New file: `crates/zeus-prometheus/src/ambient_summarizer.rs`
- `crates/zeus-prometheus/src/lib.rs` — add `AmbientSummarizer` to `Prometheus` struct
- `src/gateway.rs` — spawn ambient summarizer tick in background task loop

---

## 6. Migration Plan

### Sprint 1 (P0 — 3-5 days)

| Step | File | Change | Risk |
|---|---|---|---|
| S1.1 | `crates/zeus-mnemosyne/src/db.rs` | Add `ALTER TABLE messages ADD COLUMN channel_kind TEXT` migration | Low — additive, nullable |
| S1.2 | `crates/zeus-mnemosyne/src/lib.rs:4136` | Add `store_with_embedding_with_channel(...)`, keep old shim | Low — additive, old shim unchanged |
| S1.3 | `src/gateway.rs:2299` | Update write call with `channel_kind`, `chat_id` from message source | Low — values already in scope |
| S1.4 | `src/gateway.rs:2393` | Same update for internal API path | Low |
| S1.5 | Tests | Unit tests for schema migration + channel metadata write/read round-trip | Zero risk |

**Zero breaking changes.** Existing Mnemosyne rows get `NULL` channel metadata — treated as "unknown channel" in read queries.

### Sprint 2 (P0 — 3-5 days)

| Step | File | Change | Risk |
|---|---|---|---|
| S2.1 | `crates/zeus-mnemosyne/src/lib.rs` | Add `search_by_channel_exclusion(query, exclude_channel_kind, exclude_chat_id)` | Low — new query method |
| S2.2 | `crates/zeus-prometheus/src/memory_injector.rs` | Add `inject_cross_channel(...)` method | Low — additive |
| S2.3 | `src/gateway.rs` | Add cross-channel injection call after existing memory injection | Medium — touches system prompt assembly |
| S2.4 | `zeus_core::Config` | Add `memory.cross_channel_injection: bool` feature flag | Low |
| S2.5 | Tests | Integration test: write from channel A, query from channel B, assert cross-channel block present | Medium |

**Feature-flagged** (`cross_channel_injection = false` default). Deploy Phase 1, validate data quality in Mnemosyne for 1-2 days, then enable Phase 2 flag.

### Sprint 3 (P1 — 2-3 days, optional)

| Step | File | Change | Risk |
|---|---|---|---|
| S3.1 | New: `crates/zeus-prometheus/src/ambient_summarizer.rs` | `AmbientSummarizer` struct + `tick()` impl | Low — isolated new file |
| S3.2 | `crates/zeus-prometheus/src/lib.rs` | Add summarizer to `Prometheus` | Low |
| S3.3 | `src/gateway.rs` | Spawn summarizer background task | Low |

---

## 7. What Does NOT Change

To be explicit about scope boundaries:

1. **`ChannelSessionRouter` is unchanged.** Session isolation is correct. We're not merging sessions.
2. **`inject_channel_history` is unchanged.** The Discord cold-start history injector stays as-is. It solves a different problem (recent history on cold-start) and runs before our cross-channel injection.
3. **`session_resolver` / `FleetSessionAlias`** — the Lane 3b stub. This is about human identity correlation across channels, not topic awareness. Orthogonal to this design. Can land independently.
4. **`ContextJournal` is unchanged.** Per-session compaction state continuity is a separate concern.
5. **No new microservice.** Everything runs in the existing gateway process.
6. **No cross-session writes.** Each session still writes its own messages. Cross-channel awareness is read-only at query time.

---

## 8. Prior Art — What We Learned

### ChatGPT Memory (2024-2025)
- **What works:** Curated memory entries extracted from conversations, stored and retrieved semantically. Agent can say "remember this."
- **What fails:** Early implementation injected full verbatim memories without token budgeting → context explosion. Fixed by summarization + budget caps.
- **We adopt:** Budget-capped injection (our 800-token cap), importance scoring (Mnemosyne already has this at `lib.rs:4149`).

### Claude Projects
- **What works:** Shared knowledge base injected into all conversations in a project. Structured separation between project knowledge (stable) and conversation history (ephemeral).
- **What fails:** All-or-nothing injection — entire knowledge base hits every prompt regardless of relevance.
- **We adopt:** Semantic relevance filtering (our `inject_cross_channel` queries by similarity to the current message, not all stored memories).

### LangGraph Cross-Thread Memory (2025)
- **What works:** Namespace-keyed memory store. Cross-thread reads via explicit namespace query. Thread contexts stay isolated; cross-thread data is explicitly fetched.
- **What fails:** No automatic injection — requires the agent to explicitly call a memory tool. High developer friction.
- **We adopt:** The namespace model (our `channel_kind` + `chat_id` metadata). We add automatic injection so agents get it without tool calls.

### Cursor @-mentions / Notion AI
- **What works:** Explicit user-driven context expansion. User pulls in context from other files/docs when relevant.
- **What fails:** Requires user to know what to pull. No ambient awareness.
- **We adopt:** Nothing directly — but it informs our token budget decision. Cursor's context windows are expensive; bounded injection is mandatory.

### Current Zeus `inject_channel_history`
- **What works:** Gives agent 10-min Discord history on cold session start. Reduces "who are you / what were we talking about" confusion.
- **What fails:** Discord-only. Time-bounded (10 min). Not semantic. Fires only on cold start (session_count ≤ 20).
- **We extend:** Our design is semantic (not time-bounded), cross-platform, and fires on every message (not just cold start), but with a hard token budget to avoid the cost issues of the current approach at scale.

---

## 9. Risks and Mitigations

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Cross-channel context is irrelevant / noisy | Medium | Medium | Feature flag (default off); semantic relevance filter filters low-score results |
| Token budget creep — 800 tokens not enough | Low | Low | Configurable via `config.memory.cross_channel_max_tokens` |
| Schema migration fails on existing Mnemosyne DBs | Low | High | Additive nullable columns; migration wrapped in `IF NOT EXISTS` check |
| Privacy: cross-channel injection leaks user A's messages to user B | Low | High | Only applies within same agent identity; DM-to-DM injection disabled (chat_id exclusion); Phase 1 gated behind feature flag |
| Mnemosyne query latency adds to TTFR | Low | Medium | Search is async, budget-capped; existing memory injection already runs this path; no new blocking work |
| `session_resolver` Lane 3b still unimplemented | Confirmed | Low for this design | Orthogonal — this design works without Lane 3b. Lane 3b adds human-identity correlation (same person across channels); our design adds topic-awareness. Both valuable, neither blocks the other. |

---

## 10. Success Criteria

The implementation is complete when:

1. **A message on Discord about topic X** causes a subsequent message on Telegram to include a labelled cross-channel context block referencing the Discord conversation, when the Telegram message is semantically related to topic X.
2. **A task kicked off on TUI** (e.g. "research X and report back") is visible as context when the same agent is addressed on Discord about topic X.
3. **Unrelated messages do not show cross-channel noise** — a message on Discord about topic Y does not inject Telegram memories about topic Z.
4. **Token budget is respected** — cross-channel injection block never exceeds 800 tokens (verified via logging).
5. **No session contamination** — conversation history (tool calls, compaction state) remains per-channel-isolated.

---

## 11. Summary

The central brain already exists. It's Mnemosyne. It already receives writes from every channel. The two missing wires are:

1. **Write path:** Tag every Mnemosyne entry with `channel_kind` and `chat_id` (2-line change at `src/gateway.rs:2299`, schema extension in `crates/zeus-mnemosyne/src/db.rs`)
2. **Read path:** Query cross-channel memories semantically and inject as a labelled, token-budgeted block in the system prompt (new `inject_cross_channel` method in `crates/zeus-prometheus/src/memory_injector.rs`, called from `src/gateway.rs` system prompt assembly)

Three sprints. Feature-flagged throughout. Zero breaking changes. No new services.

The agent on Discord will know what happened on Telegram — not because we merged their sessions, but because they share a memory.
