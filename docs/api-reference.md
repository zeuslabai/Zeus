# Zeus API Reference

Zeus exposes **200+ REST API endpoints** through its HTTP gateway. This reference covers the most-used endpoints with curl examples and request/response JSON.

**Base URL**: `http://localhost:8080` (configure with `zeus serve -p <port>`)

All request/response bodies are JSON. Errors return `{"error": "description"}` with an appropriate HTTP status code.

---

## Authentication

Zeus supports optional bearer-token authentication. When the gateway starts with the `ZEUS_API_TOKEN` environment variable set, every request — except a small allowlist of bootstrap endpoints — must carry a matching `Authorization` header or it will be rejected with `401 Unauthorized`.

### Enabling auth

```bash
export ZEUS_API_TOKEN="$(openssl rand -hex 32)"
zeus gateway --host 0.0.0.0 --port 8080
```

If `ZEUS_API_TOKEN` is unset, the gateway runs in open mode — suitable for `localhost` development, not for any exposed deployment.

### Authenticating REST requests

Pass the token as a Bearer credential:

```bash
curl http://localhost:8080/v1/status \
  -H "Authorization: Bearer $ZEUS_API_TOKEN"
```

Every `/v1/*` endpoint covered in this reference uses the same header. Omit it once the token is configured and you will get:

```json
{ "error": "unauthorized" }
```
(HTTP 401)

### Unauthenticated allowlist

These routes bypass the middleware even when a token is set, so clients can reach them during onboarding or health-check flows:

| Path | Purpose |
|------|---------|
| `GET /` | Root probe |
| `GET /health` | Load-balancer health check |
| `POST /v1/auth/*` | Login + credential exchange |
| `GET /v1/auth/*` | OAuth callbacks |
| `POST /v1/onboarding/*` | First-run bootstrap |
| `GET /v1/onboarding/*` | First-run bootstrap |

### Authenticating WebSocket upgrades

Browsers cannot attach custom headers to a `WebSocket` handshake, so Zeus accepts the bearer token as a query parameter on the upgrade URL:

```
ws://localhost:8080/v1/ws?token=<ZEUS_API_TOKEN>
```

Omitting `?token=` against an authed gateway returns `401 Unauthorized` during the upgrade and the socket never opens. The token value should be URL-encoded if it contains reserved characters.

Native clients (e.g. `websocat`, `wscat`) may also send the header directly:

```bash
websocat -H "Authorization: Bearer $ZEUS_API_TOKEN" ws://localhost:8080/v1/ws
```

### Rotating the token

Use the key-rotation endpoint to replace the live token without restarting the gateway. The old token remains valid for a configurable grace period (default 24 h) so in-flight clients can reconnect:

```bash
curl -X POST http://localhost:8080/v1/security/rotate-key \
  -H "Authorization: Bearer $ZEUS_API_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"new_key": "newsecret...", "reason": "scheduled rotation"}'
```

```json
{
  "success": true,
  "message": "Key rotated. Old key valid until 2026-04-12T18:00:00Z.",
  "grace_period_hours": 24,
  "new_key_hash": "sha256:..."
}
```

Check rotation status with `GET /v1/security/rotation-status`.

---

## Health & Status

### GET /health

Health check. Returns 200 if the server is running.

```bash
curl http://localhost:8080/health
```

```json
{ "status": "ok" }
```

### GET /v1/status

Server status with model, provider, session count, and uptime.

```bash
curl http://localhost:8080/v1/status
```

```json
{
  "model": "anthropic/claude-sonnet-4-20250514",
  "provider": "anthropic",
  "auth_enabled": false,
  "sessions_count": 12,
  "tools_count": 212,
  "uptime_seconds": 3600
}
```

### GET /v1/doctor

Run diagnostics (config, workspace, credentials, Ollama, sessions, permissions — 17 checks).

```bash
curl http://localhost:8080/v1/doctor
```

```json
{
  "checks": [
    { "name": "Config file", "status": "ok" },
    { "name": "Workspace directory", "status": "ok" },
    { "name": "API credentials", "status": "ok" },
    { "name": "Stale sessions", "status": "warn", "detail": "3 sessions older than 30 days" }
  ],
  "passed": 15,
  "warnings": 2,
  "failed": 0
}
```

---

## Chat

### POST /v1/chat

Send a message and receive the full response.

```bash
curl -X POST http://localhost:8080/v1/chat \
  -H "Content-Type: application/json" \
  -d '{"message": "List the files in my workspace"}'
```

```json
{
  "response": "Here are the files in your workspace:\n- AGENTS.md\n- SOUL.md\n- USER.md\n- memory/MEMORY.md",
  "session_id": "01JMXYZ...",
  "tool_calls": [
    {
      "tool": "list_dir",
      "arguments": { "path": "~/.zeus/workspace" },
      "result": "AGENTS.md\nSOUL.md\nUSER.md\nmemory/\ndaily/"
    }
  ]
}
```

### POST /v1/chat/completions

OpenAI-compatible ChatCompletion endpoint. Works with any OpenAI SDK client.

```bash
curl -X POST http://localhost:8080/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "anthropic/claude-sonnet-4-20250514",
    "messages": [
      {"role": "user", "content": "Hello!"}
    ],
    "stream": false
  }'
```

```json
{
  "id": "chatcmpl-abc123",
  "object": "chat.completion",
  "model": "anthropic/claude-sonnet-4-20250514",
  "choices": [{
    "index": 0,
    "message": { "role": "assistant", "content": "Hello! How can I help?" },
    "finish_reason": "stop"
  }],
  "usage": { "prompt_tokens": 12, "completion_tokens": 8, "total_tokens": 20 }
}
```

For streaming, set `"stream": true` — response is Server-Sent Events (SSE) with `data: {...}` chunks followed by `data: [DONE]`.

### GET /v1/models

List available models in OpenAI-compatible format.

```bash
curl http://localhost:8080/v1/models
```

```json
{
  "object": "list",
  "data": [
    { "id": "anthropic/claude-sonnet-4-20250514", "object": "model", "owned_by": "anthropic" }
  ]
}
```

---

## Sessions

### GET /v1/sessions

List sessions with pagination.

```bash
curl "http://localhost:8080/v1/sessions?limit=10&offset=0"
```

```json
{
  "sessions": [
    { "id": "01JMXYZ...", "created": "2026-02-24T10:00:00Z", "message_count": 14 }
  ],
  "total": 42
}
```

### POST /v1/sessions

Create a new session.

```bash
curl -X POST http://localhost:8080/v1/sessions
```

```json
{ "id": "01JMXYZ...", "created": "2026-02-24T12:00:00Z" }
```

### GET /v1/sessions/:id

Get all messages in a session.

```bash
curl http://localhost:8080/v1/sessions/01JMXYZ...
```

```json
{
  "id": "01JMXYZ...",
  "messages": [
    { "role": "user", "content": "Hello" },
    { "role": "assistant", "content": "Hi there!" }
  ]
}
```

### GET /v1/sessions/:id/stats

Session statistics — turns, tokens, tool calls, estimated cost.

```bash
curl http://localhost:8080/v1/sessions/01JMXYZ.../stats
```

```json
{
  "turns": 7,
  "total_tokens": 4200,
  "tool_calls": 3,
  "estimated_cost_usd": 0.042
}
```

---

## Tools

### GET /v1/tools

List all available tools with schemas.

```bash
curl http://localhost:8080/v1/tools
```

```json
{
  "tools": [
    {
      "name": "read_file",
      "description": "Read file contents",
      "parameters": {
        "type": "object",
        "properties": {
          "path": { "type": "string", "description": "File path to read" }
        },
        "required": ["path"]
      }
    }
  ],
  "count": 212
}
```

### POST /v1/tools/:name

Execute a tool by name.

```bash
curl -X POST http://localhost:8080/v1/tools/shell \
  -H "Content-Type: application/json" \
  -d '{"command": "uname -a"}'
```

```json
{
  "success": true,
  "output": "Darwin macbook.local 25.3.0 Darwin Kernel Version 25.3.0..."
}
```

### POST /v1/tokens/count

Count tokens in a string (useful for context window management).

```bash
curl -X POST http://localhost:8080/v1/tokens/count \
  -H "Content-Type: application/json" \
  -d '{"text": "Hello, how many tokens is this?"}'
```

```json
{ "tokens": 8 }
```

---

## Memory

### GET /v1/memory

Get the current workspace context (AGENTS.md, SOUL.md, USER.md, MEMORY.md).

```bash
curl http://localhost:8080/v1/memory
```

```json
{
  "agents": "You are Zeus, an autonomous AI assistant...",
  "soul": "Personality: helpful, precise, proactive...",
  "user": "Name: Mike\nPreferences: ...",
  "memory": "- Prefers Rust over Python\n- Uses Ollama locally"
}
```

### POST /v1/memory/remember

Add a fact to long-term memory.

```bash
curl -X POST http://localhost:8080/v1/memory/remember \
  -H "Content-Type: application/json" \
  -d '{"fact": "The production database is on port 5432"}'
```

```json
{ "status": "remembered" }
```

### POST /v1/memory/note

Add a note to today's daily journal.

```bash
curl -X POST http://localhost:8080/v1/memory/note \
  -H "Content-Type: application/json" \
  -d '{"content": "Deployed v2.1 to production"}'
```

```json
{ "status": "noted", "file": "daily/2026-02-24.md" }
```

### POST /v1/memory/search

Search memory using text or hybrid (FTS5 + vector) search.

```bash
curl -X POST http://localhost:8080/v1/memory/search \
  -H "Content-Type: application/json" \
  -d '{"query": "database configuration", "limit": 5}'
```

```json
{
  "results": [
    { "content": "The production database is on port 5432", "score": 0.92, "source": "memory" }
  ]
}
```

---

## Channels

### GET /v1/channels

List configured messaging channels.

```bash
curl http://localhost:8080/v1/channels
```

```json
{
  "channels": [
    { "id": "telegram-1", "type": "telegram", "name": "Main", "status": "connected" },
    { "id": "discord-1", "type": "discord", "name": "Zeus Bot", "status": "connected" }
  ]
}
```

### POST /v1/channels/:id/test

Test connectivity for a channel.

```bash
curl -X POST http://localhost:8080/v1/channels/telegram-1/test
```

```json
{ "status": "ok", "latency_ms": 120 }
```

---

## Skills

### GET /v1/skills

List installed skills with full metadata (category, tags, requirements, tools).

```bash
curl http://localhost:8080/v1/skills
```

```json
{
  "skills": [
    {
      "id": "git-assistant",
      "name": "Git Assistant",
      "description": "Advanced git operations and workflow management",
      "version": "1.0.0",
      "enabled": true,
      "category": "development",
      "tags": ["git", "vcs"],
      "tools_count": 3
    }
  ],
  "total": 52
}
```

### GET /v1/skills/search

Search and filter skills by text query and category.

```bash
curl "http://localhost:8080/v1/skills/search?q=email&category=messaging"
```

```json
{
  "skills": [
    { "id": "email-manager", "name": "Email Manager", "category": "messaging" }
  ],
  "total": 1
}
```

### GET /v1/skills/categories

List skill categories with counts.

```bash
curl http://localhost:8080/v1/skills/categories
```

```json
{
  "categories": [
    { "name": "development", "count": 15 },
    { "name": "messaging", "count": 8 },
    { "name": "infrastructure", "count": 6 }
  ]
}
```

---

## Agents

### GET /v1/agents

List agent definitions.

```bash
curl http://localhost:8080/v1/agents
```

```json
{
  "agents": [
    { "id": "default", "name": "Zeus", "model": "anthropic/claude-sonnet-4-20250514", "status": "active" }
  ]
}
```

### POST /v1/agents/spawn

Spawn an agent into the runtime registry.

```bash
curl -X POST http://localhost:8080/v1/agents/spawn \
  -H "Content-Type: application/json" \
  -d '{"agent_id": "researcher", "model": "anthropic/claude-sonnet-4-20250514"}'
```

```json
{ "id": "researcher", "status": "spawned" }
```

### POST /v1/agents/:id/send

Send a message to a spawned agent.

```bash
curl -X POST http://localhost:8080/v1/agents/researcher/send \
  -H "Content-Type: application/json" \
  -d '{"message": "Research the latest Rust async patterns"}'
```

```json
{
  "response": "Here are the key async patterns in Rust...",
  "tool_calls": []
}
```

---

## Cron Jobs

### GET /v1/cron/jobs

List all scheduled cron jobs.

```bash
curl http://localhost:8080/v1/cron/jobs
```

```json
{
  "jobs": [
    {
      "id": "daily-review",
      "name": "Daily review",
      "cron": "0 9 * * *",
      "enabled": true,
      "last_run": "2026-02-24T09:00:00Z"
    }
  ],
  "count": 3
}
```

### POST /v1/cron/jobs

Create a new cron job (manual or from template).

```bash
# Manual creation
curl -X POST http://localhost:8080/v1/cron/jobs \
  -H "Content-Type: application/json" \
  -d '{
    "name": "health-check",
    "cron": "every 5 minutes",
    "task_type": {"type": "shell", "command": "curl -s http://localhost:8080/health"},
    "enabled": true
  }'

# From template
curl -X POST http://localhost:8080/v1/cron/jobs \
  -H "Content-Type: application/json" \
  -d '{"template": "daily_summary"}'
```

```json
{ "id": "01JMXYZ..." }
```

### GET /v1/cron/jobs/running

List currently running jobs with concurrency info.

```bash
curl http://localhost:8080/v1/cron/jobs/running
```

```json
{
  "running": ["daily-review", "health-check"],
  "count": 2,
  "active_jobs": 2,
  "max_concurrent": 4,
  "available_slots": 2
}
```

### POST /v1/cron/jobs/:id/abort

Abort a running cron job. Uses CancellationToken to cooperatively cancel the task; shell tasks are killed immediately.

```bash
curl -X POST http://localhost:8080/v1/cron/jobs/daily-review/abort
```

```json
{ "aborted": "daily-review" }
```

Returns 404 if the job is not currently running.

### GET /v1/cron/templates

List available job templates.

```bash
curl http://localhost:8080/v1/cron/templates
```

```json
{
  "templates": [
    { "id": "daily_summary", "name": "Daily Summary", "cron": "0 9 * * *" },
    { "id": "weekly_review", "name": "Weekly Review", "cron": "0 9 * * MON" }
  ],
  "count": 6
}
```

---

## Configuration

### GET /v1/config

Get current configuration (secrets redacted).

```bash
curl http://localhost:8080/v1/config
```

```json
{
  "model": "anthropic/claude-sonnet-4-20250514",
  "workspace": "~/.zeus/workspace",
  "max_iterations": 20,
  "tui": { "theme": "dark", "vim_mode": false }
}
```

### PUT /v1/config

Update safe configuration fields at runtime.

```bash
curl -X PUT http://localhost:8080/v1/config \
  -H "Content-Type: application/json" \
  -d '{"model": "ollama/llama3.2", "max_iterations": 30}'
```

```json
{ "status": "updated" }
```

---

## Pantheon (Multi-Agent Missions)

Since Sprint 8, Pantheon missions execute real tools through the same `ToolExecutor` as the main agent loop — shell, read_file, web_fetch, browser, and Talos invocations all run for real during a mission step instead of returning simulated output. The gateway wires a dedicated `AgentToolExecutor` into `AppState.tool_executor` at startup; missions that omit `tool_executor` plumbing still fall back to simulated execution, but this is no longer the default.

### POST /v1/pantheon/missions

Create a multi-agent mission.

```bash
curl -X POST http://localhost:8080/v1/pantheon/missions \
  -H "Content-Type: application/json" \
  -d '{
    "description": "Research and summarize Rust async ecosystem",
    "agents": ["researcher", "writer"],
    "strategy": "sequential"
  }'
```

```json
{ "mission_id": "01JMXYZ...", "status": "created" }
```

### GET /v1/pantheon/missions/:id

Get mission status and results.

```bash
curl http://localhost:8080/v1/pantheon/missions/01JMXYZ...
```

```json
{
  "id": "01JMXYZ...",
  "status": "completed",
  "agents": ["researcher", "writer"],
  "results": [
    { "agent": "researcher", "status": "completed", "output": "..." },
    { "agent": "writer", "status": "completed", "output": "..." }
  ]
}
```

---

## WebSocket Streaming

### GET /v1/ws

Real-time streaming via WebSocket. Connect and send JSON messages. When `ZEUS_API_TOKEN` is configured on the gateway, pass it via the `token` query parameter (see the [Authentication](#authentication) section):

```bash
# Unauthenticated gateway
websocat ws://localhost:8080/v1/ws

# Authenticated gateway (browser-compatible query param)
websocat "ws://localhost:8080/v1/ws?token=$ZEUS_API_TOKEN"
```

**Send:**
```json
{"type": "chat", "message": "Hello Zeus!", "session_id": null}
```

**Receive** (sequence):
```json
{"type": "text_chunk", "content": "Hello"}
{"type": "text_chunk", "content": "! How"}
{"type": "text_chunk", "content": " can I help?"}
{"type": "response_complete", "content": "Hello! How can I help?", "session_id": "01JMXYZ..."}
{"type": "finished", "session_id": "01JMXYZ..."}
```

`response_complete` carries the `session_id` so clients can maintain session continuity when switching between WebSocket and the `POST /v1/chat` fallback path without creating a duplicate session.

Tool calls during streaming:
```json
{"type": "tool_call", "tool": "shell", "arguments": {"command": "ls"}}
{"type": "tool_result", "tool": "shell", "result": "file1.txt\nfile2.rs"}
```

---

## Outcome Templates

Reusable goal presets that enrich a user's task before it reaches the planner. The registry ships with 8 built-in templates (`debug-rust-crate`, `write-blog-post`, `code-review`, `research-topic`, `refactor-codebase`, `create-unit-tests`, `deploy-service`, `generate-image`) and supports user-authored YAML templates in `~/.zeus/templates/`.

Since Sprint 8, the Prometheus planner automatically attempts template enrichment on every task that passes through `create_plan`, so applying a template via these endpoints is only necessary when you want to inspect the match, surface missing providers, or override the auto-selection.

### GET /v1/templates

List all templates, optionally filtered by category.

```bash
curl "http://localhost:8080/v1/templates?category=programming" \
  -H "Authorization: Bearer $ZEUS_API_TOKEN"
```

```json
{
  "templates": [
    {
      "id": "debug-rust-crate",
      "name": "Debug a Rust Crate",
      "description": "Diagnose and fix a compile or runtime error in a Rust workspace",
      "categories": ["programming", "rust"],
      "tags": ["rust", "cargo", "debug"],
      "builtin": true,
      "usage_count": 14
    }
  ]
}
```

### GET /v1/templates/categories

Distinct category names across all loaded templates.

```bash
curl http://localhost:8080/v1/templates/categories \
  -H "Authorization: Bearer $ZEUS_API_TOKEN"
```

```json
{ "categories": ["programming", "rust", "writing", "research", "devops"] }
```

### GET /v1/templates/search?q=...

Keyword search over name, description, tags, and categories.

```bash
curl "http://localhost:8080/v1/templates/search?q=rust" \
  -H "Authorization: Bearer $ZEUS_API_TOKEN"
```

```json
{ "templates": [ { "id": "debug-rust-crate", "name": "Debug a Rust Crate" } ] }
```

### GET /v1/templates/:id

Fetch a single template's full definition (system prompt, tool policy, planning config, success criteria).

```bash
curl http://localhost:8080/v1/templates/debug-rust-crate \
  -H "Authorization: Bearer $ZEUS_API_TOKEN"
```

### POST /v1/templates

Create a user-authored template. Persists to `~/.zeus/templates/<id>.yaml`. Built-in IDs cannot be overwritten.

```bash
curl -X POST http://localhost:8080/v1/templates \
  -H "Authorization: Bearer $ZEUS_API_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "id": "incident-triage",
    "name": "Incident Triage",
    "description": "Walk through an on-call incident end-to-end",
    "categories": ["ops"],
    "tags": ["incident", "oncall"],
    "system_prompt": "You are an SRE on-call engineer...",
    "required_providers": [],
    "expected_outcome": "Root cause identified + mitigation applied + postmortem drafted"
  }'
```

### PUT /v1/templates/:id

Update a user-authored template. Built-ins return `403`.

### DELETE /v1/templates/:id

Delete a user-authored template. Built-ins return `403`.

### POST /v1/templates/:id/apply

Apply a template to a user goal and return the enriched prompt + any missing requirements. Use this to preview what the planner would see before committing to execution.

```bash
curl -X POST http://localhost:8080/v1/templates/debug-rust-crate/apply \
  -H "Authorization: Bearer $ZEUS_API_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"user_goal": "Fix the lifetime error in my parser", "configured_providers": ["llm"]}'
```

```json
{
  "template_id": "debug-rust-crate",
  "user_goal": "Fix the lifetime error in my parser",
  "enriched_prompt": "You are an expert Rust engineer...\n\n## Expected Outcome\nCrate compiles cleanly...\n\n## User Goal\nFix the lifetime error in my parser",
  "missing_providers": [],
  "missing_skills": [],
  "tool_policy": { "allowed_tools": null, "forbidden_tools": [], "max_shell_commands": null, "require_approval_for": [] },
  "planning_config": { "max_steps": 8, "execution_mode": "agent", "parallel_execution": false, "auto_replan": true, "step_timeout_ms": 120000, "include_context": [] }
}
```

`missing_providers` is populated when the template requires a category (e.g. `image_gen`) that the installation has not configured. The enriched prompt is still returned so the caller can decide whether to proceed anyway.

---

## Error Responses

Every endpoint returns JSON on both success and failure. Error bodies follow a single shape:

```json
{ "error": "human-readable description" }
```

Status codes used across the API:

| Code | Meaning | When it appears |
|------|---------|-----------------|
| `200 OK` | Success | Standard read/update response |
| `201 Created` | Resource created | `POST` to collection endpoints (sessions, agents, templates) |
| `204 No Content` | Success, no body | Idempotent deletes |
| `400 Bad Request` | Malformed payload | Missing required field, invalid JSON, invalid model string |
| `401 Unauthorized` | Missing / invalid token | Auth middleware rejected the request (see [Authentication](#authentication)) |
| `403 Forbidden` | Operation not allowed | Attempting to delete a built-in template, rotating a key without rotation configured |
| `404 Not Found` | Resource missing | Session / agent / template / mission ID does not exist |
| `409 Conflict` | State collision | Spawning an agent that is already spawned, creating a template whose ID already exists |
| `422 Unprocessable Entity` | Validated payload, domain error | Planner rejected the goal, tool execution blocked by policy |
| `429 Too Many Requests` | Upstream rate limit | Surfaced from an LLM provider or a configured cost budget |
| `500 Internal Server Error` | Unexpected failure | Bug — capture `X-Request-Id` header and the gateway log for triage |
| `503 Service Unavailable` | Subsystem degraded | Key rotation not configured, optional feature not wired, gateway shutting down |

Errors from LLM providers are normalized into the Zeus error shape before being returned, so clients do not need to distinguish between, say, an Anthropic 401 and an OpenAI 401 — both become Zeus `401` with a descriptive message.

---

## Full endpoint list

Zeus serves 200+ routes. Beyond the 30 documented above, additional endpoint groups include:

| Group | Prefix | Endpoints |
|-------|--------|-----------|
| Analytics | `/v1/analytics/*` | Cost aggregation, token usage, provider breakdown, budgets |
| Security | `/v1/security/*` | Threat log, permissions, API key inventory, allowlists |
| Approvals | `/v1/approvals/*` | Pending tool approval queue, approve/deny |
| MCP Servers | `/v1/mcp/servers/*` | Connect/disconnect MCP servers, list MCP tools |
| Extensions | `/v1/extensions/*` | Extension lifecycle (install, enable, disable, uninstall) |
| Projects | `/v1/projects/*` | Project CRUD, agent assignment |
| Network | `/v1/network/*` | Agent discovery, inter-agent messaging |
| Webhooks | `/v1/webhooks/*` | Inbound webhook receivers (generic, WhatsApp, voice) |
| Observatory | `/v1/observatory/*` | Real-time task monitoring dashboard |
| Fleet | `/v1/fleet/*` | Multi-machine agent fleet management |

The complete OpenAPI 3.0.3 spec is available at runtime:

```bash
curl http://localhost:8080/docs/openapi.json
```

Interactive documentation is served at `GET /docs`.
