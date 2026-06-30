# Unified Session Architecture — #91
**Date:** 2026-05-23  
**Author:** zeus-spark  
**Branch:** `docs/91-unified-session-architecture-zeus-spark`  
**Status:** Design — awaiting 3-seat ratify

---

## 1. Verified Substrate Map

All citations pinned via `git show origin/main:<path>` (HEAD `f82a9cef`).

### 1.1 Session ID Derivation — `channel_router.rs`

```
crates/zeus-session/src/channel_router.rs

L61:  pub struct ChannelKey { channel_type: String, chat_id: String, user_id: Option<String> }
L106: pub fn derive_session_id(key: &ChannelKey) -> String
L108:   Some(uid) => format!("agent:{}:{}:{}", key.channel_type, key.chat_id, uid)  // DM 4-part
L109:   None      => format!("agent:{}:{}", key.channel_type, key.chat_id)           // group 3-part

Examples (from module doc):
  agent:discord:1488620262676238426     — Discord channel
  agent:slack:C0123456789               — Slack channel
  agent:telegram:-1001234567890         — Telegram group
  agent:telegram:dm-123:456             — Telegram DM (4-part, user_id suffix)
```

**Key finding:** Every `(channel_type, chat_id)` pair gets its own session file today. No cross-channel shared session exists. The `user_id` suffix (DM variant, L108) further isolates per-user within a chat.

---

### 1.2 Session Store — `store.rs`

```
crates/zeus-session/src/store.rs

L57:  pub struct SessionStore { sessions_dir: PathBuf, locks: RwLock<HashMap<...>> }
L79:  pub async fn acquire(&self, session_id: &str) -> SessionGuard   // per-session Mutex
L106: pub async fn load(&self, id: &str) -> Result<Session>           // JSONL load from disk
L113: pub async fn create(&self) -> Result<Session>                   // JSONL init on disk
L123: pub async fn list(&self) -> Result<Vec<(String, DateTime<Utc>)>>
L150: pub async fn get_or_create_labeled(&self, label: &str) -> Result<Session>
```

**Key finding:** Sessions are JSONL files on disk keyed by an arbitrary `session_id` string. The store is key-scheme-agnostic — any string works. Per-session `Mutex` at L79 serializes concurrent writes to the same session. Two sessions sharing a merged key serialize correctly via this existing mechanism.

---

### 1.3 Gateway Cook Entry — `gateway.rs`

```
src/gateway.rs

L182:  use zeus_session::{ChannelKey, ChannelSessionRouter, Session};
L295:  // "Per-channel traffic gets deterministic session IDs via ChannelSessionRouter"
L300:  let channel_session_router = Arc::new(ChannelSessionRouter::new());
L1768: let key = ChannelKey::new(msg.source.channel_type(), chat_id);
L1769: let session_id = channel_session_router_for_rx.resolve(&key).await;
       // → Session::resume_or_create(&sessions_dir, &session_id) swapped onto dispatch_agent
L2163: prom_guard.cook_with_history_interruptible(...)   // channel cook entry
L2323: if let Some(mnemosyne) = guard.mnemosyne() {      // #86 Mnemosyne injection
L2328:   mnemosyne.store_with_embedding_tagged(&session_id, &user_msg, ck, cid)
L2329:   mnemosyne.store_with_embedding_tagged(&session_id, &assistant_msg, ck, cid)
L2451: prom_guard.cook_with_history(...)                  // API cook entry
L2923: prom_guard.cook_with_history(...)                  // autonomous_loop entry
```

**Key finding:** Session swap happens at L1768-1769. The `session_id` string is the ONLY coupling between channel identity and session file. Changing the derivation scheme changes which file is loaded — the cook chain itself is agnostic. L2328-2329 shows the `#86` Mnemosyne injection pattern — the unification seam for #91 mirrors this exactly.

---

### 1.4 PrometheusEngine Cook Chain — `zeus-prometheus/src/lib.rs`

```
crates/zeus-prometheus/src/lib.rs

L1473: pub async fn cook(&self, message, tools)                          // no-history entry
         └→ delegates to cook_with_history(message, tools, &[])
L1508: pub async fn cook_with_history(&self, message, tools, history)
         └→ L1514: delegates to cook_with_history_cancellable(... None)
L1520: pub async fn cook_with_history_cancellable(... cancel)
         └→ L1527: delegates to cook_with_history_interruptible(... cancel, None, vec![])
L1535: pub async fn cook_with_history_interruptible(
           &self, message: &str, tools: &[ToolSchema],
           conversation_history: &[Message],   // ← injection point
           cancel, interrupt_rx, attachments)  // PRIMARY cook loop
```

**Key finding:** All cook paths funnel to `cook_with_history_interruptible` at L1535. It accepts `conversation_history: &[Message]` as a plain slice. PrometheusEngine does NOT resolve session IDs — it receives pre-built history from the gateway. Injecting cross-channel tail means augmenting this slice at the gateway before calling L1535. No signature change to PrometheusEngine needed.

---

### 1.5 Channel-Tag Flow — `zeus-channels` adapters

```
crates/zeus-channels/src/discord.rs
  L1753+: account_id / role_ids config fields
  chat_id extracted from DiscordMessage.channel_id (per-message)

crates/zeus-channels/src/telegram.rs
  L260:  chat_id from teloxide Update (chat.id())
  L295:  user_id from sender (feeds ChannelKey.user_id for DM 4-part key)

crates/zeus-channels/src/slack.rs
  L2065: debouncer key = channel_type + account + chat_id + user_id
```

**Key finding:** Channel adapters produce `ChannelSource` structs. They have zero session-awareness — they emit events. The gateway constructs `ChannelKey` from `ChannelSource` at L1768 and routes to sessions. All session logic is centralized at the gateway boundary. This must stay that way per `universal-features-route-through-gateway`.

---

## 2. Problem Statement

**Today:** Each `(channel_type, chat_id)` derives its own session file. A Discord message builds history in `agent:discord:1488620262676238426`. A Telegram message builds history in `agent:telegram:-100987654321`. The agent has no conversational continuity across channels.

**#86 partial fix:** Cross-channel MEMORY injection (Mnemosyne) shares *semantic memory* across channels. The agent recalls facts via vector search. But session *dialogue* — the literal conversation thread — remains per-channel.

**Operator goal:** "Single session from every channel. Agent on Discord needs knowledge of Telegram, vice versa. Linear code + more efficient."

---

## 3. Design Options

### Option A — Single Agent-Wide Session

**Concept:** All channels collapse to one session file. `derive_session_id` returns a fixed key (`agent:unified:main`) regardless of channel.

**Mechanism:**
```
ChannelKey { discord, 1234 }  →  "agent:unified:main"
ChannelKey { telegram, 5678 } →  "agent:unified:main"
ChannelKey { slack, C999 }    →  "agent:unified:main"
```

**Tradeoffs:**

| Dimension | Result |
|---|---|
| Thread isolation | ❌ None. Per-session Mutex serializes writes but concurrent cooks from two channels interleave mid-cook. |
| Context window | ❌ Unbounded. Full history from all channels loads every cook. Multi-channel deployments hit context limits fast. |
| Chronological coherence | ✅ Natural wall-clock interleave. |
| Storage | ✅ One file. |
| Implementation | ✅ ~5 LOC change. |
| DM privacy | ❌ Broken. User A's DMs visible in user B's channel session. |
| Regression risk | ⚠️ High — all existing multi-channel deployments break. |
| Cross-channel continuity | ✅ Perfect. |

**Verdict:** Correct for single-user, single-agent, low-traffic. Unsafe for multi-user/multi-channel. Not the Cut 1 path.

---

### Option B — Per-Channel Sessions + Cross-Channel Tail Injection

**Concept:** Keep per-channel JSONL files (today). At cook time, inject a configurable tail of recent messages from other active channels into `conversation_history` before calling `cook_with_history_interruptible`.

**Mechanism:**
```rust
// gateway.rs ~L1768 (after session swap):
let primary_history = current_session.messages.clone();
let mut injected_history = Vec::new();

for other_key in channel_session_router.active_keys().await {
    if other_key == current_key { continue; }
    if other_key.is_dm() { continue; }   // DM privacy gate
    let other_session_id = channel_session_router.resolve(&other_key).await;
    let tail = session_store.tail_snapshot(&other_session_id, tail_n).await;
    injected_history.extend(wrap_as_cross_channel_context(tail, &other_key));
}
injected_history.extend(primary_history);

prom_guard.cook_with_history_interruptible(msg, tools, &injected_history, ...)
```

**New methods needed:**
- `SessionStore::tail_snapshot(id: &str, n: usize) -> Vec<Message>` — last N messages from JSONL
- `ChannelSessionRouter::active_keys() -> Vec<ChannelKey>` — already tracked in cache (L~85)

**Tradeoffs:**

| Dimension | Result |
|---|---|
| Thread isolation | ✅ Full — each channel writes its own JSONL. |
| Context window | ✅ Tunable. N=10, 3 channels → ~30 extra messages (~3K tokens). Bounded. |
| Chronological coherence | ⚠️ Tail is prepended as a block, not true interleave. Agent sees "recent Telegram context" then "this channel's history." |
| Storage | ✅ Same as today. |
| Implementation | ✅ ~150-200 LOC. |
| DM privacy | ✅ Preserved — DM sessions (4-part key) excluded by default. |
| Regression risk | ✅ Low — additive. Feature-flagged off at N=0. |
| Cross-channel continuity | ✅ Good — agent sees recent activity from all channels before each cook. |

**Verdict:** Safest migration path. Incremental, reversible, zero behavior change until operator opts in. Mirrors `#86` shape exactly.

---

### Option C — Hybrid: Per-Thread + Global Distilled Session

**Concept:** Two-tier topology. Per-channel sessions (Option B) remain. Additionally, a *global session* (`agent:global:main`) receives a distilled summary after each cook. At cook entry, global tail is injected first, then per-channel primary history.

**Mechanism:**
```
On cook completion:
  summary = distill_to_1_3_lines(cook_result)
  global_session.append(Message { role: "system", content: summary })

At cook entry:
  global_tail = global_session.messages.last(20)
  history = [global_tail..., primary_channel_messages...]
  cook_with_history_interruptible(msg, tools, history)
```

**Tradeoffs:**

| Dimension | Result |
|---|---|
| Thread isolation | ✅ Full. Global session uses its own Mutex. |
| Context window | ✅ Best — summaries are compact. Predictable size regardless of channel count. |
| Chronological coherence | ✅ Best — global session is true chronological cross-channel log (distilled). |
| Storage | ⚠️ Two write paths per cook. Still cheap. |
| Implementation | ❌ ~400+ LOC. Requires summarization prompt + post-cook write path. |
| Regression risk | ✅ Low — additive. |
| Cross-channel continuity | ✅ Excellent. |

**Verdict:** Best long-term architecture. Too complex for Cut 1. Natural Cut 3 target after Option B proves out.

---

## 4. Option Comparison

| | Option A | Option B | Option C |
|---|---|---|---|
| Cut 1 LOC | ~5 | ~150-200 | ~400+ |
| Thread safety | ⚠️ Risky | ✅ Safe | ✅ Safe |
| Context growth | ❌ Unbounded | ✅ Tunable/bounded | ✅ Bounded |
| DM privacy | ❌ Broken | ✅ Preserved | ✅ Preserved |
| Regression risk | ⚠️ High | ✅ Minimal | ✅ Minimal |
| Cross-channel continuity | ✅ Perfect | ✅ Good | ✅ Excellent |
| Cut 1 readiness | ⚠️ Risky | ✅ Yes | ❌ No |

---

## 5. Phasing Recommendation

### Cut 1 — Minimal Viable Unification (Option B, ~150-200 LOC)

**Branch:** `feat/91-unified-session-cut1`

**Changes:**

```
1. crates/zeus-session/src/store.rs
   + SessionStore::tail_snapshot(id: &str, n: usize) -> Vec<Message>
     Reads last N messages from JSONL without acquiring write lock.

2. crates/zeus-session/src/channel_router.rs
   + ChannelSessionRouter::active_keys() -> Vec<ChannelKey>
     Returns keys currently tracked in the in-memory cache.
   + ChannelKey::is_dm() -> bool
     Returns true if user_id is Some (4-part key).

3. src/gateway.rs ~L1768
   After session swap: collect tail from active non-DM channels,
   prepend as context block before cook_with_history_interruptible.
   Gate: config cross_channel_session_tail_n (default 0 = disabled).

4. crates/zeus-core/src/lib.rs (PrometheusConfig)
   + cross_channel_session_tail_n: Option<usize>  (default None = 0)

5. Tests (≥6):
   - tail_snapshot returns last N correctly
   - tail_snapshot n=0 returns empty
   - active_keys returns cached keys
   - is_dm() true/false
   - injection skips DM sessions
   - n=0 bypass (no injection, behavior identical to today)
```

**Verified seam points:**
- `channel_router.rs:L61` — `ChannelKey` struct, add `is_dm()` method
- `store.rs:L106` — add `tail_snapshot` alongside `load`
- `gateway.rs:L1768` — injection immediately after session swap (mirrors L2328 Mnemosyne pattern)
- `zeus-prometheus/src/lib.rs:L1535` — `conversation_history: &[Message]` accepts augmented slice, no signature change

---

### Cut 2 — Polish (~100 LOC)

```
- Per-channel tail_n config (override global default per channel)
- Explicit DM exclusion policy configurable (allow_dm_cross_channel: bool)
- Tail ordering option: newest-foreign-first vs oldest-foreign-first
- Tracing spans: "cross-channel tail injected N messages from K channels"
- UI: meta_loop.rs surfaces cross_channel_session_tail_n hint
```

---

### Cut 3 — Global Distilled Session (Option C, ~400 LOC)

```
- Global session agent:global:main lifecycle management
- Post-cook summarization: 1-3 line distillation appended to global session
- Global tail replaces per-channel raw tail injection from Cut 1
- Retire cross_channel_session_tail_n in favor of global_session_tail_n
- Migration: existing per-channel sessions preserved, global starts fresh
```

---

## 6. #86 Architectural Sibling

The `#86` trilogy established: cross-channel **semantic** memory injection at cook entry via Mnemosyne (`gateway.rs:L2328`). The `#91` design extends the same shape from semantic to **episodic** context:

```
#86 shape:  mnemosyne.retrieve(query)              → inject as system context before cook
#91 shape:  session_store.tail_snapshot(other_ch)  → inject as history slice before cook
```

Both are pre-cook injections at the gateway boundary. Both are additive and feature-flaggable. Both leave channel adapters and PrometheusEngine untouched. Cut 1 implementation should mirror `#86`'s injection style for consistency and reviewability.

The `universal-features-route-through-gateway` standing discipline applies: all cross-channel session logic lives in `gateway.rs` + `zeus-session`. Channel adapters stay channel-unaware.

---

*Substrate citations verified via `git show origin/main:<path>`. All line numbers confirmed against HEAD `f82a9cef`.*
