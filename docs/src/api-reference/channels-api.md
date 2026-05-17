# Channels

Manage messaging channels for multi-platform communication. Channels connect Zeus to external messaging platforms (Telegram, Discord, Slack, Email, iMessage, WhatsApp, Signal, Matrix) and custom webhooks.

## Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/v1/channels` | List channels |
| `POST` | `/v1/channels` | Create channel |
| `GET` | `/v1/channels/:id` | Get channel details |
| `PUT` | `/v1/channels/:id` | Update channel config |
| `DELETE` | `/v1/channels/:id` | Delete channel |
| `POST` | `/v1/channels/:id/test` | Test channel connectivity |
| `GET` | `/v1/channels/:id/status` | Channel status |

---

## GET `/v1/channels`

List all configured channels with their ID, type, name, and status.

**Response** `200 OK`

```json
{
  "channels": [
    {
      "id": "a1b2c3d4-...",
      "channel_type": "telegram",
      "name": "My Telegram",
      "enabled": true,
      "status": "connected",
      "last_message_at": "2026-02-11T10:30:00Z"
    },
    {
      "id": "b2c3d4e5-...",
      "channel_type": "discord",
      "name": "Dev Server",
      "enabled": true,
      "status": "connected",
      "last_message_at": null
    }
  ]
}
```

---

## POST `/v1/channels`

Create a new channel. The required config fields depend on the channel type.

**Request Body**

```json
{
  "channel_type": "telegram",
  "name": "My Telegram Bot",
  "config": {
    "api_id": "12345",
    "api_hash": "abc123...",
    "phone": "+1234567890"
  }
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `channel_type` | string | Yes | One of: `telegram`, `discord`, `slack`, `email`, `imessage`, `whatsapp`, `signal`, `matrix`, `webhook` |
| `name` | string | Yes | Display name for the channel |
| `config` | object | Yes | Channel-specific configuration (see below) |
| `enabled` | boolean | No | Whether the channel is active (default: `true`) |

### Required Config by Channel Type

| Type | Required Config Keys |
|------|---------------------|
| `telegram` | `api_id`, `api_hash`, `phone` |
| `discord` | `token` |
| `slack` | `bot_token`, `app_token` |
| `email` | `smtp_host`, `imap_host`, `username`, `password` |
| `imessage` | (none -- uses local AppleScript bridge, macOS only) |
| `whatsapp` | `phone_number_id`, `access_token` |
| `signal` | `phone` |
| `matrix` | `homeserver`, `user`, `password` |
| `webhook` | `webhook_url` |

**Response** `201 Created`

```json
{
  "id": "c3d4e5f6-...",
  "channel_type": "telegram",
  "name": "My Telegram Bot",
  "enabled": true,
  "created_at": "2026-02-11T10:00:00Z"
}
```

---

## GET `/v1/channels/:id`

Get the full details for a specific channel, including its configuration (secrets are masked).

**Response** `200 OK`

```json
{
  "id": "a1b2c3d4-...",
  "channel_type": "telegram",
  "name": "My Telegram Bot",
  "config": {
    "api_id": "12345",
    "api_hash": "abc1****",
    "phone": "+1234567890"
  },
  "enabled": true,
  "status": "connected",
  "created_at": "2026-02-11T10:00:00Z",
  "last_message_at": "2026-02-11T10:30:00Z"
}
```

---

## PUT `/v1/channels/:id`

Update a channel's configuration. Supports partial updates.

**Request Body**

```json
{
  "name": "Updated Name",
  "enabled": false,
  "config": {
    "phone": "+9876543210"
  }
}
```

**Response** `200 OK`

```json
{
  "id": "a1b2c3d4-...",
  "channel_type": "telegram",
  "name": "Updated Name",
  "enabled": false
}
```

---

## DELETE `/v1/channels/:id`

Delete a channel. Disconnects the adapter if currently running.

**Response** `204 No Content`

---

## POST `/v1/channels/:id/test`

Test connectivity for a channel. Validates required config keys and attempts to establish a connection to the platform.

**Response** `200 OK`

```json
{
  "status": "ok",
  "message": "Successfully connected to Telegram"
}
```

**Response** `400 Bad Request` -- missing or invalid configuration:

```json
{
  "error": "Missing required config key: api_hash"
}
```

**Response** `502 Bad Gateway` -- connection failed:

```json
{
  "error": "Failed to connect to Telegram: authentication error"
}
```

---

## GET `/v1/channels/:id/status`

Get the runtime status of a channel.

**Response** `200 OK`

```json
{
  "id": "a1b2c3d4-...",
  "channel_type": "telegram",
  "status": "connected",
  "uptime_seconds": 7200,
  "messages_sent": 42,
  "messages_received": 38,
  "last_error": null
}
```

| Status | Description |
|--------|-------------|
| `connected` | Channel is active and connected |
| `disconnected` | Channel is configured but not connected |
| `error` | Channel encountered a connection error |
| `disabled` | Channel is disabled by configuration |
