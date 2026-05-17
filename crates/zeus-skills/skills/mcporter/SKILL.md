# mcporter

MCP (Model Context Protocol) server management and tool bridging.

## Version
1.0.0

## Author
Zeus

## System Prompt
You are an MCP server management assistant. Help users discover, install, configure, and manage MCP servers that extend AI capabilities with additional tools and resources.

## Tools

### mcp_list_servers
List installed MCP servers.
```json
{
  "type": "object",
  "properties": {}
}
```

### mcp_server_info
Get info about an MCP server.
```json
{
  "type": "object",
  "properties": {
    "server": {
      "type": "string",
      "description": "Server name"
    }
  },
  "required": ["server"]
}
```

### mcp_install
Install an MCP server.
```json
{
  "type": "object",
  "properties": {
    "server": {
      "type": "string",
      "description": "Server package name or URL"
    }
  },
  "required": ["server"]
}
```

### mcp_start
Start an MCP server.
```json
{
  "type": "object",
  "properties": {
    "server": {
      "type": "string"
    },
    "port": {
      "type": "integer"
    }
  },
  "required": ["server"]
}
```

### mcp_stop
Stop an MCP server.
```json
{
  "type": "object",
  "properties": {
    "server": {
      "type": "string"
    }
  },
  "required": ["server"]
}
```

### mcp_list_tools
List tools provided by an MCP server.
```json
{
  "type": "object",
  "properties": {
    "server": {
      "type": "string"
    }
  },
  "required": ["server"]
}
```

### mcp_call_tool
Call a tool on an MCP server.
```json
{
  "type": "object",
  "properties": {
    "server": {
      "type": "string"
    },
    "tool": {
      "type": "string"
    },
    "args": {
      "type": "object"
    }
  },
  "required": ["server", "tool"]
}
```

### mcp_config
Get or set MCP configuration.
```json
{
  "type": "object",
  "properties": {
    "server": {
      "type": "string"
    },
    "key": {
      "type": "string"
    },
    "value": {
      "type": "string"
    }
  },
  "required": ["server"]
}
```

## Commands

### list_servers
```bash
ls -1 ~/.config/mcp/servers/
```

### install_npm
```bash
npm install -g {server}
```

### start_server
```bash
mcp-server-{server} --port {port} &
```

### stop_server
```bash
pkill -f "mcp-server-{server}"
```

### list_tools
```bash
curl -s http://localhost:{port}/tools | jq '.tools[] | {name, description}'
```

## Permissions
- shell
- network
- filesystem
