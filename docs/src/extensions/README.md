# Extensions

Zeus supports three extensibility mechanisms: the Deno bridge for JavaScript/TypeScript extensions, the OpenClaw-compatible skill system for SKILL.md-based plugins, and MCP (Model Context Protocol) server integration for connecting external tool providers.

## Deno Bridge (`zeus-extensions`)

Extensions can be written in JavaScript or TypeScript and run in isolated Deno subprocesses. The Deno bridge communicates with extensions via JSON-RPC over stdin/stdout, providing a secure and language-agnostic way to add new capabilities.

Extensions have fine-grained permissions controlling network access, filesystem read/write, and environment variable access. These map directly to Deno's permission flags (`--allow-net`, `--allow-read`, `--allow-write`, `--allow-env`).

See [Deno Bridge](./deno-bridge.md) for details.

## OpenClaw Compatibility (`zeus-skills`)

Zeus parses SKILL.md files following the OpenClaw format. Skills are self-contained packages that define custom tools, system prompts, and permissions in a single markdown file. They can be installed from ClawHub or loaded from local directories.

See [OpenClaw Compatibility](./openclaw.md) and [Skills System](./skills.md) for details.

## MCP Server Integration (`zeus-mcp`)

Zeus can connect to MCP (Model Context Protocol) servers, which expose tools and resources over a standardized protocol. MCP servers are managed via the API:

```
GET  /v1/mcp/servers           # List connected servers
POST /v1/mcp/servers           # Connect a new server
DELETE /v1/mcp/servers/:id     # Disconnect a server
GET  /v1/mcp/servers/:id/tools # List tools from a server
```

Tools from connected MCP servers are merged into the agent's tool palette and can be invoked like any built-in tool.

## Plugin System (`zeus-skills`)

Beyond SKILL.md parsing, `zeus-skills` provides a full plugin system supporting multiple runtimes:

- **Node.js plugins** -- Run via the `ProcessBridge` with JSON-RPC communication.
- **Python plugins** -- Run via the `ProcessBridge` with JSON-RPC communication.
- **Shell plugins** -- Execute shell commands with argument substitution.
- **WASM plugins** -- Run in a WebAssembly sandbox with configurable capabilities.
- **Native plugins** -- Dynamic library loading with hot-reload support.

## API Endpoints

Extensions and skills are managed through the REST API:

```
GET    /v1/skills           # List installed skills
POST   /v1/skills           # Install a skill
PUT    /v1/skills/:id       # Update skill (enable/disable)
DELETE /v1/skills/:id       # Uninstall a skill
```
