# Zeus Deployment

Scripts and templates for deploying Zeus across the fleet.

## Quick Deploy (MCP Node)

Deploy Zeus MCP to a remote macOS node:

```bash
./deployment/deploy-mcp-node.sh 192.168.1.106
```

## Scripts

| Script | Purpose |
|--------|---------|
| `deploy-mcp-node.sh` | Deploy Zeus as MCP server for Claude Code on a remote node |
| `deploy-mac.sh` | Build from source and install locally on macOS |
| `deploy-to-node.sh` | Full deploy (build + install + daemon) to a remote Mac |
| `deploy-freebsd.sh` | Build and deploy gateway + web to FreeBSD |
| `deploy-web.sh` | Build and deploy Leptos/WASM web frontend |

## Templates

| File | Destination |
|------|-------------|
| `config.toml.template` | `~/.zeus/config.toml` |
| `ai.zeus.gateway.plist.template` | `~/Library/LaunchAgents/ai.zeus.gateway.plist` |
| `claude-settings.json.template` | `~/.claude/settings.json` |

Replace `{{VARIABLE}}` placeholders before use.

## MCP Transports

Zeus MCP supports three transports for Claude Code:

| Transport | Config Type | How |
|-----------|-------------|-----|
| **Stdio** | `"command"` | `zeus mcp` — reads JSON-RPC from stdin (recommended) |
| **SSE** | `"sse"` | Gateway `GET /sse` — server-sent events with session management |
| **HTTP POST** | n/a | Gateway `POST /mcp` — plain JSON-RPC (for direct API usage) |

## Fleet Nodes

| Node | IP | Status |
|------|-----|--------|
| .106 Mac Studio | 192.168.1.106 | Zeus MCP deployed |
| .100 Mac Mini M5 | 192.168.1.100 | Pending |
| .102 Mac Mini M2 | 192.168.1.102 | Pending |
| .107 Mac Mini M4 Pro | 192.168.1.107 | Pending |
