# Contributor Guide

Thank you for contributing to Zeus. This guide covers everything you need to set up a development environment, understand the codebase, and get changes merged.

For a high-level overview of the platform, see [Architecture Overview](./architecture/README.md). For API reference, see [API Reference](./api-reference/README.md).

---

## Development Setup

### Prerequisites

| Requirement | Minimum | Notes |
|---|---|---|
| Rust | 1.85+ | Edition 2024. Install via [rustup](https://rustup.rs) |
| macOS | 14+ (Sonoma) | Full feature set (Talos, Aegis, iMessage). Core crates build on Linux/FreeBSD |
| Xcode Command Line Tools | Latest | `xcode-select --install` |
| Deno | 1.40+ | Only needed to run extensions. `brew install deno` or https://deno.land |
| trunk | Latest | Only needed for WebUI (`apps/ZeusWeb`). `cargo install trunk` |

### Clone and Build

```bash
git clone https://github.com/zeuslabai/Zeus.git
cd Zeus
cargo build --workspace        # debug build (~2–4 min first time)
cargo build --release          # release build
```

### Run Tests

```bash
cargo test --workspace
```

There are ~7,307 tests. All should pass except 28 pre-existing aegis sandbox isolation failures (known macOS Seatbelt issue on CI). No external services are required — tests use temp directories, in-memory databases, and offline data.

Count tests precisely:
```bash
cargo test --workspace --list 2>/dev/null | grep -c ': test$'
```

### Run the Application

```bash
cargo run                    # TUI (default)
cargo run -- serve           # API server (port 8080)
cargo run -- gateway         # Full daemon (API + channels + heartbeat + cron)
cargo run -- chat "Hello"   # Single message
```

You need at least one LLM API key (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, etc.) or a local Ollama instance.

### Deployment Scripts

Never build manually for production. Always use the scripts:

```bash
./scripts/build.sh           # Background tmux build (recommended)
./scripts/test.sh            # Background tmux tests
./scripts/install.sh         # Full install + service registration
./scripts/config-guard.sh    # Validate config, detect corruption
```

`build.sh` handles codesigning, `install.sh` handles service registration, env var wiring, and identity deployment. Manual `cargo build --release` + copy is not equivalent.

---

## Project Structure

```
Zeus/
├── src/                        # Main binary (CLI entrypoint, command routing)
├── crates/                     # 32 workspace crates
│   ├── zeus-core/              # Types, errors, Config, Message, ToolSchema
│   ├── zeus-llm/               # Unified LLM (11 providers)
│   ├── zeus-memory/            # File-based workspace memory
│   ├── zeus-session/           # JSONL session storage + context manager
│   ├── zeus-agent/             # Agent loop + 8 core tools + subagents
│   ├── zeus-tui/               # Ratatui TUI (23 screens)
│   ├── zeus-api/               # REST API + WebSocket gateway (267 routes)
│   ├── zeus-mcp/               # Model Context Protocol support
│   ├── zeus-nous/              # Cognitive engine (intent, reasoning, learning)
│   ├── zeus-prometheus/        # Orchestration (planner, executor, cron, cooking loop)
│   ├── zeus-channels/          # 8 messaging adapters (Telegram, Discord, Slack, …)
│   ├── zeus-hermes/            # Notification router
│   ├── zeus-mnemosyne/         # Advanced memory (SQLite FTS5 + vector embeddings)
│   ├── zeus-athena/            # Documentation engine (Obsidian, Apple Notes)
│   ├── zeus-aegis/             # Security sandboxing (macOS Seatbelt, approvals, audit)
│   ├── zeus-talos/             # macOS automation (193 tools, 22 categories)
│   ├── zeus-browser/           # Chrome CDP browser automation (11 tools)
│   ├── zeus-skills/            # SKILL.md parser + OpenClaw compatibility
│   ├── zeus-voice/             # Voice calls (Twilio) + STT (Whisper) + TTS
│   ├── zeus-ffi/               # UniFFI Swift bindings (decommissioned S21)
│   ├── zeus-agora/             # Agent skill marketplace (listings, wallets, protocol)
│   ├── zeus-tts/               # Modular TTS providers (OpenAI, macOS say, Piper, Kokoro)
│   ├── zeus-sandbox/           # WASM sandbox with capability-based security
│   ├── zeus-orchestra/         # Multi-agent collaboration + Pantheon backend
│   ├── zeus-extensions/        # Deno extension runtime + OpenClaw compatibility
│   ├── zeus-acp/               # ACP/MCP bridge — stdio MCP server
│   ├── zeus-marketplace/       # Agent-to-agent skill marketplace
│   ├── zeus-economy/           # SQLite token/credit economy
│   ├── zeus-wallet/            # Ed25519 keypair + x402 payment protocol
│   ├── zeus-setup/             # TUI installer, builder, deployer
│   └── zeus-templates/         # Outcome templates — reusable goal presets
├── apps/
│   ├── ZeusWeb/                # Leptos/WASM web frontend (45 pages)
│   ├── ZeusDesktop/            # macOS SwiftUI app
│   ├── ZeusMobile/             # iOS SwiftUI app
│   └── zeus-android/           # Android app
└── scripts/                    # Build, install, deploy, config-guard
```

Each crate has a single responsibility and is independently compilable. The workspace `Cargo.toml` defines all shared dependencies. See [Crate Map](./architecture/crate-map.md) for dependency relationships.

---

## Code Style

### Formatting and Linting

Run both before every commit:

```bash
cargo fmt
cargo clippy --workspace -- -D warnings
```

Zero-warnings policy. CI rejects clippy warnings.

### Error Handling

- Use `anyhow::Context` to add context on external calls (IO, network, parsing).
- Use `zeus_core::Error` variants for domain-specific errors.
- No `unwrap()` on fallible paths in production code. Tests may use it freely.
- Return `Result<T>` (aliased as `zeus_core::Result<T>`) from public functions.
- Validate at system boundaries (user input, external APIs). Trust internal code and framework guarantees.

```rust
// Good
let content = std::fs::read_to_string(&path)
    .with_context(|| format!("reading config from {}", path.display()))?;

// Bad
let content = std::fs::read_to_string(&path).unwrap();
```

### Logging

Use the `tracing` crate. Add `#[instrument]` spans on agent loop iterations, tool executions, LLM calls, and message routing.

```rust
#[tracing::instrument(skip(self, args))]
async fn execute(&self, args: Value) -> Result<String> {
    tracing::info!("executing tool");
    // ...
}
```

### General Conventions

- Follow existing patterns in the codebase. When in doubt, read adjacent modules.
- Keep modules focused. Files growing past ~600 lines should be split.
- macOS-specific code must be guarded: `#[cfg(target_os = "macos")]`.
- Async-first: use `tokio` for all async work. CPU-bound work goes in `tokio::task::spawn_blocking`.
- Prefer `&str` over `String` in function parameters where ownership is not needed.
- Don't add error handling, fallbacks, or validation for scenarios that can't happen. Only validate at system boundaries.
- Don't add docstrings, comments, or type annotations to code you didn't change.

---

## Adding New Talos Tools

`zeus-talos` uses a trait-based registry. Each tool implements `TalosTool` and is registered in `TalosRegistry`. The agent discovers tools from the registry at startup — no other wiring needed.

### Step 1: Implement `TalosTool`

Add your tool to an existing module in `crates/zeus-talos/src/` or create a new module file for a new category.

```rust
use crate::TalosTool;
use async_trait::async_trait;
use serde_json::{json, Value};
use zeus_core::{Error, Result, ToolSchema};

pub struct MyNewTool;

#[async_trait]
impl TalosTool for MyNewTool {
    fn name(&self) -> &'static str { "my_new_tool" }
    fn description(&self) -> &'static str { "One sentence describing what this tool does." }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("input", "string", "The primary input value", true)
            .with_param("flag", "boolean", "Optional flag", false)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let input = args.get("input")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'input'".to_string()))?;

        // tool logic here

        Ok(serde_json::to_string_pretty(&json!({ "result": input }))?)
    }
}
```

### Step 2: Register

In `crates/zeus-talos/src/lib.rs`, add to the `register_all()` method:

```rust
// Cross-platform tools — outside the #[cfg] block
self.register(Box::new(my_module::MyNewTool));

// macOS-only tools — inside the #[cfg(target_os = "macos")] block
#[cfg(target_os = "macos")]
{
    self.register(Box::new(my_module::MyMacOnlyTool));
}
```

If you created a new module file, also add `pub mod my_module;` at the top of `lib.rs`.

### Step 3: Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_my_new_tool_schema() {
        let tool = MyNewTool;
        let schema = tool.schema();
        assert_eq!(schema.name, "my_new_tool");
        // verify required params exist
    }

    #[tokio::test]
    async fn test_my_new_tool_execute() {
        let tool = MyNewTool;
        let result = tool.execute(serde_json::json!({"input": "test"})).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_my_new_tool_missing_required_param() {
        let tool = MyNewTool;
        let result = tool.execute(serde_json::json!({})).await;
        assert!(result.is_err());
    }
}
```

---

## Adding Channel Adapters

`zeus-channels` uses a `ChannelAdapter` trait with a central `ChannelManager` for routing.

### Step 1: Implement `ChannelAdapter`

Create `crates/zeus-channels/src/myplatform.rs`:

```rust
use async_trait::async_trait;
use tokio::sync::mpsc;
use zeus_core::Result;
use crate::{ChannelAdapter, ChannelMessage, ChannelSource, ReceiveMode};

pub struct MyPlatformAdapter {
    // connection state, credentials, etc.
}

#[async_trait]
impl ChannelAdapter for MyPlatformAdapter {
    fn channel_type(&self) -> &'static str { "myplatform" }

    fn receive_mode(&self) -> ReceiveMode {
        ReceiveMode::WebSocket // or Polling, LongPolling, Webhook, Native
    }

    async fn start(&self, tx: mpsc::Sender<ChannelMessage>) -> Result<()> {
        // Begin listening; forward inbound messages to tx
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        Ok(())
    }

    async fn send(&self, to: &ChannelSource, content: &str) -> Result<()> {
        Ok(())
    }

    fn is_connected(&self) -> bool { true }
}
```

### Step 2: Add Config

Add a config struct in `crates/zeus-channels/src/config.rs` and wire it into `ChannelsConfig`. This maps to `[channels.myplatform]` in the user's `config.toml`.

### Step 3: Register

Add `pub mod myplatform;` in `crates/zeus-channels/src/lib.rs` and instantiate the adapter in `ChannelManager::from_config()`.

---

## Adding Extensions (Deno Runtime)

Extensions are JavaScript/TypeScript modules that run as Deno subprocesses and communicate with Zeus over JSON-RPC on stdin/stdout.

### Extension Structure

An extension is a single `.ts` file (or a directory with `index.ts` for OpenClaw extensions):

```typescript
// my_extension.ts
// Zeus sends JSON-RPC requests on stdin; extension replies on stdout.

import { readLines } from "https://deno.land/std/io/mod.ts";

for await (const line of readLines(Deno.stdin)) {
    const req = JSON.parse(line);
    let result: unknown = null;

    switch (req.method) {
        case "ping":
            result = { status: "pong", id: req.id };
            break;
        case "process":
            result = { output: `Processed: ${req.params.input}` };
            break;
        default:
            // Unknown method — return null result
            break;
    }

    const resp = { jsonrpc: "2.0", id: req.id, result };
    console.log(JSON.stringify(resp));
}
```

### Registering an Extension via API

```bash
curl -X POST http://localhost:8080/v1/extensions \
  -H "Authorization: Bearer $ZEUS_API_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "my-extension",
    "source": "/path/to/my_extension.ts",
    "version": "1.0.0",
    "description": "What this extension does",
    "permissions": {
      "allow_read": true,
      "allow_net": false,
      "allow_write": false,
      "allow_env": false
    }
  }'
```

Then start it:

```bash
curl -X POST http://localhost:8080/v1/extensions/{id}/start \
  -H "Authorization: Bearer $ZEUS_API_TOKEN"
```

### OpenClaw Extensions

OpenClaw extensions live in `~/openclaw/extensions/<name>/index.ts` and are loaded via the bridge shim. Install the bridge script at `~/.zeus/bridge/openclaw_bridge.ts` (or next to the zeus binary). Source convention in the API: use `openclaw:<name>` as the source value.

### Source Convention

The `info_to_registry_extension` helper in `zeus-api/src/extensions.rs` maps source strings:

| Source string | Runtime type |
|---|---|
| `http://...` or `https://...` | `ExtensionSource::Url` |
| `openclaw:<name>` | `ExtensionSource::OpenClaw` |
| anything else | `ExtensionSource::Local` (filesystem path) |

### Permissions

Extensions run in Deno's permission sandbox. Permissions are declared per-extension and mapped to Deno flags at spawn time (`--allow-net`, `--allow-read`, etc.). Grant only what the extension needs.

---

## WebUI Contributions (apps/ZeusWeb)

The WebUI is a Leptos 0.8 / WASM single-page app built with Tailwind CSS.

### Prerequisites

```bash
cargo install trunk
rustup target add wasm32-unknown-unknown
```

### Development

```bash
cd apps/ZeusWeb
trunk serve          # hot-reload dev server on http://localhost:8080
```

### Building

```bash
trunk build --release    # outputs to dist/
```

### Key Conventions

- Use `use leptos::prelude::*` (not the deprecated `use leptos::*`).
- All API calls go through `src/api/mod.rs` helpers — never use `gloo_net` directly in pages. This ensures auth headers are always present.
- WebSocket URLs must include `?token=` for the browser WS upgrade (browsers can't set `Authorization` headers on WS connections): `format!("{}//{}/v1/ws?token={}", protocol, host, token)`.
- Use `RwSignal` for local page state. Don't share signals across component boundaries — pass props or use context.
- Polling loops use `gloo_timers::future::TimeoutFuture` and should check tab visibility via `use_tab_visible()` before firing to avoid background churn.
- New pages are registered in `src/main.rs` with `<Route path="..." view=MyPage />`. Add the route, the page module under `src/pages/`, and a nav entry in `src/components/sidebar.rs`.

### WASM Constraints

`std` library features unavailable in WASM:
- No filesystem access — use API calls instead.
- No `std::process`, no threads. Use `spawn_local` for async tasks.
- `zeus_core::floor_char_boundary` is not available — inline the char-boundary walk:

```rust
let truncated = &s[..{
    let mut i = 300.min(s.len());
    while !s.is_char_boundary(i) { i -= 1; }
    i
}];
```

---

## Testing

### Running Tests

```bash
cargo test --workspace              # All ~7,307 tests
cargo test -p zeus-agent            # Single crate
cargo test -p zeus-api              # API + handler tests
cargo test -p zeus-talos            # Automation tool tests
cargo test -p zeus-channels         # Channel adapter tests
cargo test -p zeus-aegis            # Security sandbox tests
cargo test -p zeus-extensions       # Deno runtime tests
cargo test my_test_name             # Specific test by name
cargo test --workspace -- --nocapture   # With stdout
```

### Test Conventions

- Tests use `tempfile::TempDir` for filesystem isolation. Never write to real user directories.
- No external services required. Mock network calls or test against offline data.
- Tests live in the same file under `#[cfg(test)] mod tests { ... }`, or in a `tests/` directory for integration tests.
- Test names describe the behavior being verified: `test_ping_tool_missing_host_returns_error`.
- Async tests use `#[tokio::test]`. Sync tests use `#[test]`.

### What to Test

For every change:
1. Schema correctness (tool name, required parameters, descriptions).
2. Happy path execution.
3. Error cases (missing required parameters, invalid input, not found).
4. Boundary conditions relevant to your change.

### Platform Coverage

- Core crate changes must pass on macOS and Linux.
- macOS-only code (Talos, Aegis, iMessage) is guarded by `#[cfg(target_os = "macos")]` and tested only on macOS.
- FreeBSD is supported at the gateway/daemon level. Service scripts are in `scripts/freebsd/`.

---

## Commit Guidelines

Use [Conventional Commits](https://www.conventionalcommits.org/):

```
feat: add Safari reading list tool to Talos
fix: handle empty response from Ollama provider
refactor: extract message chunking into shared module
test: add integration tests for webhook endpoint
docs: update contributor guide with WASM constraints
chore: bump tokio to 1.44
```

- Keep commits focused. One logical change per commit.
- Reference issues when applicable: `fix: resolve session export crash (#42)`.
- Imperative mood: "add feature" not "added feature".
- If the change touches multiple crates and they're logically atomic (e.g., a cross-crate refactor), a single commit is fine. Don't split for the sake of splitting.

---

## Pull Request Process

1. **Fork** and create a branch from `main`.
2. **Build**: `cargo build --workspace`.
3. **Test**: `cargo test --workspace`.
4. **Lint**: `cargo clippy --workspace -- -D warnings`.
5. **Format**: `cargo fmt -- --check`.
6. **Submit** a PR against `main`.

### PR Requirements

- All tests pass (28 pre-existing aegis failures are known-acceptable).
- Zero clippy warnings.
- `cargo fmt` clean.
- New public functionality includes tests.
- If the change affects the architecture (new crate, changed subsystem wiring, new AppState field), update `CLAUDE.md` accordingly.

### PR Description Template

```markdown
## What

Brief description of the change.

## Why

Motivation — what was broken, missing, or could be improved.

## Testing

How to manually verify the change.

## Breaking changes

Any API or config incompatibilities, and migration path if applicable.
```

---

## Architecture Principles

Keep these in mind when contributing:

**Crate boundaries**: Each crate has a single, well-defined responsibility. Avoid circular dependencies. The `zeus-core` crate is the only shared foundation — everything else depends upward, not sideways.

**Optional subsystems**: All advanced crates (Nous, Mnemosyne, Athena, Aegis, Hermes, Prometheus) are optional and wired in through `Agent::with_subsystems()`. The core agent must work without them.

**Platform isolation**: macOS-specific code is guarded with `#[cfg(target_os = "macos")]`. Core functionality must compile and run on Linux and FreeBSD.

**Async-first**: Tokio is the async runtime. All IO-bound operations are async. CPU-bound work uses `spawn_blocking`. The `SharedState = Arc<RwLock<AppState>>` pattern is the standard for shared gateway state.

**Fail-open for non-critical subsystems**: Optional subsystems that fail to initialize (Nous, Mnemosyne, extensions) should warn and continue, not crash the gateway. Example: `AppState::boot()` starts extensions in `tokio::spawn` and logs warnings on failure.

**No panics in production paths**: Use `Result` types and proper error propagation. Reserve `unwrap()` and `expect()` for genuinely infallible cases (static regex compilation, initialized-at-startup singletons) or test code.

**Auth at the boundary**: The `ZEUS_API_TOKEN` Bearer token is checked in middleware. Handlers receive an already-authenticated request. For browser WebSocket connections (which can't set `Authorization` headers), the token is passed as `?token=` query param and validated at upgrade time.

**Simple by default, powerful when configured**: File-based memory for the common case, SQLite FTS5 for advanced search. Console output by default, multi-channel messaging when configured. Extensions disabled by default, opt-in per installation.

---

## License

By contributing to Zeus, you agree that your contributions will be licensed under the same terms as the project: MIT OR Apache-2.0 (dual license).
