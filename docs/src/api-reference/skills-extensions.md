# Skills, MCP & Extensions

Manage installed skills (SKILL.md plugins), Model Context Protocol (MCP) server connections, and extensions.

## Skills Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/v1/skills` | List installed skills |
| `POST` | `/v1/skills` | Install skill |
| `PUT` | `/v1/skills/:id` | Update skill |
| `DELETE` | `/v1/skills/:id` | Uninstall skill |

## MCP Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/v1/mcp/servers` | List MCP connections |
| `POST` | `/v1/mcp/servers` | Connect MCP server |
| `DELETE` | `/v1/mcp/servers/:id` | Disconnect server |
| `GET` | `/v1/mcp/servers/:id/tools` | List server tools |
| `POST` | `/v1/mcp/tools/:tool/test` | Test MCP tool |

## Extensions Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/v1/extensions` | List extensions |
| `POST` | `/v1/extensions` | Install extension |
| `GET` | `/v1/extensions/:id` | Get extension |
| `PUT` | `/v1/extensions/:id` | Update extension |
| `DELETE` | `/v1/extensions/:id` | Delete extension |
| `POST` | `/v1/extensions/:id/start` | Start extension |
| `POST` | `/v1/extensions/:id/stop` | Stop extension |

---

## Skills

Skills are SKILL.md-based plugins compatible with the OpenClaw format. They define custom tools, permissions, and behaviors that extend the agent's capabilities.

### GET `/v1/skills`

List all installed skills.

**Response** `200 OK`

```json
{
  "skills": [
    {
      "id": "code-review",
      "name": "Code Review",
      "version": "1.0.0",
      "description": "Automated code review with best practices",
      "enabled": true,
      "tools": ["review_file", "review_diff"],
      "source": "clawhub"
    }
  ]
}
```

---

### POST `/v1/skills`

Install a new skill from a SKILL.md file or ClawHub registry.

**Request Body**

```json
{
  "source": "clawhub",
  "name": "code-review"
}
```

Or install from a local SKILL.md file:

```json
{
  "source": "file",
  "path": "/path/to/SKILL.md"
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `source` | string | Yes | Installation source: `"clawhub"` or `"file"` |
| `name` | string | Conditional | Skill name (required for `clawhub` source) |
| `path` | string | Conditional | File path (required for `file` source) |

**Response** `201 Created`

```json
{
  "id": "code-review",
  "name": "Code Review",
  "version": "1.0.0",
  "tools": ["review_file", "review_diff"],
  "permissions": ["read_file", "shell"]
}
```

---

### PUT `/v1/skills/:id`

Update a skill. Primarily used to enable or disable a skill.

**Request Body**

```json
{
  "enabled": false
}
```

**Response** `200 OK`

```json
{
  "id": "code-review",
  "enabled": false
}
```

---

### DELETE `/v1/skills/:id`

Uninstall a skill. Removes its tools from the agent's available tool set.

**Response** `204 No Content`

---

## MCP (Model Context Protocol)

Connect to MCP servers to access external tools and data sources via the standardized Model Context Protocol.

### GET `/v1/mcp/servers`

List all connected MCP servers.

**Response** `200 OK`

```json
{
  "servers": [
    {
      "id": "mcp-a1b2c3d4-...",
      "name": "filesystem-server",
      "url": "stdio:///usr/local/bin/mcp-filesystem",
      "status": "connected",
      "tools_count": 5,
      "connected_at": "2026-02-11T10:00:00Z"
    }
  ]
}
```

---

### POST `/v1/mcp/servers`

Connect to an MCP server.

**Request Body**

```json
{
  "name": "filesystem-server",
  "url": "stdio:///usr/local/bin/mcp-filesystem",
  "args": ["--root", "/home/user"]
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | Yes | Display name for the server |
| `url` | string | Yes | Server URL (supports `stdio://` and `http://` transports) |
| `args` | array | No | Additional arguments for stdio servers |

**Response** `201 Created`

```json
{
  "id": "mcp-b2c3d4e5-...",
  "name": "filesystem-server",
  "status": "connected",
  "tools": [
    { "name": "read_file", "description": "Read a file from the filesystem" },
    { "name": "write_file", "description": "Write content to a file" }
  ]
}
```

---

### DELETE `/v1/mcp/servers/:id`

Disconnect from an MCP server and remove its tools from the available tool set.

**Response** `204 No Content`

---

### GET `/v1/mcp/servers/:id/tools`

List all tools provided by a specific MCP server.

**Response** `200 OK`

```json
{
  "server_id": "mcp-a1b2c3d4-...",
  "tools": [
    {
      "name": "read_file",
      "description": "Read a file from the filesystem",
      "parameters": {
        "type": "object",
        "properties": {
          "path": { "type": "string", "description": "File path to read" }
        },
        "required": ["path"]
      }
    }
  ]
}
```

---

### POST `/v1/mcp/tools/:tool/test`

Test an MCP tool by executing it with sample arguments.

**Path Parameters**

| Parameter | Type | Description |
|-----------|------|-------------|
| `tool` | string | The MCP tool name to test |

**Request Body**

```json
{
  "arguments": {
    "path": "/tmp/test.txt"
  }
}
```

**Response** `200 OK`

```json
{
  "tool": "read_file",
  "success": true,
  "result": "File contents here...",
  "duration_ms": 12
}
```

---

## Extensions

Extensions are longer-running processes that add capabilities to Zeus. They can provide tools, background services, or integrations.

### GET `/v1/extensions`

List all installed extensions.

**Response** `200 OK`

```json
{
  "extensions": [
    {
      "id": "ext-a1b2c3d4-...",
      "name": "Custom Analytics",
      "version": "1.0.0",
      "status": "running",
      "tools_provided": 3,
      "installed_at": "2026-02-10T12:00:00Z"
    }
  ]
}
```

---

### POST `/v1/extensions`

Install a new extension.

**Request Body**

```json
{
  "name": "Custom Analytics",
  "source": "https://github.com/user/zeus-analytics-ext",
  "config": {
    "api_key": "..."
  }
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | Yes | Extension display name |
| `source` | string | Yes | Installation source (URL or local path) |
| `config` | object | No | Extension-specific configuration |

**Response** `201 Created`

```json
{
  "id": "ext-b2c3d4e5-...",
  "name": "Custom Analytics",
  "version": "1.0.0",
  "status": "installed"
}
```

---

### GET `/v1/extensions/:id`

Get details for a specific extension.

**Response** `200 OK`

```json
{
  "id": "ext-a1b2c3d4-...",
  "name": "Custom Analytics",
  "version": "1.0.0",
  "description": "Custom analytics dashboard and reporting",
  "status": "running",
  "tools_provided": ["analytics_query", "analytics_report", "analytics_export"],
  "config": { "api_key": "****" },
  "installed_at": "2026-02-10T12:00:00Z",
  "started_at": "2026-02-10T12:00:05Z"
}
```

---

### PUT `/v1/extensions/:id`

Update an extension's configuration.

**Request Body**

```json
{
  "config": {
    "api_key": "new-key-value"
  }
}
```

**Response** `200 OK`

```json
{
  "id": "ext-a1b2c3d4-...",
  "status": "updated"
}
```

---

### DELETE `/v1/extensions/:id`

Uninstall an extension. Stops the extension if it is currently running.

**Response** `204 No Content`

---

### POST `/v1/extensions/:id/start`

Start a stopped extension.

**Response** `200 OK`

```json
{
  "id": "ext-a1b2c3d4-...",
  "status": "running"
}
```

---

### POST `/v1/extensions/:id/stop`

Stop a running extension. The extension remains installed but inactive.

**Response** `200 OK`

```json
{
  "id": "ext-a1b2c3d4-...",
  "status": "stopped"
}
```
