# Sessions

Manage conversation sessions. Sessions persist as JSONL files and track messages, tool calls, and metadata.

## Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/v1/sessions` | List sessions |
| `POST` | `/v1/sessions` | Create new session |
| `GET` | `/v1/sessions/:id` | Get session messages |
| `DELETE` | `/v1/sessions/:id` | Delete session |
| `GET` | `/v1/sessions/:id/stats` | Session statistics |
| `GET` | `/v1/sessions/:id/replay` | Full session replay |
| `GET` | `/v1/sessions/:id/replay/:turn` | Single turn by index |
| `GET` | `/v1/sessions/:id/raw` | Raw JSONL data |
| `GET` | `/v1/sessions/:id/audit` | Audit trail |
| `GET` | `/v1/sessions/:id/tools` | Tool call chain |
| `POST` | `/v1/sessions/:id/branch` | Create branch |
| `GET` | `/v1/sessions/:id/branches` | List branches |

---

## GET `/v1/sessions`

List all sessions with pagination.

**Query Parameters**

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `limit` | integer | 20 | Maximum number of sessions to return |
| `offset` | integer | 0 | Number of sessions to skip |

**Response** `200 OK`

```json
{
  "sessions": [
    {
      "id": "a1b2c3d4-...",
      "created_at": "2026-02-10T14:30:00Z",
      "updated_at": "2026-02-10T15:12:00Z",
      "message_count": 12
    }
  ],
  "total": 5
}
```

---

## POST `/v1/sessions`

Create a new empty session.

**Response** `201 Created`

```json
{
  "id": "b2c3d4e5-...",
  "created_at": "2026-02-11T10:00:00Z"
}
```

---

## GET `/v1/sessions/:id`

Retrieve all messages in a session.

**Response** `200 OK`

```json
{
  "id": "a1b2c3d4-...",
  "messages": [
    { "role": "user", "content": "List my files" },
    { "role": "assistant", "content": "Here are your files..." }
  ]
}
```

---

## DELETE `/v1/sessions/:id`

Delete a session and its JSONL file.

**Response** `204 No Content`

---

## GET `/v1/sessions/:id/stats`

Get statistics for a session including turn count, token usage, tools used, model, cost estimate, and duration.

**Response** `200 OK`

```json
{
  "id": "a1b2c3d4-...",
  "turns": 6,
  "total_tokens": 4820,
  "tools_used": ["list_dir", "read_file", "shell"],
  "model_used": "anthropic/claude-sonnet-4-20250514",
  "cost_estimate": 0.0234,
  "duration_ms": 18500
}
```

Token estimation uses a `chars / 4` heuristic consistent with the context manager.

---

## GET `/v1/sessions/:id/replay`

Full chronological replay of a session with per-entry metadata.

**Response** `200 OK`

```json
{
  "entries": [
    {
      "index": 0,
      "timestamp": "2026-02-10T14:30:00Z",
      "role": "user",
      "content": "List my files",
      "tool_calls": null,
      "tool_name": null,
      "tool_results": null,
      "thinking": null,
      "token_count": 12
    },
    {
      "index": 1,
      "timestamp": "2026-02-10T14:30:02Z",
      "role": "assistant",
      "content": "Here are your files...",
      "tool_calls": [
        { "tool": "list_dir", "arguments": { "path": "." } }
      ],
      "tool_name": null,
      "tool_results": null,
      "thinking": "The user wants to see their files. I should use list_dir.",
      "token_count": 156
    }
  ]
}
```

The `thinking` field is extracted from `<thinking>` tags in assistant content when present.

---

## GET `/v1/sessions/:id/replay/:turn`

Retrieve a single turn from the replay by 0-based index.

**Response** `200 OK`

```json
{
  "index": 1,
  "timestamp": "2026-02-10T14:30:02Z",
  "role": "assistant",
  "content": "Here are your files...",
  "tool_calls": [...],
  "thinking": "...",
  "token_count": 156
}
```

**Response** `404 Not Found` if the turn index is out of range.

---

## GET `/v1/sessions/:id/raw`

Return the raw JSONL session data as stored on disk.

**Response** `200 OK`

```
Content-Type: application/x-ndjson
```

Each line is a JSON object representing one session entry.

---

## GET `/v1/sessions/:id/audit`

Audit trail for a session showing tool calls and memory writes.

**Response** `200 OK`

```json
{
  "entries": [
    {
      "timestamp": "2026-02-10T14:30:02Z",
      "action": "tool_call",
      "tool": "shell",
      "arguments": { "command": "ls -la" },
      "result_summary": "Listed 12 files"
    },
    {
      "timestamp": "2026-02-10T14:30:05Z",
      "action": "memory_write",
      "file": "memory/MEMORY.md",
      "content_summary": "Added fact about project structure"
    }
  ]
}
```

---

## GET `/v1/sessions/:id/tools`

Tool call chain for the session, useful for building execution graphs.

**Response** `200 OK`

```json
{
  "tool_calls": [
    {
      "index": 0,
      "tool": "list_dir",
      "arguments": { "path": "." },
      "duration_ms": 12,
      "success": true
    },
    {
      "index": 1,
      "tool": "read_file",
      "arguments": { "path": "Cargo.toml" },
      "duration_ms": 5,
      "success": true
    }
  ]
}
```

---

## POST `/v1/sessions/:id/branch`

Create a branch from a session at a specific point, allowing alternate conversation paths.

**Request Body**

```json
{
  "at_turn": 4,
  "name": "explore-alternative"
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `at_turn` | integer | No | Turn index to branch from (defaults to latest) |
| `name` | string | No | Optional branch label |

**Response** `201 Created`

```json
{
  "id": "c3d4e5f6-...",
  "parent_id": "a1b2c3d4-...",
  "branched_at_turn": 4
}
```

---

## GET `/v1/sessions/:id/branches`

List all branches created from a session.

**Response** `200 OK`

```json
{
  "branches": [
    {
      "id": "c3d4e5f6-...",
      "name": "explore-alternative",
      "branched_at_turn": 4,
      "created_at": "2026-02-11T09:00:00Z"
    }
  ]
}
```
