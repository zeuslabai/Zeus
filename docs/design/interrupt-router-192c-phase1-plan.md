# #192c Interrupt Router — Phase 1 Lifecycle Map + Development Plan

Status: phase-1 plan cut, no runtime code changes.
Base inspected: `origin/main` at `1f59a8da201eb6fe515dba2b537818734524bb68`.

## Problem statement

The new blocking TUI path enters the same cooking loop as other surfaces, but uses `/v1/chat` -> `InboxSender::send_and_wait`. When that request is interrupted by caller timeout, client disconnect, or the inbox consumer's processing timeout, the cook future can be dropped while its checkpoint is still resumable. Bootstrap then finds the incomplete checkpoint and can auto-resume with the wrapped `"You were in the middle of..."` prompt, which turns a transient interruption into recursive session corruption.

The fix should not special-case TUI. Channel surfaces should remain transports; session/cook semantics must be uniform. The lifecycle bug is ownership: interrupt routing and checkpoint cleanup need to be tied to the cook's actual lifetime, not to a happy-path HTTP response.

## Current lifecycle map

### Blocking `/v1/chat` / `send_and_wait`

Observed path on `origin/main`:

1. `crates/zeus-api/src/handlers/chat_handlers.rs` calls `inbox.send_and_wait(..., use_cooking = true)` for blocking chat.
2. `crates/zeus-core/src/inbox.rs:88-121` creates a `oneshot` response channel, enqueues `InboxMessage`, then waits on the `oneshot` under a caller-side timeout.
3. `crates/zeus-core/src/inbox.rs:207-236` `run_consumer` receives the message and separately wraps `handler(msg)` in `tokio::time::timeout(msg.timeout_secs, ...)`.
4. `src/gateway.rs` resolves the inbox cook session as `agent:main:main` and acquires the per-session lane before cooking.
5. `crates/zeus-prometheus/src/cooking_checkpoint.rs` stores incomplete cooking sessions until they are marked completed or deleted.
6. `src/gateway_bootstrap.rs:180-250` discovers incomplete checkpoints and auto-resumes by building `You were in the middle of: ...` from `original_message`.

Interrupt/drop hazards:

- Caller-side `send_and_wait` timeout can drop the waiting future before the consumer finishes.
- Consumer-side `tokio::time::timeout(handler(msg))` can drop the actual cook future.
- If the cook future is dropped after checkpoint start/save but before completion cleanup, the checkpoint remains resumable.
- The response channel closing is not currently a cook-lifetime cleanup primitive.

### Streaming `/v1/chat/completions` / `send_and_stream`

Observed path on `origin/main`:

1. `crates/zeus-api/src/handlers/chat_handlers.rs` calls `inbox.send_and_stream(..., use_cooking = true)` for streaming chat completions.
2. `crates/zeus-core/src/inbox.rs:126-151` creates an `mpsc` response stream, enqueues `InboxMessage`, and returns the receiver immediately.
3. The same `run_consumer` timeout and gateway cook handler execute the cook.

Important delta:

- Old TUI used streaming `/v1/chat/completions` + `send_and_stream`.
- New TUI uses blocking `/v1/chat` + `send_and_wait`.
- The resolved session is not the old/new delta; both TUI paths cook in `agent:main:main` through the inbox. The regression lives in blocking request interruption/cleanup behavior.

### Channel surfaces

Discord/channel consumers resolve a channel context and build `CookContext` from that resolved session instead of funneling through the inbox main session. That isolation explains why Discord is less likely to corrupt titan's main session, but #192c should still make interrupt/checkpoint semantics uniform across surfaces.

## Design goals

1. Deliver interrupts over a per-session `mpsc` lane, not broadcast.
2. Make lane registration/cleanup RAII-owned by the active cook.
3. Make checkpoint cleanup RAII-owned by the active cook.
4. Preserve existing stop, pause, and redirect semantics exactly.
5. Keep session/cook behavior uniform across TUI, Discord, Telegram, and other surfaces; transport should not decide corruption behavior.
6. Clean bad historical checkpoints at bootstrap instead of merely skipping them.
7. Bound resume attempts so an interrupted or recursive checkpoint cannot loop forever.

## Proposed architecture

### 1. `SessionLaneManager`

Extend the existing per-session lane primitive into a lifecycle manager:

- Registry key: resolved cook session id.
- Per-session state:
  - async mutex for FIFO single-cook serialization within the session;
  - optional active interrupt `mpsc::UnboundedSender<InterruptCommand>` for the currently running cook.
- `begin_cook(session_key)` returns:
  - the acquired lane guard;
  - an interrupt receiver for this cook;
  - a RAII interrupt registration guard.
- `send_interrupt(session_key, command)` sends to exactly the active cook for that session.
- Dropping the registration guard clears the sender only if it still belongs to that cook, preventing a late drop from clearing a newer cook's lane.

Why `mpsc`, not broadcast:

- There is only one active cook per session.
- Stop/pause/redirect commands are work items for that cook, not announcements to historical receivers.
- `mpsc` avoids missed/duplicated wakeups caused by broadcast lag, stale subscribers, or receiver churn.

### 2. Checkpoint RAII guard

Introduce a cook-scoped checkpoint guard in the cooking/checkpoint layer:

- Created when a cook starts checkpointing.
- Tracks `session_id`, raw original user message, store handle, and completion state.
- `mark_completed()` marks the checkpoint completed and disables drop cleanup.
- `abandon()` / drop path deletes the incomplete checkpoint if the cook exits through timeout, cancellation, client disconnect, panic unwind, or response-channel loss.

Rules:

- Checkpoint the raw user message, never a bootstrap-generated resume wrapper.
- Completion is explicit; every non-completion exit path deletes the incomplete checkpoint.
- Drop cleanup must be best-effort and non-blocking-safe. If direct async cleanup is impossible in `Drop`, spawn a small cleanup task using the store handle.

### 3. Interrupt command model

Represent stop/pause/redirect as typed commands instead of raw strings where practical:

- `Stop` keeps current stop semantics.
- `Pause` keeps current pause semantics and should leave only intentionally resumable state.
- `Redirect { target/session/channel }` keeps existing redirect semantics and must not duplicate delivery.

If a full enum would make the first code cut too wide, keep the existing command payload at the boundary but centralize routing through `SessionLaneManager`; convert to typed commands in a follow-up only after parity tests exist.

### 4. Bootstrap cleanup + bounded resume

On boot:

- Scan incomplete checkpoints.
- Delete recursive checkpoints whose `original_message` already contains the resume wrapper marker.
- Delete oversized checkpoints above a conservative configured threshold.
- Add max-attempts and/or TTL metadata to resume state.
- Resume only checkpoints that are non-recursive, under size, within TTL, and below max attempts.

The current bootstrap recursive gate is directionally right, but skipping is not enough. Bad checkpoints should be cleaned so the same boot does not rediscover them forever.

## Phase cuts

### Phase 1 — plan/lifecycle map

This document only. No runtime behavior change.

Verification:

- `git diff --check`
- branch push verified on origin

### Phase 2 — checkpoint RAII guard

Files expected:

- `crates/zeus-prometheus/src/cooking_checkpoint.rs`
- cooking loop call sites that start/complete checkpoint sessions
- focused tests for dropped/timed-out cook cleanup

Acceptance:

- A cook dropped before completion deletes its incomplete checkpoint.
- A normally completed cook marks completed and is not deleted by the guard.
- Stored `original_message` is raw user content, not a resume wrapper.

### Phase 3 — `SessionLaneManager` interrupt lane

Files expected:

- `crates/zeus-core/src/session_lane.rs`
- gateway/inbox wiring where cooks acquire lanes and where interrupts route

Acceptance:

- One active `mpsc` receiver per session cook.
- Interrupt delivered to the active cook only.
- Dropping/timing out a cook clears its interrupt sender.
- Same-session cooks stay FIFO; different sessions still run concurrently.

### Phase 4 — preserve stop/pause/redirect semantics

Files expected:

- `src/gateway.rs`
- any existing interrupt command handlers
- regression tests around stop, pause, redirect

Acceptance:

- Stop still stops the active session cook.
- Pause still creates intentional resumable state only when pause semantics require it.
- Redirect still routes exactly once and does not leave stale interrupt receivers.

### Phase 5 — bootstrap cleanup + bounded resume

Files expected:

- `src/gateway_bootstrap.rs`
- `crates/zeus-prometheus/src/cooking_checkpoint.rs`

Acceptance:

- Recursive checkpoints are deleted, not just skipped.
- Oversized checkpoints are deleted or quarantined.
- Resume has max-attempts and/or TTL.
- Boot cannot recurse indefinitely on the same checkpoint.

### Phase 6 — gates + push

Run:

- `cargo test --bin zeus`
- `cargo clippy --bin zeus --no-deps`
- `cargo build --workspace --locked`

Push the final implementation SHA for gate/read.

## Test plan

Minimum focused tests:

1. `send_and_wait` interruption leaves `agent:main:main` without an incomplete checkpoint.
2. Consumer-side timeout during a cook invokes checkpoint cleanup.
3. Client disconnect / closed response channel does not leave a resumable checkpoint.
4. Recursive checkpoint with `You were in the middle of:` in `original_message` is deleted during bootstrap.
5. Oversized checkpoint is cleaned or quarantined during bootstrap.
6. Resume attempts are bounded by attempts/TTL.
7. Stop command reaches exactly one active cook in a session.
8. Pause semantics remain intentionally resumable and do not use the abandon-cleanup path incorrectly.
9. Redirect semantics route once and clear the old lane.
10. Same-session cooks serialize FIFO; different sessions remain concurrent.

## Open uncertainties to resolve during code cuts

- The exact current interrupt payload type may be stringly-typed in some paths; if so, Phase 3 should first centralize routing without widening semantics.
- Async cleanup from `Drop` needs care. The likely implementation is a guard that spawns cleanup onto the runtime, with tests proving cleanup completes before assertions.
- Pause may intentionally preserve a checkpoint. That path must explicitly disarm the abandon guard; all other interrupt exits should clean.
- If bootstrap metadata lacks attempt counters, Phase 5 may need a small schema migration for resume attempts and last-resume timestamp.
