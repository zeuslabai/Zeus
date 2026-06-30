# Deno Bridge

Zeus extensions can be written in JavaScript or TypeScript and run via the Deno bridge in `zeus-extensions`. Each extension runs in an isolated Deno subprocess, communicating with Zeus over JSON-RPC via stdin/stdout.

## Architecture

```
Zeus Process
    |
    |  JSON-RPC (stdin/stdout)
    v
Deno Subprocess (isolated)
    |
    |  Extension code (.ts / .js)
    v
Extension logic (tools, handlers)
```

The `ExtensionRegistry` manages the full lifecycle of extensions: registration, startup, shutdown, and uninstallation.

## Extension Sources

Extensions can be loaded from three sources:

| Source | Description |
|--------|-------------|
| **Local** | A path to a `.ts` or `.js` file on the local filesystem. |
| **URL** | A URL to download the extension from (planned, not yet implemented). |
| **OpenClaw** | A name in the OpenClaw extension registry. Uses the bridge script to wrap OpenClaw extensions. |

## Permissions

Each extension has a `ExtensionPermissions` struct that controls what the Deno subprocess can access:

```rust
ExtensionPermissions {
    allow_net: vec!["api.example.com"],   // Hosts the extension can connect to
    allow_read: vec!["/home/user/data"],  // Paths it can read from
    allow_write: vec!["/tmp"],            // Paths it can write to
    allow_env: vec!["API_KEY"],           // Environment variables it can access
}
```

These are translated directly to Deno CLI flags:

| Permission | Deno Flag | Wildcard |
|------------|-----------|----------|
| `allow_net` | `--allow-net=host1,host2` | `"*"` allows all network |
| `allow_read` | `--allow-read=path1,path2` | `"/"` allows all reads |
| `allow_write` | `--allow-write=path1,path2` | (no wildcard) |
| `allow_env` | `--allow-env=VAR1,VAR2` | `"*"` allows all env vars |

For trusted extensions, use `ExtensionPermissions::allow_all()` which grants broad access.

## Lifecycle

### Registration

```rust
let ext = Extension::new("my-extension", ExtensionSource::Local("/path/to/ext.ts".into()))
    .with_permissions(ExtensionPermissions::allow_all())
    .with_version("1.0.0");

registry.register(ext).await?;
```

### Starting

```rust
registry.start("extension-id").await?;
```

This spawns a Deno subprocess with the configured permissions. A background task reads stdout/stderr and stores log entries on the extension (last 500 entries retained).

### Communication

Zeus sends JSON-RPC requests to the extension's stdin:

```json
{
    "jsonrpc": "2.0",
    "id": 1,
    "method": "execute_tool",
    "params": {"name": "my_tool", "args": {"input": "hello"}}
}
```

The extension responds on stdout:

```json
{
    "jsonrpc": "2.0",
    "id": 1,
    "result": {"output": "world"}
}
```

### Stopping

```rust
registry.stop("extension-id").await?;
```

Sends a SIGTERM to the Deno subprocess and waits for it to exit.

### Uninstallation

```rust
registry.uninstall("extension-id").await?;
```

An extension must be stopped before it can be uninstalled.

## OpenClaw Bridge

Extensions sourced from the OpenClaw registry are run through a bridge script (`openclaw_bridge.ts`). The bridge wraps the OpenClaw extension interface to work with Zeus's JSON-RPC protocol. The bridge script is located at one of:

- Next to the Zeus binary
- `~/.zeus/bridge/openclaw_bridge.ts`
- `share/zeus/openclaw_bridge.ts` relative to the binary

The bridge can also be set explicitly via `ExtensionRegistry::with_bridge_path()`.

## Extension Status

Extensions report their runtime status:

| Status | Description |
|--------|-------------|
| `running` | Deno subprocess is active |
| `stopped` | Not running |
| `starting` | Subprocess is being spawned |
| `stopping` | Subprocess is shutting down |
| `error(msg)` | An error occurred |

## Logging

Each extension maintains a rolling log of the last 500 entries with timestamp, level (debug/info/warn/error), and message. Logs are stored in memory on the `Extension` struct and can be retrieved via the registry.
