# Slack

Zeus connects to Slack using the **Web API** (via reqwest) for sending messages and **Socket Mode** (via tokio-tungstenite) for receiving events in real time. Socket Mode avoids the need for a public URL or webhook endpoint.

## Configuration

Add the following to your `~/.zeus/config.toml`:

```toml
[channels.slack]
bot_token = "xoxb-..."
app_token = "xapp-..."
```

| Field | Description |
|-------|-------------|
| `bot_token` | Slack bot token (starts with `xoxb-`) |
| `app_token` | Slack app-level token for Socket Mode (starts with `xapp-`) |

## Setup

### 1. Create a Slack App

1. Go to [https://api.slack.com/apps](https://api.slack.com/apps) and click **Create New App**.
2. Choose **From scratch** and select the workspace to install it in.

### 2. Enable Socket Mode

1. In your app settings, navigate to **Socket Mode** in the left sidebar.
2. Toggle **Enable Socket Mode** on.
3. Create an app-level token with the `connections:write` scope.
4. Copy this token -- this is your `app_token` (starts with `xapp-`).

### 3. Configure Bot Permissions

1. Navigate to **OAuth & Permissions** in the left sidebar.
2. Under **Scopes**, add the following Bot Token Scopes:
   - `chat:write` -- Send messages
   - `channels:read` -- View channel info
   - `channels:history` -- Read messages in channels
   - `im:read` -- View DM info
   - `im:history` -- Read DMs
   - `im:write` -- Send DMs

### 4. Subscribe to Events

1. Navigate to **Event Subscriptions** in the left sidebar.
2. Toggle **Enable Events** on.
3. Under **Subscribe to bot events**, add:
   - `message.channels` -- Messages in public channels
   - `message.im` -- Direct messages

### 5. Install the App

1. Navigate to **Install App** in the left sidebar.
2. Click **Install to Workspace** and authorize.
3. Copy the **Bot User OAuth Token** -- this is your `bot_token` (starts with `xoxb-`).

### 6. Configure Zeus

Add both tokens to your `config.toml` as shown above.

## Features

- Real-time event receiving via Socket Mode (no public URL needed)
- Send messages to channels and DMs via the Web API
- Automatic message chunking for long responses
- No webhook server required

## Limitations

- Socket Mode is required -- Zeus does not support Slack's HTTP webhook mode.
- The bot must be invited to channels before it can receive messages there (use `/invite @your-bot`).
