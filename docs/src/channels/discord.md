# Discord

Zeus connects to Discord using the **serenity** library, which provides both gateway (WebSocket) and HTTP API support. This enables Zeus to operate as a Discord bot that can receive messages in real time and respond in any channel or DM.

## Configuration

Add the following to your `~/.zeus/config.toml`:

```toml
[channels.discord]
token = "your_bot_token"
```

| Field | Description |
|-------|-------------|
| `token` | Discord bot token |

## Setup

### 1. Create a Discord Application

1. Go to the [Discord Developer Portal](https://discord.com/developers/applications).
2. Click **New Application** and give it a name.
3. Navigate to the **Bot** section in the left sidebar.
4. Click **Add Bot** (or **Reset Token** if one already exists).
5. Copy the bot token -- this is your `token` value.

### 2. Configure Bot Permissions

Under the **Bot** section:

- Enable **Message Content Intent** (required to read message text).
- Enable **Server Members Intent** if you need member information.
- Enable **Presence Intent** if you need online status.

### 3. Invite the Bot to Your Server

1. Go to the **OAuth2** section, then **URL Generator**.
2. Select the `bot` scope.
3. Select the permissions your bot needs (at minimum: Send Messages, Read Message History, View Channels).
4. Copy the generated URL and open it in your browser to invite the bot.

### 4. Configure Zeus

Add the bot token to your `config.toml` as shown above.

## Features

- Real-time message receiving via the Discord gateway (WebSocket)
- Send messages to any channel or DM the bot has access to
- HTTP API for additional Discord operations
- Automatic message chunking for responses exceeding Discord's 2000-character limit

## Limitations

- The bot can only operate in servers it has been invited to.
- Message Content Intent must be enabled to read message text.
- Rate limits are handled automatically by serenity.
