# Projects, Teams & Network

Manage projects and agent assignments, configure teams and delegations, use smart routing, and discover agents on the network.

## Projects Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/v1/projects` | List projects |
| `POST` | `/v1/projects` | Create project |
| `GET` | `/v1/projects/:id` | Project detail |
| `PUT` | `/v1/projects/:id` | Update project |
| `DELETE` | `/v1/projects/:id` | Delete project |
| `PUT` | `/v1/projects/:id/agents` | Assign agents |

## Teams & Delegations Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/v1/teams` | List teams |
| `POST` | `/v1/teams` | Create team |
| `GET` | `/v1/teams/:id` | Get team |
| `GET` | `/v1/delegations` | List delegations |
| `POST` | `/v1/delegations` | Create delegation |

## Routing Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/v1/routing/recommend` | Smart route recommendation |
| `GET` | `/v1/routing/costs` | Routing cost info |
| `GET` | `/v1/routing/budget` | Budget tracking |
| `POST` | `/v1/routing/cost-recommend` | Cost-based recommendation |

## Network Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/v1/network/agents` | Discovered agents |
| `GET` | `/v1/network/discover` | Discovery results |
| `GET` | `/v1/network/messages` | Inter-agent messages |

---

## Projects

### GET `/v1/projects`

List all projects.

**Response** `200 OK`

```json
{
  "projects": [
    {
      "id": "proj-a1b2c3d4-...",
      "name": "Zeus Development",
      "description": "Main Zeus project",
      "agents": ["default", "code-reviewer"],
      "created_at": "2026-02-01T10:00:00Z"
    }
  ]
}
```

---

### POST `/v1/projects`

Create a new project.

**Request Body**

```json
{
  "name": "Website Redesign",
  "description": "Redesign the company website with new branding"
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | Yes | Project name |
| `description` | string | No | Project description |

**Response** `201 Created`

```json
{
  "id": "proj-b2c3d4e5-...",
  "name": "Website Redesign",
  "created_at": "2026-02-11T10:00:00Z"
}
```

---

### GET `/v1/projects/:id`

Get project details including assigned agents and status.

**Response** `200 OK`

```json
{
  "id": "proj-a1b2c3d4-...",
  "name": "Zeus Development",
  "description": "Main Zeus project",
  "agents": ["default", "code-reviewer"],
  "sessions": 12,
  "last_activity": "2026-02-11T11:30:00Z",
  "created_at": "2026-02-01T10:00:00Z"
}
```

---

### PUT `/v1/projects/:id`

Update a project. Supports partial updates.

**Request Body**

```json
{
  "name": "Zeus Development v2",
  "description": "Updated project description"
}
```

**Response** `200 OK`

```json
{
  "id": "proj-a1b2c3d4-...",
  "name": "Zeus Development v2"
}
```

---

### DELETE `/v1/projects/:id`

Delete a project. Does not delete associated sessions or agents.

**Response** `204 No Content`

---

### PUT `/v1/projects/:id/agents`

Assign agents to a project. Replaces the current agent assignments.

**Request Body**

```json
{
  "agents": ["default", "code-reviewer", "qa-tester"]
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `agents` | array | Yes | List of agent IDs to assign |

**Response** `200 OK`

```json
{
  "id": "proj-a1b2c3d4-...",
  "agents": ["default", "code-reviewer", "qa-tester"]
}
```

---

## Teams & Delegations

### GET `/v1/teams`

List all configured teams.

**Response** `200 OK`

```json
{
  "teams": [
    {
      "id": "team-a1b2c3d4-...",
      "name": "Engineering",
      "agents": ["default", "code-reviewer"],
      "created_at": "2026-02-05T10:00:00Z"
    }
  ]
}
```

---

### POST `/v1/teams`

Create a new team.

**Request Body**

```json
{
  "name": "Engineering",
  "agents": ["default", "code-reviewer"]
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | Yes | Team name |
| `agents` | array | No | Initial agent IDs to include |

**Response** `201 Created`

```json
{
  "id": "team-b2c3d4e5-...",
  "name": "Engineering",
  "agents": ["default", "code-reviewer"]
}
```

---

### GET `/v1/teams/:id`

Get team details.

**Response** `200 OK`

```json
{
  "id": "team-a1b2c3d4-...",
  "name": "Engineering",
  "agents": [
    { "id": "default", "name": "Zeus", "status": "idle" },
    { "id": "code-reviewer", "name": "Code Reviewer", "status": "processing" }
  ],
  "created_at": "2026-02-05T10:00:00Z"
}
```

---

### GET `/v1/delegations`

List all active delegations between agents.

**Response** `200 OK`

```json
{
  "delegations": [
    {
      "id": "del-a1b2c3d4-...",
      "from_agent": "default",
      "to_agent": "code-reviewer",
      "task": "Review all pull requests",
      "status": "active",
      "created_at": "2026-02-10T10:00:00Z"
    }
  ]
}
```

---

### POST `/v1/delegations`

Create a delegation from one agent to another.

**Request Body**

```json
{
  "from_agent": "default",
  "to_agent": "code-reviewer",
  "task": "Review the latest PR for security issues"
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `from_agent` | string | Yes | ID of the delegating agent |
| `to_agent` | string | Yes | ID of the agent receiving the delegation |
| `task` | string | Yes | Description of the delegated task |

**Response** `201 Created`

```json
{
  "id": "del-b2c3d4e5-...",
  "from_agent": "default",
  "to_agent": "code-reviewer",
  "status": "active"
}
```

---

## Routing

Smart routing recommendations for directing messages to the optimal agent based on capability and cost.

### POST `/v1/routing/recommend`

Get a routing recommendation for a given message. Analyzes the message content and suggests the best agent to handle it.

**Request Body**

```json
{
  "message": "Review this Rust code for memory safety issues",
  "candidates": ["default", "code-reviewer", "security-auditor"]
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `message` | string | Yes | The message to route |
| `candidates` | array | No | Limit recommendations to these agent IDs |

**Response** `200 OK`

```json
{
  "recommended_agent": "code-reviewer",
  "confidence": 0.92,
  "reasoning": "Message requests code review, matching Code Reviewer agent specialization",
  "alternatives": [
    { "agent": "security-auditor", "confidence": 0.78 },
    { "agent": "default", "confidence": 0.45 }
  ]
}
```

---

### GET `/v1/routing/costs`

Get cost information for routing to each available agent.

**Response** `200 OK`

```json
{
  "agents": [
    {
      "id": "default",
      "model": "anthropic/claude-sonnet-4-20250514",
      "cost_per_1k_prompt": 0.003,
      "cost_per_1k_completion": 0.015
    },
    {
      "id": "code-reviewer",
      "model": "anthropic/claude-sonnet-4-20250514",
      "cost_per_1k_prompt": 0.003,
      "cost_per_1k_completion": 0.015
    }
  ]
}
```

---

### GET `/v1/routing/budget`

Get current budget tracking across all agents and routing decisions.

**Response** `200 OK`

```json
{
  "total_spent": 12.45,
  "budget_limit": 100.00,
  "remaining": 87.55,
  "by_agent": {
    "default": 10.20,
    "code-reviewer": 2.25
  }
}
```

---

### POST `/v1/routing/cost-recommend`

Get a routing recommendation optimized for cost. Prefers cheaper models when they can adequately handle the task.

**Request Body**

```json
{
  "message": "What time is it?",
  "max_cost": 0.01
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `message` | string | Yes | The message to route |
| `max_cost` | number | No | Maximum cost budget for the request |

**Response** `200 OK`

```json
{
  "recommended_agent": "default",
  "model": "ollama/llama3.2",
  "estimated_cost": 0.00,
  "reasoning": "Simple query can be handled by local model at zero cost"
}
```

---

## Network

Agent discovery and inter-agent communication on the network.

### GET `/v1/network/agents`

List agents discovered on the network (local and remote).

**Response** `200 OK`

```json
{
  "agents": [
    {
      "id": "default",
      "name": "Zeus",
      "location": "local",
      "status": "online",
      "capabilities": ["core", "talos", "browser"]
    },
    {
      "id": "remote-agent-1",
      "name": "Remote Worker",
      "location": "192.168.1.50:8080",
      "status": "online",
      "capabilities": ["core"]
    }
  ]
}
```

---

### GET `/v1/network/discover`

Trigger and return agent discovery results. Scans for Zeus instances on the local network.

**Response** `200 OK`

```json
{
  "discovered": [
    {
      "address": "192.168.1.50:8080",
      "name": "Remote Worker",
      "version": "1.0.0",
      "discovered_at": "2026-02-11T10:00:00Z"
    }
  ],
  "scan_duration_ms": 2500
}
```

---

### GET `/v1/network/messages`

View inter-agent messages exchanged between local and remote agents.

**Response** `200 OK`

```json
{
  "messages": [
    {
      "id": "msg-a1b2c3d4-...",
      "from": "default",
      "to": "remote-agent-1",
      "content": "Please run the test suite on the remote server",
      "timestamp": "2026-02-11T11:00:00Z",
      "status": "delivered"
    },
    {
      "id": "msg-b2c3d4e5-...",
      "from": "remote-agent-1",
      "to": "default",
      "content": "All 1711 tests passed",
      "timestamp": "2026-02-11T11:02:00Z",
      "status": "delivered"
    }
  ]
}
```
