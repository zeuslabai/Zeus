# Memory & Knowledge

Manage the workspace file-based memory, search across memories with full-text and semantic search, and access context journals.

## Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/v1/memory` | Get workspace context |
| `POST` | `/v1/memory/remember` | Add fact to MEMORY.md |
| `POST` | `/v1/memory/note` | Add to daily note |
| `GET` | `/v1/memory/files` | List workspace files |
| `GET` | `/v1/memory/files/*path` | Read specific file |
| `PUT` | `/v1/memory/files/*path` | Write specific file |
| `DELETE` | `/v1/memory/files/*path` | Delete file |
| `POST` | `/v1/memory/search` | Search memory |
| `POST` | `/v1/memory/sync` | Sync with Mnemosyne |
| `GET` | `/v1/memory/timeline` | Memory timeline |
| `GET` | `/v1/context/journals` | List context journals |

---

## GET `/v1/memory`

Get the full workspace context including system prompt (AGENTS.md), personality (SOUL.md), user context (USER.md), and long-term memory (MEMORY.md).

**Response** `200 OK`

```json
{
  "agents": "You are Zeus, an autonomous AI assistant...",
  "soul": "Personality traits and communication style...",
  "user": "User preferences and context...",
  "memory": "- Project uses Rust workspace with 20 crates\n- Preferred editor: Neovim\n...",
  "heartbeat": "Proactive task list..."
}
```

---

## POST `/v1/memory/remember`

Add a fact to the long-term memory file (`~/.zeus/workspace/memory/MEMORY.md`).

**Request Body**

```json
{
  "fact": "The production database runs on PostgreSQL 16"
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `fact` | string | Yes | The fact to remember |

**Response** `200 OK`

```json
{
  "status": "ok",
  "message": "Fact added to memory"
}
```

---

## POST `/v1/memory/note`

Add content to today's daily note (`~/.zeus/workspace/daily/YYYY-MM-DD.md`).

**Request Body**

```json
{
  "content": "Deployed v1.2.0 to staging environment"
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `content` | string | Yes | The note content to add |

**Response** `200 OK`

```json
{
  "status": "ok",
  "date": "2026-02-11"
}
```

---

## GET `/v1/memory/files`

List all files in the workspace directory.

**Response** `200 OK`

```json
{
  "files": [
    "AGENTS.md",
    "SOUL.md",
    "USER.md",
    "HEARTBEAT.md",
    "memory/MEMORY.md",
    "daily/2026-02-10.md",
    "daily/2026-02-11.md"
  ]
}
```

---

## GET `/v1/memory/files/*path`

Read the contents of a specific workspace file.

**Example:** `GET /v1/memory/files/memory/MEMORY.md`

**Response** `200 OK`

```json
{
  "path": "memory/MEMORY.md",
  "content": "# Long-Term Memory\n\n- Project uses Rust workspace...\n"
}
```

**Response** `404 Not Found` if the file does not exist.

---

## PUT `/v1/memory/files/*path`

Write content to a specific workspace file. Creates the file if it does not exist, overwrites if it does.

**Example:** `PUT /v1/memory/files/notes/project-plan.md`

**Request Body**

```json
{
  "content": "# Project Plan\n\n## Phase 1\n..."
}
```

**Response** `200 OK`

```json
{
  "status": "ok",
  "path": "notes/project-plan.md"
}
```

---

## DELETE `/v1/memory/files/*path`

Delete a specific workspace file.

**Example:** `DELETE /v1/memory/files/notes/old-note.md`

**Response** `204 No Content`

**Response** `404 Not Found` if the file does not exist.

---

## POST `/v1/memory/search`

Search across memory using text search (FTS5) and optional semantic search (vector embeddings). Requires Mnemosyne to be configured for full functionality.

**Request Body**

```json
{
  "query": "database configuration",
  "limit": 10
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `query` | string | Yes | Search query |
| `limit` | integer | No | Maximum results (default: 10) |

**Response** `200 OK`

```json
{
  "results": [
    {
      "content": "The production database runs on PostgreSQL 16",
      "source": "memory",
      "score": 0.92,
      "timestamp": "2026-02-10T12:00:00Z"
    },
    {
      "content": "Database connection string is stored in .env",
      "source": "session",
      "score": 0.78,
      "timestamp": "2026-02-09T16:30:00Z"
    }
  ]
}
```

Search uses hybrid ranking: BM25 full-text score weighted with cosine similarity from vector embeddings when available.

---

## POST `/v1/memory/sync`

Trigger a sync between the workspace file-based memory and the Mnemosyne SQLite database. Useful after manual edits to workspace files.

**Response** `200 OK`

```json
{
  "status": "ok",
  "synced_entries": 14
}
```

---

## GET `/v1/memory/timeline`

Get a chronological timeline of memory entries, showing when facts and notes were added.

**Response** `200 OK`

```json
{
  "entries": [
    {
      "timestamp": "2026-02-11T10:00:00Z",
      "type": "fact",
      "content": "The production database runs on PostgreSQL 16"
    },
    {
      "timestamp": "2026-02-11T09:30:00Z",
      "type": "daily_note",
      "content": "Deployed v1.2.0 to staging"
    }
  ]
}
```

---

## GET `/v1/context/journals`

List context journal files. Context journals capture structured workflow state (active task, modified files, tool call counts, decisions, next steps, blockers) before context compaction occurs.

**Response** `200 OK`

```json
{
  "journals": [
    {
      "filename": "2026-02-11T10-00-00.md",
      "path": "~/.zeus/context-journals/2026-02-11T10-00-00.md",
      "created_at": "2026-02-11T10:00:00Z"
    }
  ]
}
```

Context journals are written automatically by the agent loop when compaction thresholds are reached. They are deterministic (no LLM call) and stored as markdown files.
