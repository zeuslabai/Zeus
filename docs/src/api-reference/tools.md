# Tools

List available tools and execute them directly via the API. Zeus provides 212 tools across its core agent, macOS automation (Talos), browser automation, and connected MCP servers.

## Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/v1/tools` | List available tools |
| `POST` | `/v1/tools/:name` | Execute a tool |

---

## GET `/v1/tools`

List all available tools with their schemas and descriptions.

**Response** `200 OK`

```json
{
  "tools": [
    {
      "name": "read_file",
      "description": "Read the contents of a file",
      "parameters": {
        "type": "object",
        "properties": {
          "path": {
            "type": "string",
            "description": "Path to the file to read"
          }
        },
        "required": ["path"]
      },
      "category": "core"
    },
    {
      "name": "shell",
      "description": "Execute a shell command",
      "parameters": {
        "type": "object",
        "properties": {
          "command": {
            "type": "string",
            "description": "The shell command to execute"
          }
        },
        "required": ["command"]
      },
      "category": "core"
    }
  ],
  "count": 212
}
```

### Tool Categories

| Category | Count | Description |
|----------|-------|-------------|
| `core` | 8 | Built-in agent tools (read_file, write_file, edit_file, list_dir, shell, web_fetch, spawn, message) |
| `talos` | 193 | macOS automation tools across 22 categories |
| `browser` | 11 | Chrome CDP browser automation tools |
| `mcp` | varies | Tools from connected MCP servers |

---

## POST `/v1/tools/:name`

Execute a tool by name with the provided arguments. Tool execution passes through Aegis security checks (command filtering, URL allowlisting, path restrictions) before running.

**Path Parameters**

| Parameter | Type | Description |
|-----------|------|-------------|
| `name` | string | The tool name (e.g., `read_file`, `shell`, `system_info`) |

**Request Body**

```json
{
  "arguments": {
    "path": "/Users/me/project/Cargo.toml"
  }
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `arguments` | object | Yes | Tool-specific arguments matching the tool's parameter schema |

**Response** `200 OK`

```json
{
  "tool": "read_file",
  "result": "[package]\nname = \"zeus\"\nversion = \"1.0.0\"\n...",
  "success": true,
  "duration_ms": 5
}
```

**Response** `404 Not Found` -- unknown tool name:

```json
{
  "error": "Tool 'unknown_tool' not found"
}
```

**Response** `403 Forbidden` -- blocked by Aegis security:

```json
{
  "error": "Tool execution denied: command 'rm -rf /' is not allowed"
}
```

### Examples

**List a directory:**

```bash
curl -X POST http://localhost:8080/v1/tools/list_dir \
  -H "Content-Type: application/json" \
  -d '{"arguments": {"path": "."}}'
```

**Execute a shell command:**

```bash
curl -X POST http://localhost:8080/v1/tools/shell \
  -H "Content-Type: application/json" \
  -d '{"arguments": {"command": "cargo test --workspace 2>&1 | tail -5"}}'
```

**Fetch a URL:**

```bash
curl -X POST http://localhost:8080/v1/tools/web_fetch \
  -H "Content-Type: application/json" \
  -d '{"arguments": {"url": "https://example.com"}}'
```
