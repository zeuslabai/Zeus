# Health & Status

Basic health checks, diagnostics, and resource overview endpoints.

## Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/` | Health check |
| `GET` | `/health` | Health check |
| `GET` | `/v1/status` | Server status |
| `GET` | `/v1/doctor` | Run diagnostics |
| `GET` | `/v1/stats` | Resource overview |

---

## GET `/`

Root health check. Returns a simple confirmation that the server is running.

**Response** `200 OK`

```json
{
  "status": "ok",
  "version": "1.0.0"
}
```

---

## GET `/health`

Alias for the root health check.

**Response** `200 OK`

```json
{
  "status": "ok"
}
```

---

## GET `/v1/status`

Returns server status including the configured model, provider, authentication state, and active session count.

**Response** `200 OK`

```json
{
  "model": "anthropic/claude-sonnet-4-20250514",
  "provider": "anthropic",
  "auth_enabled": false,
  "sessions_count": 3,
  "uptime_seconds": 1842
}
```

---

## GET `/v1/doctor`

Runs a diagnostic check across configuration, workspace, credentials, and optional services (e.g., Ollama connectivity).

**Response** `200 OK`

```json
{
  "config": { "valid": true },
  "workspace": { "valid": true, "path": "~/.zeus/workspace" },
  "credentials": {
    "anthropic": true,
    "openai": false,
    "ollama": true
  },
  "ollama": { "reachable": true, "models": ["llama3.2"] }
}
```

---

## GET `/v1/stats`

Returns a resource overview including session count, available tools, and memory statistics.

**Response** `200 OK`

```json
{
  "sessions": 5,
  "tools": 212,
  "memory": {
    "facts": 14,
    "daily_notes": 7,
    "mnemosyne_entries": 342
  }
}
```
