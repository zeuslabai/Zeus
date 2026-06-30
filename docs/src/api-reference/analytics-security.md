# Analytics & Security

Monitor usage, costs, and token consumption. Manage security permissions, threat logs, and tool execution approvals.

## Analytics Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/v1/analytics/costs` | Cost aggregation |
| `GET` | `/v1/analytics/tokens` | Token usage breakdown |
| `GET` | `/v1/analytics/providers` | Per-provider costs |
| `GET` | `/v1/analytics/budgets` | Budget thresholds |
| `GET` | `/v1/pipeline/stats` | Pipeline stage metrics |
| `GET` | `/v1/activity` | Activity feed |

## Security Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/v1/security/threats` | Threat log |
| `GET` | `/v1/security/permissions` | Permission matrix |
| `PUT` | `/v1/security/permissions` | Update permissions |
| `GET` | `/v1/security/keys` | API key inventory |
| `GET` | `/v1/security/allowlist` | Shell command allowlist |
| `PUT` | `/v1/security/allowlist` | Update allowlist |
| `GET` | `/v1/approvals` | List pending approvals |
| `POST` | `/v1/approvals/:id/approve` | Approve tool execution |
| `POST` | `/v1/approvals/:id/deny` | Deny tool execution |

---

## Analytics

### GET `/v1/analytics/costs`

Aggregate cost data across all sessions and providers.

**Response** `200 OK`

```json
{
  "total_cost": 12.45,
  "currency": "USD",
  "period": {
    "start": "2026-02-01T00:00:00Z",
    "end": "2026-02-11T23:59:59Z"
  },
  "by_day": [
    { "date": "2026-02-11", "cost": 1.23 },
    { "date": "2026-02-10", "cost": 2.05 }
  ]
}
```

---

### GET `/v1/analytics/tokens`

Token usage breakdown by role and model.

**Response** `200 OK`

```json
{
  "total_tokens": 524000,
  "prompt_tokens": 312000,
  "completion_tokens": 212000,
  "by_model": {
    "anthropic/claude-sonnet-4-20250514": {
      "prompt_tokens": 280000,
      "completion_tokens": 190000
    },
    "ollama/llama3.2": {
      "prompt_tokens": 32000,
      "completion_tokens": 22000
    }
  }
}
```

---

### GET `/v1/analytics/providers`

Per-provider cost breakdown.

**Response** `200 OK`

```json
{
  "providers": [
    {
      "name": "anthropic",
      "total_cost": 10.20,
      "total_tokens": 480000,
      "request_count": 156
    },
    {
      "name": "ollama",
      "total_cost": 0.00,
      "total_tokens": 44000,
      "request_count": 32
    }
  ]
}
```

---

### GET `/v1/analytics/budgets`

Get budget thresholds and current spend against them.

**Response** `200 OK`

```json
{
  "daily_budget": 5.00,
  "daily_spent": 1.23,
  "monthly_budget": 100.00,
  "monthly_spent": 12.45,
  "alerts": []
}
```

When spend exceeds a threshold, an alert is included:

```json
{
  "alerts": [
    {
      "level": "warning",
      "message": "Daily spend at 80% of budget",
      "threshold": 4.00,
      "current": 4.10
    }
  ]
}
```

---

### GET `/v1/pipeline/stats`

Metrics for each pipeline stage in the agent processing pipeline.

**Response** `200 OK`

```json
{
  "stages": {
    "input_processing": { "count": 200, "avg_ms": 2 },
    "context_building": { "count": 200, "avg_ms": 15 },
    "llm_call": { "count": 200, "avg_ms": 2400 },
    "tool_execution": { "count": 85, "avg_ms": 340 },
    "response_assembly": { "count": 200, "avg_ms": 1 }
  }
}
```

---

### GET `/v1/activity`

Activity feed showing recent actions across the system.

**Query Parameters**

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `limit` | integer | 50 | Maximum number of entries to return |

**Response** `200 OK`

```json
{
  "activities": [
    {
      "timestamp": "2026-02-11T11:30:00Z",
      "type": "chat",
      "summary": "User asked about project structure",
      "session_id": "a1b2c3d4-..."
    },
    {
      "timestamp": "2026-02-11T11:28:00Z",
      "type": "tool_call",
      "summary": "Executed shell: cargo test",
      "session_id": "a1b2c3d4-..."
    }
  ]
}
```

---

## Security

### GET `/v1/security/threats`

View the threat log. Records blocked commands, denied URL fetches, path violations, and other security events.

**Response** `200 OK`

```json
{
  "threats": [
    {
      "timestamp": "2026-02-11T10:15:00Z",
      "type": "command_blocked",
      "detail": "Blocked shell command: rm -rf /",
      "source": "agent_loop",
      "session_id": "a1b2c3d4-..."
    },
    {
      "timestamp": "2026-02-10T16:00:00Z",
      "type": "url_blocked",
      "detail": "URL not in allowlist: http://malicious.example.com",
      "source": "web_fetch"
    }
  ]
}
```

---

### GET `/v1/security/permissions`

Get the current permission matrix defining what the agent is allowed to do.

**Response** `200 OK`

```json
{
  "level": "standard",
  "shell": {
    "enabled": true,
    "blocked_commands": ["rm -rf /", "dd if=", "mkfs"],
    "require_approval": ["sudo *", "docker rm *"]
  },
  "filesystem": {
    "read_paths": ["~", "/tmp"],
    "write_paths": ["~/.zeus", "/tmp"],
    "blocked_paths": ["/etc", "/usr"]
  },
  "network": {
    "enabled": true,
    "url_allowlist": ["*"]
  }
}
```

---

### PUT `/v1/security/permissions`

Update the permission matrix.

**Request Body**

```json
{
  "level": "strict",
  "shell": {
    "require_approval": ["sudo *", "docker *", "kubectl *"]
  },
  "filesystem": {
    "write_paths": ["~/.zeus"]
  }
}
```

**Response** `200 OK`

```json
{
  "status": "ok",
  "updated_fields": ["level", "shell.require_approval", "filesystem.write_paths"]
}
```

---

### GET `/v1/security/keys`

API key inventory showing which keys are configured. Key values are never returned.

**Response** `200 OK`

```json
{
  "keys": [
    { "name": "ANTHROPIC_API_KEY", "set": true, "source": "environment" },
    { "name": "OPENAI_API_KEY", "set": false, "source": null },
    { "name": "OPENROUTER_API_KEY", "set": true, "source": "config" },
    { "name": "GOOGLE_API_KEY", "set": false, "source": null }
  ]
}
```

---

### GET `/v1/security/allowlist`

Get the shell command allowlist. Commands matching these patterns can execute without approval.

**Response** `200 OK`

```json
{
  "patterns": [
    "ls *",
    "cat *",
    "cargo *",
    "git *",
    "grep *",
    "find *"
  ]
}
```

---

### PUT `/v1/security/allowlist`

Update the shell command allowlist.

**Request Body**

```json
{
  "patterns": [
    "ls *",
    "cat *",
    "cargo *",
    "git *",
    "grep *",
    "find *",
    "npm *",
    "node *"
  ]
}
```

**Response** `200 OK`

```json
{
  "status": "ok",
  "pattern_count": 8
}
```

---

### GET `/v1/approvals`

List pending tool execution approvals. When Aegis requires approval for a sensitive operation, it is queued here until approved or denied.

**Response** `200 OK`

```json
{
  "approvals": [
    {
      "id": "apr-a1b2c3d4-...",
      "tool": "shell",
      "arguments": { "command": "sudo systemctl restart nginx" },
      "reason": "Command matches require_approval pattern: sudo *",
      "requested_at": "2026-02-11T11:00:00Z",
      "session_id": "a1b2c3d4-..."
    }
  ]
}
```

---

### POST `/v1/approvals/:id/approve`

Approve a pending tool execution. The tool will be executed immediately.

**Response** `200 OK`

```json
{
  "status": "approved",
  "id": "apr-a1b2c3d4-...",
  "tool": "shell",
  "result": "nginx restarted successfully"
}
```

---

### POST `/v1/approvals/:id/deny`

Deny a pending tool execution. Optionally provide a reason.

**Request Body** (optional)

```json
{
  "reason": "Not authorized to restart services in production"
}
```

**Response** `200 OK`

```json
{
  "status": "denied",
  "id": "apr-a1b2c3d4-...",
  "reason": "Not authorized to restart services in production"
}
```
