# Webhooks

Receive inbound webhooks from external services and register outbound webhooks to notify external systems of Zeus events.

## Endpoints

### Inbound

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/v1/webhooks` | Health check |
| `POST` | `/v1/webhooks` | Generic webhook receiver |
| `POST` | `/v1/webhooks/:source` | Source-specific receiver |

### Outbound

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/v1/webhooks/outbound` | List outbound registrations |
| `POST` | `/v1/webhooks/outbound` | Register outbound webhook |
| `DELETE` | `/v1/webhooks/outbound/:id` | Delete outbound webhook |

---

## Inbound Webhooks

### GET `/v1/webhooks`

Health check for the webhook subsystem.

**Response** `200 OK`

```json
{
  "status": "ok",
  "registered_sources": ["github", "stripe"]
}
```

---

### POST `/v1/webhooks`

Generic webhook receiver. Accepts any JSON payload and routes it through the agent for processing.

**Request Body**

Any valid JSON object:

```json
{
  "event": "build_complete",
  "project": "zeus",
  "status": "success",
  "commit": "abc123"
}
```

**Response** `200 OK`

```json
{
  "status": "received",
  "id": "wh-a1b2c3d4-..."
}
```

---

### POST `/v1/webhooks/:source`

Source-specific webhook receiver. The `:source` parameter identifies the origin system, enabling specialized parsing and routing.

**Path Parameters**

| Parameter | Type | Description |
|-----------|------|-------------|
| `source` | string | Source identifier (e.g., `github`, `stripe`, `linear`) |

**Example:** `POST /v1/webhooks/github`

**Request Body**

The payload format depends on the source:

```json
{
  "action": "opened",
  "pull_request": {
    "number": 42,
    "title": "Add new feature"
  }
}
```

**Response** `200 OK`

```json
{
  "status": "received",
  "source": "github",
  "id": "wh-b2c3d4e5-..."
}
```

---

## Outbound Webhooks

Register URLs to receive notifications when events occur in Zeus.

### GET `/v1/webhooks/outbound`

List all registered outbound webhooks.

**Response** `200 OK`

```json
{
  "webhooks": [
    {
      "id": "owh-a1b2c3d4-...",
      "url": "https://example.com/zeus-events",
      "events": ["tool_call", "message"],
      "secret": true,
      "created_at": "2026-02-11T10:00:00Z",
      "last_delivery_at": "2026-02-11T11:30:00Z",
      "delivery_count": 42,
      "failure_count": 1
    }
  ]
}
```

The `secret` field indicates whether an HMAC secret is configured (the actual value is never returned).

---

### POST `/v1/webhooks/outbound`

Register a new outbound webhook.

**Request Body**

```json
{
  "url": "https://example.com/zeus-events",
  "events": ["tool_call", "error", "task_complete"],
  "secret": "my-hmac-secret"
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `url` | string | Yes | The URL to deliver events to |
| `events` | array | Yes | List of event types to subscribe to |
| `secret` | string | No | HMAC secret for payload signing |

### Event Types

| Event | Description |
|-------|-------------|
| `tool_call` | A tool was executed by the agent |
| `message` | A message was sent or received |
| `error` | An error occurred during processing |
| `task_complete` | A Prometheus task completed |

**Response** `201 Created`

```json
{
  "id": "owh-b2c3d4e5-...",
  "url": "https://example.com/zeus-events",
  "events": ["tool_call", "error", "task_complete"],
  "created_at": "2026-02-11T10:00:00Z"
}
```

### Delivery Format

Outbound webhook payloads are delivered as POST requests with the following structure:

```json
{
  "event": "tool_call",
  "timestamp": "2026-02-11T11:30:00Z",
  "data": {
    "tool": "shell",
    "arguments": { "command": "cargo test" },
    "result": "test result: ok. 42 passed",
    "success": true,
    "duration_ms": 5200
  }
}
```

When a secret is configured, the payload is signed with HMAC-SHA256. The signature is included in the `X-Zeus-Signature` header:

```
X-Zeus-Signature: sha256=abc123...
```

### Retry Policy

Failed deliveries (non-2xx response or timeout) are retried up to **3 times** with exponential backoff:

| Attempt | Delay |
|---------|-------|
| 1st retry | 5 seconds |
| 2nd retry | 25 seconds |
| 3rd retry | 125 seconds |

---

### DELETE `/v1/webhooks/outbound/:id`

Delete an outbound webhook registration. Stops all future deliveries.

**Response** `204 No Content`
