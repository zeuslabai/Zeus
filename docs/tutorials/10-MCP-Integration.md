# MCP Integration — Zeus as an MCP Server

Zeus implements the Model Context Protocol (MCP) and works as an MCP server for Claude Code. This gives Claude Code access to all 212 Zeus tools.

## What is MCP?

MCP (Model Context Protocol) is a standard for connecting AI assistants to tool servers. When Zeus runs as an MCP server, Claude Code can call Zeus tools like `shell`, `read_file`, `web_fetch`, etc.

## Setup

### During Onboarding

The setup wizard (`zeus onboard`) can configure MCP automatically. When asked about Claude Code integration, say yes.

### Manual Setup

Add Zeus to your Claude Code MCP configuration:

**macOS/Linux** — `~/.config/claude/claude_mcp_config.json`:

```json
{
  "mcpServers": {
    "zeus": {
      "command": "/usr/local/bin/zeus",
      "args": ["mcp"],
      "env": {
        "ANTHROPIC_API_KEY": "sk-ant-..."
      }
    }
  }
}
```

> ⚠️ The canonical binary path is `/usr/local/bin/zeus`. Never change this path — if Zeus crashes, diagnose the root cause instead.

### Verify

In Claude Code, you should see Zeus tools available:

```
mcp__zeus__shell
mcp__zeus__read_file
mcp__zeus__write_file
mcp__zeus__list_dir
mcp__zeus__web_fetch
mcp__zeus__edit_file
```

## Using Zeus Tools in Claude Code

Once configured, Claude Code can use Zeus tools directly:

```
mcp__zeus__shell: ls -la
mcp__zeus__read_file: /path/to/file.rs
mcp__zeus__web_fetch: https://example.com
```

## MCP Server Configuration

```toml
# ~/.zeus/config.toml
[mcp_server]
enable_mnemosyne = true    # Enable memory tools in MCP mode
```

When `enable_mnemosyne = true`, MCP clients also get access to memory tools:
- `memory_recall` — Search long-term memory
- `memory_store` — Store a fact
- `memory_search` — Full-text search

## Environment Variables

MCP reads environment variables at startup (via `OnceLock`). If you change env vars, you must restart the MCP server for changes to take effect.

All env vars should be in `~/.zeus/.env`:

```bash
ANTHROPIC_API_KEY=sk-ant-...
OPENAI_API_KEY=sk-...
```

## Troubleshooting

### MCP crashes or fails to connect

1. **Never change the binary path** in `settings.json` — canonical path is always `/usr/local/bin/zeus`
2. Check MCP logs: `~/.zeus/mcp.log` (if configured)
3. Verify binary exists: `ls -la /usr/local/bin/zeus`
4. Run manually: `/usr/local/bin/zeus mcp` — check for error output

### Tools not showing up

1. Restart Claude Code after changing MCP config
2. Verify config syntax in `claude_mcp_config.json`
3. Check that Zeus builds successfully: `zeus doctor`

### OnceLock / env var issues

MCP initializes env vars at startup via `OnceLock`. If a key isn't being picked up:
1. Add it to `~/.zeus/.env`
2. Restart the MCP server (restart Claude Code)

## What's Next

→ [[11-Browser-Automation]] — Chrome DevTools Protocol
→ [[05-Tools]] — Full tool reference
