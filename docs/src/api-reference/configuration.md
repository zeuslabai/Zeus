# Configuration & Auth

Manage Zeus configuration, test provider connections, and handle OAuth authentication.

## Endpoints

### Configuration

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/v1/config` | Get config (sanitized) |
| `PUT` | `/v1/config` | Update config |
| `POST` | `/v1/config/test` | Test provider connection |
| `GET` | `/v1/config/providers` | Get provider configuration |
| `POST` | `/v1/config/reload` | Reload config from disk |
| `GET` | `/v1/config/history` | Last 10 changes |

### Authentication

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/v1/auth/login` | OAuth login |
| `GET` | `/v1/auth/status` | Auth status |
| `POST` | `/v1/auth/token` | Token refresh |
| `POST` | `/v1/auth/logout` | Logout |

---

## GET `/v1/config`

Get the current configuration. API keys and secrets are sanitized (masked) in the response.

**Response** `200 OK`

```json
{
  "model": "anthropic/claude-sonnet-4-20250514",
  "workspace": "~/.zeus/workspace",
  "sessions": "~/.zeus/sessions",
  "max_iterations": 20,
  "tui": {
    "theme": "dark",
    "vim_mode": false
  },
  "auth": {
    "use_oauth": false
  },
  "ollama": {
    "url": "http://localhost:11434"
  },
  "mnemosyne": {
    "db_path": "~/.zeus/mnemosyne.db",
    "enable_fts": true
  }
}
```

---

## PUT `/v1/config`

Update configuration values. Only safe fields can be modified through the API -- sensitive fields like API keys must be set via environment variables or direct config file editing.

**Request Body**

```json
{
  "model": "openai/gpt-4o",
  "max_iterations": 30,
  "tui": {
    "vim_mode": true
  }
}
```

**Updatable Fields**

| Field | Type | Description |
|-------|------|-------------|
| `model` | string | Model identifier (`provider/model-name`) |
| `max_iterations` | integer | Maximum agent loop iterations |
| `tui.theme` | string | TUI color theme |
| `tui.vim_mode` | boolean | Enable vim keybindings in TUI |
| `ollama.url` | string | Ollama server URL |
| `mnemosyne.*` | various | Mnemosyne memory settings |
| `athena.*` | various | Athena documentation settings |
| `aegis.level` | string | Security level |
| `nous.*` | various | Cognitive engine settings |

**Response** `200 OK`

```json
{
  "status": "ok",
  "updated_fields": ["model", "max_iterations", "tui.vim_mode"]
}
```

---

## POST `/v1/config/test`

Test the connection to the currently configured LLM provider. Sends a minimal request to verify that credentials and connectivity are working.

**Response** `200 OK`

```json
{
  "status": "ok",
  "provider": "anthropic",
  "model": "claude-sonnet-4-20250514",
  "latency_ms": 340
}
```

**Response** `502 Bad Gateway`

```json
{
  "error": "Provider connection failed: invalid API key"
}
```

---

## GET `/v1/config/providers`

Get information about the configured LLM provider, including the model format and available environment variables.

**Response** `200 OK`

```json
{
  "current_provider": "anthropic",
  "current_model": "claude-sonnet-4-20250514",
  "model_string": "anthropic/claude-sonnet-4-20250514",
  "api_key_set": true,
  "available_providers": [
    { "name": "anthropic", "env_var": "ANTHROPIC_API_KEY", "configured": true },
    { "name": "openai", "env_var": "OPENAI_API_KEY", "configured": false },
    { "name": "ollama", "env_var": null, "configured": true },
    { "name": "openrouter", "env_var": "OPENROUTER_API_KEY", "configured": false }
  ]
}
```

---

## POST `/v1/config/reload`

Reload configuration from disk (`~/.zeus/config.toml`). Useful after manually editing the config file.

**Response** `200 OK`

```json
{
  "status": "ok",
  "message": "Configuration reloaded"
}
```

---

## GET `/v1/config/history`

Get the last 10 configuration changes with timestamps and the fields that were modified.

**Response** `200 OK`

```json
{
  "changes": [
    {
      "timestamp": "2026-02-11T10:30:00Z",
      "fields": ["model"],
      "old_values": { "model": "anthropic/claude-sonnet-4-20250514" },
      "new_values": { "model": "openai/gpt-4o" }
    },
    {
      "timestamp": "2026-02-10T15:00:00Z",
      "fields": ["max_iterations"],
      "old_values": { "max_iterations": 20 },
      "new_values": { "max_iterations": 30 }
    }
  ]
}
```

---

## Authentication

Zeus supports optional OAuth authentication. When `auth.use_oauth = true` in the config, all API endpoints require a valid Bearer token.

### POST `/v1/auth/login`

Initiate OAuth login. Returns a URL for the user to authenticate.

**Request Body**

```json
{
  "provider": "github"
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `provider` | string | No | OAuth provider (default: configured provider) |

**Response** `200 OK`

```json
{
  "auth_url": "https://github.com/login/oauth/authorize?client_id=...",
  "state": "random-state-token"
}
```

---

### GET `/v1/auth/status`

Check the current authentication status.

**Response** `200 OK`

```json
{
  "authenticated": true,
  "user": "mike",
  "provider": "github",
  "expires_at": "2026-02-12T10:00:00Z"
}
```

**Response** `200 OK` (not authenticated)

```json
{
  "authenticated": false,
  "auth_required": true
}
```

---

### POST `/v1/auth/token`

Refresh an expiring authentication token.

**Request Body**

```json
{
  "refresh_token": "rt-abc123..."
}
```

**Response** `200 OK`

```json
{
  "access_token": "at-xyz789...",
  "expires_at": "2026-02-12T10:00:00Z"
}
```

---

### POST `/v1/auth/logout`

Invalidate the current authentication session.

**Response** `200 OK`

```json
{
  "status": "ok",
  "message": "Logged out"
}
```
