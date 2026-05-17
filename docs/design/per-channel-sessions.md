# Per-Channel Sessions — Design Scoping

**Branch:** `feat/per-channel-sessions`
**Owner:** fbsd2
**Status:** Scoping (awaiting direction)

## Problem

Today, most relays share a single "default" Zeus session per agent. That means:
- Two Discord channels talking to the same bot → same session → cross-contamination
- Slack DM + Slack channel → share history
- Context window fills up with irrelevant threads

Telegram already solved this with `RelaySessionKey` + per-chat `zeus_session_id`. The pattern exists; it's not applied elsewhere.

## Current State (verified)

| Relay    | Per-channel session? | Location |
|----------|---------------------|----------|
| Telegram | ✅ yes               | `telegram_relay.rs:439` (`RelaySessionKey` → `ChatState.zeus_session_id`) |
| Discord  | ❌ single default    | needs audit |
| Slack    | ❌ single default    | needs audit |
| WebChat  | partial (client_id ≈ session_id) | `webchat.rs:20` |
| TUI      | pane-based (separate concern) | — |

`zeus-session::store::SessionStore::acquire(&session_id)` is already keyed by string — the storage layer is ready. The gap is in **routing**: relays don't look up the right session_id per channel.

## Design Options

### Option A — Narrow (Discord only)
Mirror Telegram's `RelaySessionKey` pattern inside `discord_relay.rs`. ~150 LOC.
- **Pro:** small, reviewable, ships this sprint
- **Con:** duplicates the Telegram pattern; tech debt we'll pay again for Slack

### Option B — Shared helper in `zeus-session`
Add `ChannelSessionRouter { map: HashMap<ChannelKey, SessionId> }` in zeus-session. Each relay constructs one, calls `router.resolve(channel_key)` before `acquire()`. ~250 LOC + refactor Telegram to use it.
- **Pro:** one pattern, not three
- **Con:** touches Telegram (already working); risk of regression

### Option C — Infrastructure layer
Formalize a `SessionRouter` trait in `zeus-core`, with persistence (survives restarts, maps stored in sqlite/JSONL). ~500 LOC.
- **Pro:** correct long-term answer
- **Con:** scope creep for one sprint

## Recommendation

**Option B** — but phased:
1. Land the router in `zeus-session` with Discord as first consumer (this PR)
2. Migrate Telegram in a follow-up PR (no behavior change, just deduplication)
3. Slack + WebChat as they come up

This keeps the blast radius small while laying the load-bearing wall correctly.

## Open Questions (for Zeus100)

1. **Scope confirm:** is this Discord-only, or all relays?
2. **Persistence:** should channel→session mapping survive agent restart? (Telegram's does; others don't)
3. **Naming:** `session-{channel_id}` vs `{agent}-{channel_id}` vs user-specified?
4. **Existing sessions:** migrate old "default" session per channel, or fresh start?

## Next Steps (once scope confirmed)

- [ ] Add `ChannelSessionRouter` to `zeus-session`
- [ ] Wire Discord relay to route by `channel_id`
- [ ] Tests: two channels → two session files → no cross-contamination
- [ ] Docs update in `docs/channels/`
