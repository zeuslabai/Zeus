# Skills System

The `zeus-skills` crate provides a comprehensive plugin and skill system that goes beyond SKILL.md parsing. It supports multiple plugin runtimes, a WASM sandbox, dynamic native library loading with hot-reload, and a unified plugin registry.

## SKILL.md Parser

The core of the skills system is the SKILL.md parser, which reads OpenClaw-format skill files and produces `Skill` structs. See [OpenClaw Compatibility](./openclaw.md) for the file format.

Each parsed skill contains:

| Field | Type | Description |
|-------|------|-------------|
| `name` | String | Skill name (from H1 heading) |
| `description` | String | Brief description |
| `version` | String | Semantic version |
| `author` | Option | Skill author |
| `system_prompt` | String | Agent instructions when skill is active |
| `tools` | Vec | Tool definitions with implementations |
| `permissions` | Vec | Required permissions |
| `path` | PathBuf | Directory containing the skill |
| `raw_content` | String | Original SKILL.md content |

## Permission System

Skills declare their permission requirements in the `## Permissions` section. The skill manager checks these permissions before execution. Common permission types include:

- **network** -- The skill needs internet access.
- **filesystem** -- The skill needs to read or write files.
- **shell** -- The skill needs to execute shell commands.
- **env** -- The skill needs access to environment variables.

Permissions are advisory: the Aegis security layer enforces actual restrictions. Skill permissions serve as documentation and allow the skill manager to warn users before installation.

## Plugin Runtimes

The `zeus-skills` crate supports multiple plugin runtimes through the `ProcessBridge`:

### Node.js Plugins

```rust
let plugin = NodePlugin::new("my-plugin", "/path/to/plugin.js");
let result = plugin.execute("tool_name", args).await?;
```

Node.js plugins communicate via JSON-RPC over stdin/stdout, similar to the Deno bridge.

### Python Plugins

```rust
let plugin = PythonPlugin::new("my-plugin", "/path/to/plugin.py");
let result = plugin.execute("tool_name", args).await?;
```

Python plugins use the same JSON-RPC protocol.

### Shell Plugins

Shell-based tools execute commands directly with argument substitution and sanitization.

## WASM Sandbox

The `WasmSandbox` provides a WebAssembly runtime for executing untrusted plugin code in a secure environment:

```rust
let sandbox = WasmSandboxBuilder::new()
    .with_capabilities(WasmCapabilities {
        allow_network: false,
        allow_filesystem: false,
        max_memory_bytes: 64 * 1024 * 1024,  // 64 MB
        max_execution_time: Duration::from_secs(30),
    })
    .build()?;

let result = sandbox.execute(wasm_bytes, "tool_name", args).await?;
```

WASM plugins run with configurable capabilities:

| Capability | Description |
|------------|-------------|
| `allow_network` | Whether the plugin can make network requests |
| `allow_filesystem` | Whether the plugin can access the filesystem |
| `max_memory_bytes` | Maximum memory the plugin can allocate |
| `max_execution_time` | Maximum execution duration before timeout |

## Dynamic Native Plugins

The `DynamicPluginLoader` supports loading native shared libraries (`.dylib` on macOS, `.so` on Linux) as plugins:

```rust
let loader = DynamicPluginLoaderBuilder::new()
    .with_plugin_dir("/path/to/plugins")
    .with_hot_reload(true)
    .build()?;

loader.load_all().await?;
```

Features include:

- **Hot-reload** -- Watches plugin directories for changes and reloads modified libraries without restarting Zeus.
- **Plugin events** -- Emits `PluginEvent` notifications on load, unload, and reload.
- **Metadata** -- Each native plugin provides a `NativePluginInfo` struct with name, version, and tool definitions.

## Plugin Registry

The `PluginRegistry` provides a unified interface across all plugin types:

```rust
let mut registry = PluginRegistry::new();
registry.register(plugin)?;

// List all plugins
for plugin in registry.list() {
    println!("{}: {}", plugin.name(), plugin.version());
}

// Execute a tool from any plugin
let result = registry.execute("plugin-name", "tool-name", args).await?;
```

## Skill Directory

Skills are stored under `~/.zeus/skills/` by default. The `SkillManager` scans this directory on startup, loading any subdirectory that contains a `SKILL.md` file.

```
~/.zeus/skills/
├── skill-a/
│   └── SKILL.md
├── skill-b/
│   ├── SKILL.md
│   └── helper.sh
└── skill-c/
    ├── SKILL.md
    └── plugin.wasm
```

## API Endpoints

Skills are managed through the REST API:

```
GET    /v1/skills           # List installed skills
POST   /v1/skills           # Install a skill (from ClawHub or local path)
PUT    /v1/skills/:id       # Update skill configuration (enable/disable)
DELETE /v1/skills/:id       # Uninstall a skill
```

The agent also exposes a skill summary in its system prompt so it knows which skills are available during conversation.
