# Zeus Memory Systems — Reconciliation Guide

> **T12 deliverable** — explains the two memory systems, their roles, and how they interact.

## Two Systems, One Purpose

Zeus has two memory crates that are **complementary, not competing**:

| Crate | Type | What it stores | When to use |
|-------|------|----------------|-------------|
| `zeus-memory` | File-based | Markdown files, daily notes, MEMORY.md, SOUL.md, USER.md | Persistent identity, long-term facts, human-readable context |
| `zeus-mnemosyne` | SQLite + vector | Chat messages, semantic embeddings, entity graph, session history | Searchable memory, similarity queries, episodic recall |

Think of it as: `zeus-memory` is the **filing cabinet** (static docs you read at startup), `zeus-mnemosyne` is the **searchable brain** (dynamic DB you query during a session).

---

## zeus-memory (`crates/zeus-memory`)

**Purpose:** File system abstraction over the agent's workspace.

**Key type:** `Workspace`

```rust
let ws = Workspace::from_config(&config);
ws.init().await?;                          // creates AGENTS.md, SOUL.md, etc.
ws.read("MEMORY.md").await?;               // read a workspace file
ws.write("memory/2026-03-30.md", ...).await?; // write daily note
```

**Who uses it:** `zeus-agent`, `zeus-api`, `zeus-mcp`, `zeus-prometheus`, `zeus-ffi`

**Limitation:** No search. Plain file I/O only.

---

## zeus-mnemosyne (`crates/zeus-mnemosyne`)

**Purpose:** Persistent memory with FTS5 full-text search, vector embeddings, entity graph, and temporal versioning.

**Key type:** `Mnemosyne`

```rust
let mem = Mnemosyne::new(config).await?;
mem.store(session_id, &message).await?;        // persist a message
mem.search("deployment strategy", 10).await?; // FTS search
mem.hybrid_search(query, embedding, 10).await?; // vector + FTS
mem.export_memory_summary(50).await?;          // generate MEMORY.md content
```

**Who uses it:** `zeus-agent` (optional), `zeus-nous` (learning engine), `zeus-prometheus` (consolidation)

**Key capability:** `export_memory_summary()` — generates a markdown block that can be written back into `zeus-memory`'s `MEMORY.md`. **This is the bridge between the two systems.**

---

## How They Connect

```
  zeus-mnemosyne                     zeus-memory
  ┌─────────────────┐               ┌──────────────────┐
  │ SQLite DB       │               │ Workspace files  │
  │  - messages     │               │  - MEMORY.md     │
  │  - entities     │  export_      │  - SOUL.md       │
  │  - embeddings   │ ──summary()──▶│  - USER.md       │
  │  - graph        │               │  - daily/*.md    │
  └─────────────────┘               └──────────────────┘
         ▲                                  ▲
         │  store()                         │  read() at startup
         │                                  │
  ┌──────┴──────────────────────────────────┴──────┐
  │                  zeus-agent                     │
  │   (Mnemosyne is optional — logs warning if     │
  │    not configured, cooking loop still runs)    │
  └────────────────────────────────────────────────┘
```

---

## Known Gap: Silent Mnemosyne No-Op

In `crates/zeus-agent/src/lib.rs`, Mnemosyne is stored as `Option<Arc<Mnemosyne>>`. If not configured, memory injection is **silently skipped** at line ~1310:

```rust
// Fetch memory context if Mnemosyne is available
if let Some(mnemosyne) = &self.mnemosyne {
    // inject memory...
} else {
    debug!("Mnemosyne not configured — cooking loop runs without memory injection");
}
```

The `debug!` log is only visible with `RUST_LOG=debug`. Operators running without Mnemosyne won't know they're missing memory injection unless they check logs.

**Recommendation:** Promote this to `warn!` level so it's visible in default log output.

---

## zeus-nous and Memory

`zeus-nous` uses Mnemosyne for **persistent learning** via `LearningEngine::with_mnemosyne()`. Lessons are stored as `MemoryType::Semantic` entries. Without Mnemosyne, `Nous::new()` still works but lessons are in-memory only and lost on restart.

---

## Summary

- **Don't merge the two systems.** Their separation is intentional and correct.
- **Do promote** the Mnemosyne-not-configured log from `debug!` to `warn!`.
- **Do use** `export_memory_summary()` in the prometheus/consolidation loop to keep `MEMORY.md` fresh from the SQLite store.
- The T11 audit found no cross-contamination — both systems are cleanly separated with no circular deps.
