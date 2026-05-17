# Contributing to Zeus

Thank you for your interest in contributing to Zeus. This document is a quick-start summary. The full contributor guide lives in the docs:

**→ [Full Contributor Guide](docs/src/contributing.md)**

---

## Quick Start

### Prerequisites

- Rust 1.85+ (Edition 2024) — install via [rustup](https://rustup.rs)
- macOS 14+ for full feature set (core crates build on Linux/FreeBSD)
- Deno 1.40+ for running extensions (`brew install deno`)
- `trunk` for WebUI (`cargo install trunk`)

### Build and Test

```bash
git clone https://github.com/zeuslabai/Zeus.git
cd Zeus
cargo build --workspace
cargo test --workspace         # ~7,307 tests
cargo fmt
cargo clippy --workspace -- -D warnings
```

### Run

```bash
cargo run                    # TUI
cargo run -- serve           # API server (port 8080)
cargo run -- gateway         # Full daemon
cargo run -- chat "Hello"   # Single message
```

> **Never build for production manually.** Use `./scripts/install.sh` (full install) or `./scripts/build.sh` (build only). Manual builds miss codesigning, service registration, and env var wiring.

---

## Project at a Glance

Zeus is a full-featured autonomous AI assistant (~58,800 lines of Rust, 32 crates, 7,307 tests):

- **8 core tools**: `read_file`, `write_file`, `edit_file`, `list_dir`, `shell`, `web_fetch`, `spawn`, `message`
- **11 LLM providers**: Anthropic, OpenAI, Ollama, OpenRouter, Google, Groq, Mistral, Together, Fireworks, Azure, Bedrock
- **5 frontends**: TUI (23 screens), macOS Desktop, iOS, Android, Web (45 pages, Leptos/WASM)
- **8 channel adapters**: Telegram, Discord, Slack, Email, iMessage, WhatsApp, Signal, Matrix
- **Extension runtime**: Deno subprocesses with JSON-RPC bridge, OpenClaw compatibility
- **267 API routes**, WebSocket streaming, fleet multi-agent coordination (Pantheon)

---

## Codebase Structure

```
crates/          # 32 workspace crates
apps/            # ZeusWeb (Leptos/WASM), ZeusDesktop (SwiftUI), ZeusMobile (iOS), zeus-android
scripts/         # install.sh, build.sh, test.sh, config-guard.sh
docs/src/        # mdBook documentation
src/             # Main binary (CLI entrypoint)
```

See [Architecture Overview](docs/src/architecture/README.md) and [Crate Map](docs/src/architecture/crate-map.md) for the full dependency graph.

---

## Key Conventions

- `cargo fmt` + `cargo clippy --workspace -- -D warnings` before every commit (zero-warning policy)
- No `unwrap()` on fallible paths in production code
- macOS-specific code guarded by `#[cfg(target_os = "macos")]`
- All API calls in the WebUI go through `apps/ZeusWeb/src/api/mod.rs` helpers (auth headers)
- WebSocket browser connections use `?token=` query param (browsers can't set `Authorization` on WS)
- Use `#[tracing::instrument]` on agent loop, tool execution, LLM calls, and message routing

---

## Submitting Changes

1. Fork and create a branch from `main`
2. Make your changes with tests
3. `cargo test --workspace && cargo clippy --workspace -- -D warnings && cargo fmt -- --check`
4. Submit a PR against `main`

See the [Full Contributor Guide](docs/src/contributing.md) for detailed instructions on adding Talos tools, channel adapters, extensions, WebUI pages, and the PR description template.

---

## License

MIT OR Apache-2.0 (dual license). By contributing, you agree your changes are licensed under the same terms.
