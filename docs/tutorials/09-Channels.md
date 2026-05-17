# Channels — Messaging Integrations

Zeus connects to 9 messaging platforms. Each channel is bidirectional — Zeus receives messages and replies through the same platform.

## Overview

| Channel | Protocol | Setup Complexity |
|---------|----------|-----------------|
| **Discord** | WebSocket (bot) | Easy — create bot, add token |
| **Telegram** | MTProto (user account) | Medium — API ID + hash + phone |
| **Slack** | Socket Mode | Medium — create app, scopes, install |
| **Email** | SMTP + IMAP IDLE | Easy — SMTP/IMAP credentials |
| **iMessage** | AppleScript (macOS) | Easy — macOS only, Messages app |
| **WhatsApp** | Cloud API (webhook) | Complex — Meta Business setup |
| **Signal** | signal-cli JSON-RPC | Complex — link device, install CLI |
| **Matrix** | matrix-sdk | Medium — homeserver + credentials |
| **MQTT** | MQTT v5 | Easy — broker URL + topic |

## Starting Channels

Channels start automatically with the gateway:

```bash
zeus gateway                 # All channels enabled
zeus gateway --no-channels   # API only, no channels
```

## Discord

### Setup

1. Go to [Discord Developer Portal](https://discord.com/developers/applications)
2. Create a new application → Bot → copy token
3. Enable **Message Content Intent** under Bot settings
4. Invite bot to your server with `bot` + `applications.commands` scopes

### Configure

```bash
# ~/.zeus/.env
DISCORD_BOT_TOKEN=your_bot_token
DISCORD_RELAY_CHANNEL_IDS=123456789012345678  # Channel IDs to monitor
```

```toml
# ~/.zeus/config.toml
[channels.discord]
token = ""   # Reads from DISCORD_BOT_TOKEN env var
```

### Test

```bash
zeus gateway
# In Discord: @Zeus ping
```

## Telegram

Zeus uses the Telegram MTProto protocol (user account, not bot API) via the `grammers` library.

### Setup

1. Go to [my.telegram.org](https://my.telegram.org) → API development tools
2. Create an app → get API ID and API Hash

### Configure

```bash
# ~/.zeus/.env
TELEGRAM_API_ID=12345
TELEGRAM_API_HASH=your_api_hash
TELEGRAM_PHONE=+1234567890
```

```toml
# ~/.zeus/config.toml
[channels.telegram]
api_id = 12345
api_hash = ""  # Reads from env
phone = "+1234567890"
```

### Test

```bash
zeus gateway
# First run: interactive phone code verification
# Then: send a Telegram message to yourself — Zeus will reply
```

## Slack

### Setup

1. Go to [api.slack.com/apps](https://api.slack.com/apps) → Create New App → From Scratch
2. Enable **Socket Mode** → generate App-Level Token (`xapp-...`) with `connections:write` scope
3. Add **Bot Token Scopes**: `chat:write`, `channels:read`, `channels:history`, `groups:read`, `groups:history`, `im:read`, `im:history`, `reactions:write`, `files:write`
4. **Install to Workspace** → copy Bot User OAuth Token (`xoxb-...`)
5. Enable **Event Subscriptions** → subscribe to: `message.channels`, `message.groups`, `message.im`
6. In Slack, `/invite @Zeus` to your channel

### Configure

```bash
# ~/.zeus/.env
SLACK_BOT_TOKEN=xoxb-...
SLACK_APP_TOKEN=xapp-...
SLACK_SIGNING_SECRET=...
```

```toml
[channels.slack]
bot_token = ""
app_token = ""
```

### Test

```bash
zeus gateway
# In Slack: @Zeus hello
```

## Email

### Configure

```toml
[channels.email]
smtp_host = "smtp.gmail.com"
smtp_port = 587
imap_host = "imap.gmail.com"
imap_port = 993
username = "you@gmail.com"
password = "app-password"    # Use Gmail app password, not your real password
use_tls = true
```

Zeus uses IMAP IDLE for real-time email reception and SMTP for sending.

### Test

```bash
zeus gateway
# Send an email to the configured address — Zeus reads via IMAP and replies via SMTP
```

## iMessage (macOS only)

Requires macOS with Messages app signed in.

```bash
zeus gateway
# Send an iMessage to the configured number — Zeus replies via AppleScript
```

Tools available:
```bash
zeus tool imessage_send '{"to":"+1234567890","message":"Hello"}'
zeus tool imessage_read '{}'
zeus tool imessage_list_conversations '{}'
```

## WhatsApp

Requires Meta Business account and WhatsApp Cloud API.

### Setup

1. [developers.facebook.com](https://developers.facebook.com) → Create App → Business type
2. Add WhatsApp product → API Setup
3. Generate token with `whatsapp_business_messaging` scope
4. Copy Phone Number ID (not the phone number)
5. Set webhook URL: `https://YOUR_GATEWAY/v1/webhooks/whatsapp`

### Configure

```bash
# ~/.zeus/.env
WHATSAPP_TOKEN=EAAxxxxx...
WHATSAPP_PHONE_NUMBER_ID=123456789012345
```

```toml
[channels.whatsapp]
mode = "cloud_api"
```

> ⚠️ Env var is `WHATSAPP_TOKEN` — NOT `WHATSAPP_ACCESS_TOKEN`.

## Signal

Requires `signal-cli` installed and linked to your phone.

### Setup

```bash
# macOS
brew install signal-cli

# Link as secondary device (keeps Signal working on your phone)
signal-cli link --name "Zeus"
# Scan the QR code with Signal app → Settings → Linked Devices
```

> ⚠️ Use `link`, not `register`. Registering deactivates Signal on your phone.

### Configure

```bash
# ~/.zeus/.env
SIGNAL_PHONE=+1234567890
SIGNAL_CLI_PATH=/usr/local/bin/signal-cli
```

```toml
[channels.signal]
phone = "+1234567890"
signal_cli_path = "/usr/local/bin/signal-cli"
```

## Matrix

```toml
[channels.matrix]
homeserver = "https://matrix.org"
user = "@zeus:matrix.org"
password = "your-password"
```

## Channel API

Manage channels via the API:

```bash
# List channels
curl http://localhost:3001/v1/channels | jq

# Create a channel (NOTE: field is channel_type, not type)
curl -X POST http://localhost:3001/v1/channels \
  -H "Content-Type: application/json" \
  -d '{"channel_type":"telegram","name":"My Telegram"}'

# Test connectivity
curl -X POST http://localhost:3001/v1/channels/discord/test | jq
```

## What's Next

→ [[10-MCP-Integration]] — Zeus as MCP server for Claude Code
→ [[12-Gateway]] — Run all channels via the gateway
