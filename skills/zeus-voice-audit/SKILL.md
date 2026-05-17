---
name: zeus-voice-audit
description: Audit channel wiring end-to-end in the Zeus codebase. Traces each channel from onboarding config save → agent_loop adapter creation → ChannelManager registration. Use when checking which channels actually connect vs which are stubs, or when diagnosing why a channel isn't receiving/sending messages.
---

# Zeus Voice Audit

## When to Use

Trigger on: channel not connecting, audit channel wiring, which channels work, channel stub check, trace message path, why isn't [channel] working.

**Use for:**
- Determining which channels are fully wired vs stubs
- Tracing the full path: onboarding → config → adapter → manager
- Diagnosing a channel that's configured but not receiving messages
- Pre-release channel coverage checks

**NOT for:**
- Debugging message content (use zeus-heartbeat-debug)
- Fleet health checks (use zeus-fleet-health)
- Config value problems (use zeus-config-audit)

---

## Channel Wiring Status (S81 Audit)

### Fully Wired — Live Adapters

These channels go all the way from onboarding config → `agent_loop.rs` adapter creation → `manager.add_adapter()` → `start_all()`:

| Channel | Adapter | Receive Mode | Notes |
|---------|---------|--------------|-------|
| **Telegram** | `TelegramAdapter` | Native (MTProto via grammers) | Bot mode + user mode. Typing indicator via Bot API. |
| **Discord** | `DiscordAdapter` | WebSocket (serenity) | Full: slash commands, reactions, threads, webhooks, voice. Tier 1 native identity via webhook_url. |
| **Slack** | `SlackAdapter` | WebSocket (Socket Mode) | Full: rich messages, threads, file upload. Tier 1 native identity via `send_message_as`. |
| **Email** | `EmailAdapter` | SMTP/IMAP (lettre + async-imap) | Send + receive wired. Requires SMTP + IMAP config. |
| **WhatsApp** | `WhatsAppAdapter` | Bridge or Cloud API | Two modes: Bridge (local process) and Cloud API (webhook). Multi-account via named accounts. |
| **Signal** | `SignalAdapter` | ExternalProcess (signal-cli JSON-RPC) | Requires `signal-cli` daemon running locally. Multi-account supported. |
| **Matrix** | `MatrixAdapter` | WebSocket (reqwest Client-Server API) | Full send/receive. Multi-account supported. |
| **IRC** | `IrcAdapter` | Native (tokio raw TCP) | Wired — no multi-account path seen. |

### NOT Wired — Missing from agent_loop

These channels have adapter implementations in `zeus-channels` but are **never added to the ChannelManager** in `agent_loop.rs`:

| Channel | Adapter Exists | Reason Not Wired |
|---------|---------------|-----------------|
| **iMessage** | ✅ `IMessageAdapter` | No `add_adapter` call in agent_loop. Config field exists (`cc.imessage`) but never consumed for adapter creation. |
| **X/Twitter** | ✅ `XAdapter` | Social media adapter — not wired for real-time messaging. |

---

## End-to-End Trace

### Path for a wired channel (example: Discord)

```
1. Onboarding (zeus-tui)
   └─ User sets discord.bot_token, discord.channel_id in TUI
   └─ Config saved to ~/.zeus/config.toml

2. Agent startup (zeus-agent/src/agent_loop.rs ~L434)
   └─ Reads config: cc.discord (DiscordChannelConfig)
   └─ Builds DiscordConfig { bot_token, ... }
   └─ DiscordAdapter::new(discord_config).await
   └─ manager.add_adapter(Box::new(adapter))

3. ChannelManager (zeus-channels/src/lib.rs)
   └─ manager.start_all() → adapter.start(tx).await
   └─ Spawns serenity client in background task
   └─ Messages flow into mpsc channel → agent loop

4. Outbound
   └─ agent_loop calls manager.send(&source, content)
   └─ find_adapter() matches channel_type + account_id
   └─ adapter.send() or send_as() → Discord HTTP API
```

### Path for iMessage (NOT wired)

```
1. Onboarding: iMessage config exists in TUI (screen present)
2. Config saved to ~/.zeus/config.toml with imessage section
3. agent_loop.rs: cc.imessage field EXISTS but no adapter is created
   └─ No IMessageAdapter::new() call
   └─ No manager.add_adapter() call
4. Result: iMessage messages never reach the agent loop
```

---

## Audit Procedure

### Quick check — is a channel actually running?

```bash
# 1. Check config has the channel enabled
grep -A5 '\[channels.discord\]' ~/.zeus/config.toml

# 2. Check agent_loop wires it (look for add_adapter calls)
grep -n "add_adapter\|<channel>Adapter" ~/Zeus/crates/zeus-agent/src/agent_loop.rs

# 3. Runtime check — connected channels at startup
# Look for log lines like: "Discord adapter started"
grep "adapter started\|adapter created" ~/.zeus/logs/gateway.log

# 4. Verify messages flow
# Send a test message on the channel and watch logs:
tail -f ~/.zeus/logs/gateway.log | grep -i "received\|discord\|telegram"
```

### Full wiring trace for any channel

```bash
# Step 1: Find the config struct
grep -n "<Channel>Config\|<channel>_config" ~/Zeus/crates/zeus-agent/src/agent_loop.rs

# Step 2: Verify add_adapter is called
grep -n "add_adapter" ~/Zeus/crates/zeus-agent/src/agent_loop.rs

# Step 3: Check the adapter's start() method
grep -n "fn start\|fn send\|fn is_connected" ~/Zeus/crates/zeus-channels/src/<channel>.rs

# Step 4: Check for todo!/unimplemented! (stub indicators)
grep -n "todo!\|unimplemented!" ~/Zeus/crates/zeus-channels/src/<channel>.rs
```

---

## Common Issues

**Channel configured but not receiving messages:**
1. Check `agent_loop.rs` — is `add_adapter` called for this channel?
2. Check the adapter's `start()` — does it return early on missing credentials?
3. Check logs for `"Failed to create <Channel> adapter"` — a `warn!` on error means silent skip.

**iMessage not working:**
The `IMessageAdapter` uses AppleScript (macOS only) and is fully implemented in `zeus-channels`, but `agent_loop.rs` never instantiates it. This is a known gap — the config field `cc.imessage` exists but the wiring is missing.

**Signal not receiving:**
Signal requires `signal-cli` running as a JSON-RPC daemon. If `signal-cli` isn't running, the adapter start fails silently (warn, not error). Verify:
```bash
pgrep -fl signal-cli
```

**WhatsApp Bridge vs Cloud API:**
WhatsApp has two modes. Bridge mode requires a local WhatsApp bridge process. Cloud API mode requires a Meta webhook endpoint. Check `whatsapp.mode` in config.

**Multi-account channels:**
Discord, Slack, WhatsApp, Signal, and Matrix all support named accounts in config. Each named account gets its own adapter instance in the manager, tagged with `account_id` for routing.

---

## Channel Capability Matrix

| Channel | Send | Receive | Typing | Reactions | Threading | Files | Native Identity |
|---------|------|---------|--------|-----------|-----------|-------|----------------|
| Telegram | ✅ | ✅ | ✅ | ❌ | ❌ | ✅ | ❌ (Tier 2) |
| Discord | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ (webhook) |
| Slack | ✅ | ✅ | ✅ | ❌ | ✅ | ✅ | ✅ (postMessage) |
| Email | ✅ | ✅ | ❌ | ❌ | ❌ | ✅ | ❌ |
| iMessage | ✅ | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ |
| WhatsApp | ✅ | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ |
| Signal | ✅ | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ |
| Matrix | ✅ | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ |
| IRC | ✅ | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ |
| X/Twitter | ✅ | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ |

_iMessage: adapter implemented, NOT wired in agent_loop_
_X/Twitter: adapter implemented, NOT wired for real-time messaging_

---

## Files to Know

| File | Purpose |
|------|---------|
| `crates/zeus-agent/src/agent_loop.rs` | Where adapters are created and registered — the wiring source of truth |
| `crates/zeus-channels/src/lib.rs` | `ChannelManager`, `ChannelAdapter` trait, `ChannelMessage` types |
| `crates/zeus-channels/src/config.rs` | `ChannelsConfig` — all channel config structs |
| `crates/zeus-channels/src/<channel>.rs` | Per-channel adapter implementation |
| `~/.zeus/config.toml` | Runtime config — what's actually enabled |
