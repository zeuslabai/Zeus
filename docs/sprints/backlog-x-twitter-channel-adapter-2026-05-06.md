# Backlog — X/Twitter ChannelAdapter (P0 urgent)

**Filed:** 2026-05-06
**Owner suggested:** ZeusMarketing (lived experience), zeus106 (channels lane), or fbsd2 (operator/IaC)
**Estimate:** 6-10h (adapter impl + tests + outbound wiring)
**Priority:** P0 — merakizzz escalated 2026-05-06 after ZeusMarketing test confirmed gap

---

## Resolution

**2026-05-06 — Closed as routing fix, NOT new adapter.**

The `XAdapter` already existed at `crates/zeus-channels/src/x.rs` (1422 lines, fully wired with OAuth 1.0a + 2.0, polling, threaded send). The gap was purely naming: adapter registered as `channel_type = "x"` but config and `message(channel="x_twitter", ...)` callers used `"x_twitter"`.

**Actual fix:** `XAdapter::channel_type()` renamed `"x"` → `"x_twitter"` (`d74073ae`). Zero new adapter code required. Zero callers using the short `"x"` form (verified grep).

**Sub-cooks invalidated:** items 1–3 (new adapter scaffold) were unnecessary. Items 4–6 (inbound polling, ChannelManager registration, unit tests) remain potential future work if richer X integration is desired — but the routing gap is closed.

```bash
# grep confirming no "x" callers (2026-05-06)
grep -rn 'channel="x"' --include='*.rs' --include='*.json' --include='*.md' \
  $(git rev-parse --show-toplevel)/crates $(git rev-parse --show-toplevel)/src \
  $(git rev-parse --show-toplevel)/docs 2>/dev/null | grep -v target | grep -v _legacy
# → 0 matches
```

## Original Problem

`crates/zeus-channels/` has no `x_twitter.rs` adapter. The `message` tool reads the `ChannelAdapter` registry to surface available channels; X/Twitter is missing despite `[channels.x_twitter]` config being parsed. Net result: agents cannot post to X via the canonical `message(channel="x_twitter", ...)` call.

## Witness

ZeusMarketing on 2026-05-06 ran merakizzz's "test post + reply on X" dispatch:
- `message` tool only exposed: `discord, telegram, slack, email, imessage, irc, matrix, whatsapp, signal, mattermost, file, webhook` — no `x_twitter`
- Workaround used: `tweepy` + OAuth 1.0a credentials read directly from `[channels.x_twitter]` config via Python shell script
- Confirmed working tweet: id `2051757614732840981`
- Confirmed working reply: id `2051757669078462529`
- Script saved at `~/.zeus/workspace/scripts/zeus-twitter-post.py` for reuse

merakizzz noted earlier ZeusMarketing iterations had this work — fresh-context retry on 2026-05-06 corroborates same settings + same workaround pattern. Stable across context resets.

## Scope

Create `crates/zeus-channels/src/x_twitter.rs` implementing the `ChannelAdapter` trait per the canonical Discord/Slack/Matrix pattern.

### `ChannelAdapter` trait surface (from `crates/zeus-channels/src/lib.rs:536`)

```rust
pub trait ChannelAdapter: Send + Sync {
    fn name(&self) -> &str;
    async fn start(&self, tx: mpsc::Sender<ChannelMessage>) -> Result<()>;
    async fn send(&self, to: &ChannelSource, content: &str) -> Result<()>;
    // ... plus stop, status, etc. — match existing impls
}
```

### Reference impl

The Discord adapter (`crates/zeus-channels/src/discord.rs`) is the source-of-truth pattern. Slack and Matrix have similar shapes. ZeusMarketing's working `tweepy` script captures the OAuth 1.0a auth flow that needs to happen in Rust.

### Twitter API v2 endpoints to wire

| Operation | Endpoint | HTTP |
|---|---|---|
| Verify auth | `/2/users/me` | GET (cheap, use for `start()` health-check) |
| Post tweet | `/2/tweets` | POST `{"text": "..."}` |
| Reply to tweet | `/2/tweets` with `reply: {"in_reply_to_tweet_id": "..."}` | POST |
| Quote tweet | `/2/tweets` with `quote_tweet_id: "..."` | POST |
| Delete tweet | `/2/tweets/<id>` | DELETE |
| Read mentions (inbound) | `/2/users/<my_id>/mentions` | GET (poll every 30-60s, similar to Telegram bot polling) |
| User lookup | `/2/users/by/username/<handle>` | GET |

### Auth shape

Twitter API v2 supports two auth modes:
1. **OAuth 1.0a** (user context, what tweepy uses) — required for posting on behalf of a user account
2. **Bearer token** (app-only) — only for read endpoints

For posting, OAuth 1.0a is required. Need:
- `consumer_key` (a.k.a. `api_key`)
- `consumer_secret` (a.k.a. `api_secret`)
- `access_token` (user-specific)
- `access_token_secret` (user-specific)

These should map to `[channels.x_twitter]` config fields. Confirm what ZeusMarketing's existing config looks like and align the struct shape.

### Crate dep options

Rust ecosystem options for Twitter API v2 + OAuth 1.0a:
- `egg-mode` (mature, well-maintained, supports OAuth 1.0a)
- `twitter-v2` (newer, focused on v2)
- Hand-rolled `reqwest` + `oauth1-rs` (lightest, but more code)

Lean: `egg-mode` for breadth; alt path is hand-rolled if egg-mode pulls excessive deps.

### `ChannelSource` mapping

For X/Twitter:
- `channel_type = "x_twitter"`
- `user_id` = Twitter user ID (numeric)
- `chat_id` = optional, can encode reply-thread root tweet ID
- `thread_id` = the `in_reply_to_tweet_id` for replies

Inbound (mentions polling): emit `ChannelMessage` with `is_addressed = Some(true)` since the bot was @-mentioned.

## Out of scope

- DMs (Twitter API v2 DM endpoints have separate auth + lower priority for fleet ops)
- Threads (multi-tweet sequences) — initial scope is single tweets + replies
- Media uploads — start text-only

## Acceptance gate

- [ ] `crates/zeus-channels/src/x_twitter.rs` exists, implements `ChannelAdapter`
- [ ] Adapter registered in `ChannelManager` so `message` tool surfaces `x_twitter`
- [ ] `[channels.x_twitter]` config struct has all 4 OAuth 1.0a fields + optional `default_user_id`
- [ ] `cargo check -p zeus-channels` clean
- [ ] Unit tests covering: config parse, auth construction, tweet POST request shape, reply request shape, mention polling parse
- [ ] Manual smoke test on a sandbox X account (post + reply + delete) confirms end-to-end
- [ ] ZeusMarketing's `~/.zeus/workspace/scripts/zeus-twitter-post.py` workaround can be retired once adapter merges

## Related

- ZeusMarketing X workaround: `~/.zeus/workspace/scripts/zeus-twitter-post.py`
- Reference adapter: `crates/zeus-channels/src/discord.rs`
- Channel adapter trait: `crates/zeus-channels/src/lib.rs:536`
- ChannelManager registration: `crates/zeus-channels/src/lib.rs:795` (`send` method dispatch)
