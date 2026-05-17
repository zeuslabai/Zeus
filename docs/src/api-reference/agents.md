# Agents

Manage agent definitions, spawn agents into the runtime registry, and interact with agents via the Agent-as-API pattern.

## Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/v1/agents` | List agent definitions |
| `POST` | `/v1/agents` | Create agent |
| `GET` | `/v1/agents/:id` | Get agent definition |
| `PUT` | `/v1/agents/:id` | Update agent |
| `DELETE` | `/v1/agents/:id` | Delete agent |
| `POST` | `/v1/agents/spawn` | Spawn agent into registry |
| `POST` | `/v1/agents/:id/chat` | Chat with agent |
| `POST` | `/v1/agents/:id/send` | Send message to spawned agent |
| `GET` | `/v1/agents/:id/status` | Runtime status of spawned agent |

---

## GET `/v1/agents`

List all agent definitions.

**Response** `200 OK`

```json
{
  "agents": [
    {
      "id": "default",
      "name": "Zeus",
      "model": "anthropic/claude-sonnet-4-20250514",
      "bindings": ["core", "talos"],
      "tool_policy": "allow_all",
      "priority": 1
    }
  ]
}
```

---

## POST `/v1/agents`

Create a new agent definition.

**Request Body**

```json
{
  "name": "Code Reviewer",
  "model": "anthropic/claude-sonnet-4-20250514",
  "system_prompt": "You are a senior code reviewer. Focus on correctness, performance, and readability.",
  "bindings": ["core"],
  "tool_policy": "allow_all",
  "priority": 2
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | Yes | Display name for the agent |
| `model` | string | No | Model identifier (defaults to config model) |
| `system_prompt` | string | No | Custom system prompt |
| `bindings` | array | No | Tool binding groups (e.g., `"core"`, `"talos"`, `"browser"`) |
| `tool_policy` | string | No | Tool execution policy (`"allow_all"`, `"ask"`, `"deny"`) |
| `priority` | integer | No | Agent priority for task assignment |

**Response** `201 Created`

```json
{
  "id": "a1b2c3d4-...",
  "name": "Code Reviewer",
  "model": "anthropic/claude-sonnet-4-20250514"
}
```

---

## GET `/v1/agents/:id`

Get a specific agent definition.

**Response** `200 OK`

```json
{
  "id": "a1b2c3d4-...",
  "name": "Code Reviewer",
  "model": "anthropic/claude-sonnet-4-20250514",
  "system_prompt": "You are a senior code reviewer...",
  "bindings": ["core"],
  "tool_policy": "allow_all",
  "priority": 2,
  "created_at": "2026-02-11T10:00:00Z"
}
```

---

## PUT `/v1/agents/:id`

Update an agent definition. Supports partial updates -- only the provided fields are changed.

**Request Body**

```json
{
  "bindings": ["core", "browser"],
  "tool_policy": "ask",
  "priority": 3
}
```

**Response** `200 OK`

```json
{
  "id": "a1b2c3d4-...",
  "name": "Code Reviewer",
  "bindings": ["core", "browser"],
  "tool_policy": "ask",
  "priority": 3
}
```

---

## DELETE `/v1/agents/:id`

Delete an agent definition. Stops the agent if it is currently spawned.

**Response** `204 No Content`

---

## POST `/v1/agents/spawn`

Spawn an agent into the runtime registry, activating its bindings and making it available for message routing.

**Request Body**

```json
{
  "agent_id": "a1b2c3d4-..."
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `agent_id` | string | Yes | ID of the agent definition to spawn |

**Response** `200 OK`

```json
{
  "status": "spawned",
  "agent_id": "a1b2c3d4-...",
  "runtime_id": "rt-x1y2z3-..."
}
```

---

## POST `/v1/agents/:id/chat`

Chat with a specific agent. This is the **Agent-as-API** pattern: the endpoint creates or reuses a session tied to the agent, runs the message through the full agent loop (including tools, memory, and cognitive engine), and returns the response.

**Request Body**

```json
{
  "message": "Review this function for potential issues:\n\nfn divide(a: i32, b: i32) -> i32 { a / b }"
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `message` | string | Yes | The message to send to the agent |

**Response** `200 OK`

```json
{
  "response": "This function has a division-by-zero risk. When `b` is 0, this will panic at runtime...",
  "session_id": "d4e5f6g7-...",
  "tool_calls": []
}
```

Each agent maintains its own session. Subsequent calls to the same agent reuse the session, preserving conversation context.

---

## POST `/v1/agents/:id/send`

Send a message to a spawned agent. Unlike `/chat`, this is a fire-and-forget delivery to the agent's message queue.

**Request Body**

```json
{
  "message": "Check the build status and report back"
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `message` | string | Yes | The message to deliver |

**Response** `200 OK`

```json
{
  "status": "delivered",
  "agent_id": "a1b2c3d4-..."
}
```

**Response** `404 Not Found` if the agent is not currently spawned.

---

## GET `/v1/agents/:id/status`

Get the runtime status of a spawned agent.

**Response** `200 OK`

```json
{
  "agent_id": "a1b2c3d4-...",
  "name": "Code Reviewer",
  "status": "idle",
  "session_id": "d4e5f6g7-...",
  "uptime_seconds": 3600,
  "messages_processed": 12,
  "last_active": "2026-02-11T11:30:00Z"
}
```

| Status | Description |
|--------|-------------|
| `idle` | Agent is spawned and waiting for messages |
| `processing` | Agent is currently handling a message |
| `error` | Agent encountered an error |
| `stopped` | Agent has been stopped |
