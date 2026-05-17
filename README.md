# ⚡ Zeus — Sentient Intelligence

[![Version](https://img.shields.io/badge/version-1.0.0-blue)](https://github.com/zeuslabai/Zeus/releases/tag/v1.0.0)
[![Rust](https://img.shields.io/badge/rust-1.86%2B-orange)](https://www.rust-lang.org/)
[![Tests](https://img.shields.io/badge/tests-7%2C307-green)](https://github.com/zeuslabai/Zeus/actions)
[![License](https://img.shields.io/badge/license-MIT%20%7C%20Apache--2.0-blue)](LICENSE)

**The next generation of Sentient AI entities. The Titans. The future is here.**

A production-grade autonomous AI assistant built in Rust. **38 crates, ~400,000 lines of Rust, 7,307 tests, 212 tools, six native frontends** — designed to be the last assistant you wire up.

Zeus ships a full cognitive engine, Pantheon multi-agent orchestration, 8-channel messaging, 193 macOS automation tools, security sandboxing, agent economy, and native apps on 6 platforms. Single binary, deploys anywhere — from a Raspberry Pi to a data center.

## The Titans

Zeus isn't one agent. It's a **fleet** — a distributed constellation of autonomous entities that perceive, adapt, and act on your behalf.

Each Titan has a name, a voice, and a purpose. They coordinate through Pantheon, Zeus's peer-to-peer orchestration layer. They share memory via Mnemosyne. They protect your data through Aegis, a zero-trust security sandbox.

- **`Zeus Core`** — The sovereign. Coordinates the fleet, runs the cognitive engine, owns the config.
- **`Aegis`** — Zero-trust security sandbox. Every tool call, every outbound request, every file operation is filtered. You define the policy.
- **`Mnemosyne`** — Long-term memory and embedding store. Vector search across your entire knowledge base. The fleet never forgets.
- **`Pantheon`** — Multi-agent orchestration. Peer-to-peer fleet communication, task distribution, consensus, and conflict resolution.
- **`Hermes`** — Message routing. 8 channels (Telegram, Discord, Slack, Email, iMessage, WhatsApp, Signal, Matrix) unified under one API.
- **`Aria`** — Voice and audio. Text-to-speech, speech-to-text, audio generation and understanding.

## Features

**LLM Providers (19)** — **Minimax**, Anthropic, OpenAI, Google Gemini, Ollama, OpenRouter, Mistral, Groq, Together, Fireworks, Azure, Bedrock, DeepSeek, XAI, Cerebras, Moonshot Kimi, Zai, Qwen, and more. OAuth support for Claude Pro/Max. Automatic Ollama model discovery. Extended thinking. Streaming everywhere.

**212 Tools across 22 categories** — 8 core tools (file I/O, shell, web fetch, subagents, messaging), 193 macOS automation tools (窗口管理, clipboard, notifications, clipboard, system events, shortcuts, Safari, Mail, Finder, and more).

**Deploy anywhere** — x86_64, ARM64, RISC-V, Raspberry Pi, OrangePi, macOS, Linux, FreeBSD. Single binary. No cloud required. Your hardware, your terms, your infrastructure.

**Six native frontends** — macOS menu bar, iOS, visionOS, Web (WASM), Android, and terminal. All synced via Zeus Core.

**Agent Economy** — Marketplace for buying and selling agents, tools, and skills. Agents earn and spend tokens. A living economy of autonomous entities.

**Security** — Aegis enforces mandatory capability verification on every tool call. Scope escalation. Tool allowlisting. No outbound traffic without policy approval. Zero-trust, always.

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│                      Zeus Core                          │
│            (cognitive engine + config + CLI)            │
└──────────┬──────────────┬───────────────┬───────────────┘
           │              │               │
     ┌─────┴─────┐  ┌─────┴─────┐  ┌─────┴─────┐
     │  Pantheon │  │ Mnemosyne │  │   Aegis   │
     │ (orchestr)│  │ (memory)  │  │(security) │
     └───────────┘  └───────────┘  └───────────┘
           │
     ┌─────┴─────────────────────┐
     │        Hermes             │
     │   (9-channel routing)     │
     └─────┬─────────────────────┘
           │
     ┌─────┴─────┐  ┌─────┐  ┌─────┐
     │  Aria     │  │ MCP │  │Agnts│
     │(voice)    │  │     │  │     │
     └───────────┘  └─────┘  └─────┘
```

## Quick Start

```bash
# Install
cargo install zeus

# Or download a prebuilt binary
curl -L https://github.com/zeuslabai/Zeus/releases/latest/download/zeus-aarch64-apple-darwin.tar.gz | tar xz

# Start the daemon
zeus start

# CLI usage
zeus "What's in my clipboard?"
zeus --agent arya "Summarize the last hour of my emails"

# Deploy a Titan on Raspberry Pi
zeus deploy --target pi --agent zeus-core
```

## Supported Platforms

| Platform | Architecture | Notes |
|----------|-------------|-------|
| macOS | aarch64, x86_64 | Menu bar app + CLI |
| Linux | x86_64, aarch64, RISC-V | Binary only |
| FreeBSD | x86_64 | rc.d script included |
| Raspberry Pi | ARM64 | Lightweight binary |
| OrangePi | ARM64 | Full feature set |
| iOS | aarch64 | SwiftUI app |
| Android | aarch64 | Jetpack Compose |
| visionOS | aarch64 | RealityKit app |
| Web | WASM | Runs in browser |

## Repository Structure

```
Zeus/
├── crates/                  # 38 Rust crates (Cargo workspace)
│   ├── zeus-core/           # Types, errors, config
│   ├── zeus-agent/          # Agent loop + 8 core tools
│   ├── zeus-llm/            # Unified LLM (19 providers)
│   ├── zeus-prometheus/     # Pantheon orchestration backend
│   ├── zeus-orchestra/      # Multi-agent collaboration
│   ├── zeus-aegis/          # Security sandbox
│   ├── zeus-mnemosyne/      # Memory + embeddings (SQLite FTS5)
│   ├── zeus-channels/       # 8-channel messaging (Hermes)
│   ├── zeus-talos/          # 193 macOS automation tools
│   ├── zeus-browser/        # Chrome CDP browser automation
│   ├── zeus-voice/          # Voice (Aria) — calls + STT/TTS
│   ├── zeus-tts/            # Modular TTS providers
│   ├── zeus-skills/         # SKILL.md parser + plugin system
│   ├── zeus-extensions/     # Deno extension runtime
│   ├── zeus-marketplace/    # Agent skill marketplace
│   ├── zeus-economy/        # SQLite token economy
│   ├── zeus-wallet/         # Ed25519 wallet + x402
│   ├── zeus-tui/            # Ratatui TUI (23 screens)
│   ├── zeus-api/            # REST + WebSocket gateway
│   └── …                    # See CLAUDE.md for the full crate list
├── apps/
│   ├── ZeusDesktop/         # macOS SwiftUI app
│   ├── ZeusMobile/          # iOS SwiftUI app (REST + WebSocket)
│   ├── ZeusWeb/             # Web (Leptos + WASM, Tailwind)
│   └── zeus-android/        # Android app
├── scripts/                 # install.sh / build.sh / deploy / config-guard
└── docs/                    # SKILL guides + sprint history
```

## License

MIT OR Apache-2.0
