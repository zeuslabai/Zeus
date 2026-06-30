# Cross-Channel Context Unification — Design Proposal
**Dispatch #86 · zeus-spark · 2026-05-22**  
**Status:** PROPOSAL — awaiting Zeus100/merakizzz review  
**Priority:** P0 — fleet-wide architectural gap

---

## 0. Executive Summary

merakizzz's directive: *"Regardless of where you're talking to it, it will know the context of every other channel."*

Today a Titan on Discord channel A has no knowledge of what was said on Discord channel B, Telegram, or the TUI — unless it was written to a file and re-read. This proposal defines a **Central Brain** (Unified Context Store, UCS) that acts as a write-through memory hub: every channel feeds into it, and every channel queries from it at cook time. The architecture is additive — no shipped features break.

---

## 1. Current State Audit

### 1.1 Context Storage Surfaces (all found)

| Surface | Location | Scope | Cross-channel? |
|---|---|---|---|
| Per-channel session files | `zeus-session/src/store.rs` + `ChannelSessionRouter` | One file per `agent:{platform}:{chat_id}` | ❌ isolated |
| Mnemosyne (SQLite + vector) | `crates/zeus-mnemosyne/src/db.rs` | Agent-wide, keyed by `session_id` | ⚠️ partial — stored by session, not globally queried across sessions |
| `inject_channel_history()` | `src/gateway_consumer.rs:274` | Discord-only, last 10 min, ≤15 msgs | ❌ single-channel only |
| Context journal | `zeus-session/src/context_journal.rs` | Per-session journal entries | ❌ no cross-channel read |
| `fleet_session_alias` cache | `zeus-prometheus/src/session_resolver.rs` | `(agent_id, human_id)` → `session_id` (24h TTL) | ⚠️ same-human cross-channel, NOT cross-human |
| MEMORY.md sync | `gateway.rs:2780` | Mnemosyne → workspace file, 30min | ❌ file write only, not queried live |
| Workspace files (goals, MEMORY.md) | `zeus-workspace` | Persisted to disk | ❌ injected only at session-start |

### 1.2 Context Injection Paths (all found)

```
Inbound message
    │
    ├─► inject_channel_history()       [Discord-only, last 10min, ≤15 msgs]
    │       gateway_consumer.rs:274
    │
    ├─► Goals context injection        [session start only]  
    │       gateway.rs:605
    │
    ├─► MemoryInjector                 [queries Mnemosyne via system prompt]
    │       zeus-prometheus/src/memory_injector.rs
    │       ↳ searches by current query, returns top-N semantically relevant
    │
    └─► Manual file reads              [agent tool calls only]
            workspace/memory/MEMORY.md, etc.
```

### 1.3 The Gap — Precise Definition

The `MemoryInjector` already searches Mnemosyne, but Mnemosyne stores are **session-keyed writes**: `store_with_embedding(&session_id, ...)`. When agent is on Discord channel A (`session_id = agent:discord:111`), it stores memories under that session. When queried from Discord channel B (`session_id = agent:discord:222`), the semantic search runs across ALL stored memories (session-ID is metadata, not a filter) — **but this is only true if the Mnemosyne DB is shared**.

The real gaps are:
1. **`inject_channel_history()` is Discord-only and channel-scoped** — no cross-channel recent context
2. **No active cross-channel "what's happening elsewhere" feed** — agent has no ambient awareness of other channels
3. **`fleet_session_alias` only correlates same-human across channels** — doesn't solve cross-channel topic awareness
4. **Mnemosyne stores per-session, but isn't queried with channel context** — missing "what was said on channel X about topic Y?"

---

## 2. Prior Art Survey

### 2.1 ChatGPT Memory
- **What works:** Global semantic memory store, always-on background ingestion, surfaces relevant memories in every conversation regardless of how it was created
- **What fails:** Hallucination of false memories, no user control over what's remembered, cross-contamination ("my friend's info bleeds into my cooking recommendations")
- **Key pattern:** Explicit `memory_store` tool call by the model → curated, not verbatim transcript

### 2.2 Claude Projects
- **What works:** Shared "Project Knowledge" injected into every conversation in that project — deterministic, user-controlled
- **What fails:** Siloed per-project, no cross-project awareness, manual curation required
- **Key pattern:** Structured knowledge files + per-project system prompt injection

### 2.3 LangGraph Cross-Thread Checkpoints
- **What works:** `store` namespace concept — threads write to named namespaces, other threads read by namespace key; cross-thread memory via `MemorySaver` with shared `namespace`
- **What fails:** No automatic semantic surfacing — you must know what namespace to query; high latency if namespace is large
- **Key pattern:** `(namespace, key) → value` store, namespaced by `(user_id,)` not `(thread_id,)`

### 2.4 Slack Canvas / Notion AI
- **What works:** Structured shared document as ambient context — all agents/channels that can see the canvas see the same facts
- **What fails:** Manual maintenance, becomes stale, not semantic
- **Key pattern:** Single-writer, multi-reader structured document

### 2.5 Anti-Patterns to Avoid
| Anti-pattern | Why it fails |
|---|---|
| Inject full cross-channel history verbatim | Token explosion, context window pollution |
| Single global session | Cross-contamination — unrelated channel topics bleed |
| File-based sync only (current MEMORY.md) | 30min lag, stale on fast-moving conversations |
| Channel-keyed memory writes, channel-keyed reads | Siloed — the exact current problem |
| No TTL on cross-channel context | Stale context from months ago pollutes fresh conversations |

---

## 3. Central Brain Architecture

### 3.1 Core Concept

```
┌─────────────────────────────────────────────────────────────────┐
│                    UNIFIED CONTEXT STORE (UCS)                   │
│                  (extends existing Mnemosyne DB)                 │
│                                                                  │
│  ┌─────────────────┐    ┌──────────────────┐    ┌────────────┐  │
│  │  Channel Feed   │    │  Cross-Channel   │    │  Ambient   │  │
│  │  (write path)   │    │  Index (query)   │    │  Summary   │  │
│  │                 │    │                  │    │  (rolling) │  │
│  │ session_id      │    │ channel: Discord │    │            │  │
│  │ channel_kind    │    │ channel: Slack   │    │ per-agent  │  │
│  │ chat_id         │    │ channel: TUI     │    │ ~500 tokens│  │
│  │ human_id        │    │ channel: Telegram│    │            │  │
│  │ content         │    │                  │    │ refreshed  │  │
│  │ importance      │    │ query: semantic  │    │ every 5min │  │
│  └─────────────────┘    │ filter: recency  │    │            │  │
│                         │ filter: excl.    │    │            │  │
│                         │  current channel │    └────────────┘  │
│                         └──────────────────┘                    │
└─────────────────────────────────────────────────────────────────┘
         ▲ feed-in                          │ query-out
         │                                  ▼
┌────────────────┐              ┌─────────────────────────────────┐
│  All Channels  │              │         MemoryInjector           │
│                │              │                                  │
│  Discord #dev  │              │  current: session-scoped search  │
│  Discord #gen  │              │  + NEW: cross-channel query     │
│  Slack #eng    │              │  (excluding current channel)    │
│  Telegram DM   │              │  formatted as:                  │
│  TUI           │              │  "## Cross-channel context\n    │
│  WebChat       │              │   [Discord #dev, 3h ago]: ..."  │
└────────────────┘              └─────────────────────────────────┘
```

### 3.2 Data Model (Mnemosyne extension)

Existing `store_with_embedding(session_id, content)` → extend to:

```rust
pub struct CrossChannelEntry {
    pub session_id: String,       // existing: agent:discord:111
    pub channel_kind: String,     // "discord" | "slack" | "telegram" | "tui"
    pub chat_id: String,          // channel/chat identifier
    pub human_id: Option<String>, // sender, if known
    pub content: String,          // message or summary
    pub importance: f32,          // 0.0–1.0, from ImportanceScorer
    pub stored_at: i64,           // unix timestamp
    pub ttl_hours: u32,           // default 72h, P0 events: 168h
}
```

This is additive — existing Mnemosyne schema gains 4 new nullable columns. Zero migration required for existing rows (they default to `channel_kind = "unknown"`).

### 3.3 Write Path (Feed-In)

**Where:** `gateway.rs:2299` — the two existing `store_with_embedding` calls

**Change:** After storing, also write `channel_kind` + `chat_id` metadata via extended `store_with_channel_metadata()`:

```
Inbound message on Discord #dev
    │
    ├─ store_with_embedding(session_id="agent:discord:111", content)  [existing]
    └─ tag_with_channel_metadata(channel_kind="discord", chat_id="111")  [NEW — same row]
```

Cost: zero extra DB writes — just additional columns on the existing insert.

### 3.4 Read Path (Query-Out)

**Where:** `zeus-prometheus/src/memory_injector.rs` — `inject()` method

**New method:** `inject_cross_channel()` — runs alongside existing memory search, excludes current channel, applies recency + importance filters:

```rust
pub async fn inject_cross_channel(
    &self,
    mnemosyne: &MnemosyneDb,
    query: &str,
    current_channel_kind: &str,
    current_chat_id: &str,
    max_age_hours: u32,     // default 24h
) -> Option<String>
```

Returns formatted string injected into system prompt as a distinct section:

```
## Cross-channel context (last 24h)
[Discord #general, 2h ago]: User asked about deployment pipeline, agent explained blue-green strategy
[TUI, 45min ago]: Agent drafted a Rust PR for feat/68
[Slack #devs, 8h ago]: merakizzz mentioned budget approval for GPU cluster
```

**Token budget:** Hard cap 800 tokens for cross-channel section (configurable). Uses summarization (existing `summarizer.rs`) if raw entries exceed budget.

### 3.5 Ambient Channel Summary (Rolling)

**Problem:** Semantic search only surfaces what's semantically similar to the *current query*. If user asks "how's deployment going?" it won't surface a Slack conversation about team mood.

**Solution:** Ambient summary — a 500-token rolling summary of "what happened on each channel in the last N hours" — regenerated every 5 minutes by a lightweight background task.

```
Stored as: workspace/memory/channel-summaries/<channel_kind>-<chat_id>-summary.md
Injected as: low-priority context at the bottom of system prompt
TTL: 5min (stale = omit)
```

This is the "peripheral vision" layer — the agent knows roughly what's happening everywhere, even if it doesn't remember every detail.

---

## 4. Architecture Diagram — Component Relationships

```
                    ┌──────────────┐
                    │   gateway.rs  │
                    │  (orchestrator)│
                    └──────┬───────┘
                           │
          ┌────────────────┼────────────────┐
          │                │                │
    ┌─────▼──────┐  ┌──────▼──────┐  ┌─────▼──────┐
    │ Discord    │  │  Telegram   │  │    TUI     │
    │ Consumer   │  │  Consumer   │  │  Consumer  │
    └─────┬──────┘  └──────┬──────┘  └─────┬──────┘
          │                │                │
          └────────────────▼────────────────┘
                           │ write: store_with_channel_metadata()
                           ▼
                  ┌────────────────┐
                  │   Mnemosyne    │ ← CENTRAL BRAIN
                  │  (SQLite+vec)  │
                  │                │
                  │  per-session   │
                  │  + channel_kind│
                  │  + chat_id     │
                  │  + importance  │
                  └───────┬────────┘
                          │ read: inject_cross_channel()
                          ▼
                  ┌────────────────┐
                  │ MemoryInjector │
                  │  (prometheus)  │
                  │                │
                  │  existing:     │
                  │  session-scope │
                  │  + NEW:        │
                  │  cross-channel │
                  └───────┬────────┘
                          │ formatted context block
                          ▼
                  ┌────────────────┐
                  │  System Prompt │
                  │  ## Relevant   │
                  │     Memory     │
                  │  ## Cross-     │
                  │  channel ctx   │ ← NEW SECTION
                  └────────────────┘
```

---

## 5. Migration Plan

### Phase 0: Schema extension (no behavior change) — 1 sprint
- Add `channel_kind`, `chat_id`, `human_id`, `ttl_hours` columns to Mnemosyne entries (nullable, no existing row migration)
- Add `store_with_channel_metadata()` wrapper that populates new columns
- Wire existing `store_with_embedding` calls in `gateway.rs:2299` to also pass channel metadata
- **Risk:** Zero — additive columns, existing queries unaffected

### Phase 1: Cross-channel query — 1 sprint
- Add `query_cross_channel()` to `MnemosyneDb`
- Add `inject_cross_channel()` to `MemoryInjector`
- Wire into `Prometheus::cook()` path alongside existing memory injection
- Feature-flagged off by default (`config.toml: [memory] cross_channel_context = false`)
- **Risk:** Low — new code path, feature-gated

### Phase 2: Enable + tune — 1 sprint
- Enable flag for fleet (after Phase 1 passes testing)
- Tune: max_age_hours=24, token_budget=800, min_importance=0.3
- Monitor: track cross-channel injection frequency, token overhead, agent response quality
- **Risk:** Medium — affects live agent context; monitor for context pollution

### Phase 3: Ambient channel summaries — 1 sprint (optional, P1)
- Background task: summarize each active channel every 5min → `workspace/memory/channel-summaries/`
- Inject summaries as low-priority context block
- **Risk:** Low — additive, easily disabled

### What does NOT change
- Per-channel session isolation (`ChannelSessionRouter`) — sessions stay separate
- `fleet_session_alias` logic — same-human cross-channel correlation unchanged
- `inject_channel_history()` — keeps its current narrow Discord scope
- Any existing Mnemosyne query behavior — existing semantic search is unchanged

---

## 6. Open Questions for Zeus100/merakizzz

1. **Scope:** Cross-channel within one agent (same Titan, different channels) first, or also cross-agent (different Titans sharing context)?
2. **Privacy:** Should human messages from private DMs be included in cross-channel context? Or only public channel messages?
3. **TTL:** 24h default for cross-channel look-back? Or agent-specific config?
4. **Token budget:** 800 tokens for cross-channel section — too much? Too little?
5. **Mnemosyne shared vs per-agent:** Is the Mnemosyne DB currently shared across all channels of one agent, or one DB per session? (This determines if Phase 0 even needs schema work, or if it's already implicitly cross-channel and just needs the query filter)

---

## 7. Implementation Owners (suggested)

| Phase | Owner | Branch |
|---|---|---|
| 0: Schema extension | zeus-spark | `feat/87-ucs-schema-extension` |
| 1: Cross-channel query | zeus-spark or fbsd2 | `feat/88-cross-channel-query` |
| 2: Enable + tune | Zeus100 | — |
| 3: Ambient summaries | ZeusMarketing or zeus-spark | `feat/89-ambient-summaries` |

---

## 8. TL;DR for merakizzz

**What we build:** Tag every message with its channel when storing to Mnemosyne (the existing brain). When answering on any channel, query for relevant memories from OTHER channels and inject them as a new "Cross-channel context" block in the system prompt.

**Result:** The Titan on Discord #general knows about the conversation that happened on Slack 2 hours ago. No separate "central brain" service needed — Mnemosyne IS the central brain; we just need to make it channel-aware.

**Time:** 3 focused sprints (Phases 0-2). Phase 3 is optional polish.

**Risk:** Low. Additive, feature-flagged, no breaking changes.
