# Channels

Zeus supports 8 messaging channel adapters via the `ChannelAdapter` trait in the `zeus-channels` crate. Channels allow Zeus to send and receive messages across different platforms, turning the assistant into a multi-platform chatbot or notification system.

## Supported Channels

| Channel | Library / Method | Config Location |
|---------|-----------------|-----------------|
| [Telegram](telegram.md) | grammers-client (MTProto) | `[channels.telegram]` |
| [Discord](discord.md) | serenity (gateway + HTTP) | `[channels.discord]` |
| [Slack](slack.md) | reqwest + tokio-tungstenite (Socket Mode) | `[channels.slack]` |
| [Email](email.md) | lettre (SMTP) + async-imap (IMAP IDLE) | `[channels.email]` |
| [iMessage](imessage.md) | AppleScript bridge (macOS only) | No config needed |
| [WhatsApp](whatsapp.md) | Cloud API via reqwest | `[channels.whatsapp]` |
| [Signal](signal.md) | signal-cli (JSON-RPC subprocess) | `[channels.signal]` |
| [Matrix](matrix.md) | matrix-sdk v0.16 (native Rust) | Environment variables |

## Architecture

All channels implement the `ChannelAdapter` trait, which provides a uniform interface for sending and receiving messages. The `ChannelManager` routes outbound messages to the correct adapter and collects inbound messages via an mpsc channel.

Additional features built into the channel system:

- **Message chunking** -- Long messages are automatically split to respect platform limits.
- **Streaming delivery** -- Responses can be streamed incrementally to supported channels.
- **Channel policies** -- Per-channel rules for allowed operations and message routing.
- **Media pipeline** -- Attachments and media are handled through a unified pipeline.
- **Pairing manager** -- Links channel identities to Zeus sessions.

## Using Channels

Channels are available through two interfaces:

### Message Tool

The `message` tool in the agent loop sends messages to configured channels:

```json
{"channel": "telegram", "to": "chat_id", "text": "Hello from Zeus"}
```

### Gateway Daemon

The gateway daemon (`zeus gateway`) activates all configured channel adapters, enabling Zeus to receive and respond to inbound messages:

```bash
zeus gateway                              # Full daemon with all channels
zeus gateway --no-channels                # Daemon without channel adapters
```

## Channel API

Channels can also be managed via the REST API:

```
GET    /v1/channels              # List configured channels
POST   /v1/channels              # Create a channel
GET    /v1/channels/:id          # Get channel details
PUT    /v1/channels/:id          # Update channel config
DELETE /v1/channels/:id          # Delete a channel
POST   /v1/channels/:id/test     # Test channel connectivity
GET    /v1/channels/:id/status   # Channel status
```
