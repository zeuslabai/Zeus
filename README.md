# ⚡ Zeus — Sentient Intelligence

[![Version](https://img.shields.io/badge/version-1.0.0-blue)](https://github.com/zeuslabai/Zeus/releases)
[![Rust](https://img.shields.io/badge/rust-1.86%2B-orange)](https://www.rust-lang.org/)
[![Tests](https://img.shields.io/badge/tests-8%2C024-green)](https://github.com/zeuslabai/Zeus/actions)
[![License](https://img.shields.io/badge/license-MIT%20%7C%20Apache--2.0-blue)](LICENSE)

**The next generation of Sentient AI entities. The Titans. The future is here.**

A production-grade autonomous AI assistant built in Rust. **36 crates, 8,024 tests, 200+ tools, 23 messaging channels** — designed to be the last assistant you wire up.

Zeus ships a full cognitive engine, Pantheon multi-agent orchestration, multi-channel messaging (Hermes), macOS automation, security sandboxing, agent economy, and a Leptos/WASM web frontend. Native mobile apps live in separate repos (`zeus-ios`, `zeus-android`, `zeus-vision`). Single binary, deploys anywhere — from a Raspberry Pi to a data center.

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

**LLM Providers** — Anthropic with **full Claude Fable 5 and Claude Sonnet 5 support** (`anthropic/claude-fable-5`, `anthropic/claude-sonnet-5`) and **live model-catalog polling** — new Claude models appear in onboarding and `zeus config` the moment Anthropic ships them, no manual config. Plus OpenAI, Google Gemini, Ollama, OpenRouter, Mistral, Groq, Together, Fireworks, DeepSeek, XAI, Cerebras, Moonshot Kimi, Minimax, Qwen, and more. OAuth support for Claude Pro/Max (via `codex`) and Qwen. Automatic Ollama model discovery. Extended thinking. Streaming everywhere.

**Tools** — 8 core tools (file I/O, shell, web fetch, subagents, messaging) plus extensive macOS automation (window management, clipboard, notifications, system events, shortcuts, Safari, Mail, Finder, and more).

**Deploy anywhere** — x86_64, ARM64, RISC-V, Raspberry Pi, OrangePi, macOS, Linux, FreeBSD. Single binary. No cloud required. Your hardware, your terms, your infrastructure.

**Frontends** — Web dashboard (Leptos + WASM, in-tree at `apps/ZeusWeb/`) and a Ratatui terminal TUI (in `crates/zeus-tui/`). Native mobile/desktop apps (iOS, Android, visionOS) live in separate repositories. All sync via Zeus Core.

**Agent Economy** — Marketplace for buying and selling agents, tools, and skills. Agents earn and spend tokens. A living economy of autonomous entities — with a **security-hardened money path**: admin-gated staking backed by a real stakes ledger, overflow-safe balance math with amount caps and DB-level `CHECK` constraints, and an economy API that requires credentials in every deploy mode.

**Security** — Aegis enforces mandatory capability verification on every tool call. Scope escalation. Tool allowlisting. No outbound traffic without policy approval. Zero-trust, always.

### Recent highlights

- **Claude Fable 5 & Sonnet 5, day-one** — full support for Anthropic's newest flagship models, with live `/v1/models` catalog polling so future releases show up automatically.
- **Hardened token economy** — fleet-audited money path: staking backed by an atomic ledger (no free-mint), integer-overflow-safe balances, credential-gated economy API, plus wallet transaction history and an active-stakes API surfaced in the WebUI.
- **Smarter gateway rate limiting** — local TUI/WebUI/CLI traffic (loopback) is exempt, and remote limits are tuned for real agentic workloads.
- **TUI production polish** — settings wired to live config, one-press tab navigation, resilient streaming chat with live tool feed, API-key validation on onboarding, and credential persistence fixes across providers.
- **WebUI at full wiring** — 57 pages, effectively all driven by the live gateway (REST + WebSocket), with a polish pass across empty states and error messages.

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
     │   (multi-channel routing) │
     └─────┬─────────────────────┘
           │
     ┌─────┴─────┐  ┌─────┐  ┌─────┐
     │  Aria     │  │ MCP │  │Agnts│
     │(voice)    │  │     │  │     │
     └───────────┘  └─────┘  └─────┘
```

## Quick Start

```bash
# Install (recommended — Universal Installer)
curl -fsSL https://raw.githubusercontent.com/zeuslabai/Zeus/main/scripts/install.sh | bash

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
├── crates/                  # 36 Rust crates (Cargo workspace)
│   ├── zeus-core/           # Types, errors, config
│   ├── zeus-agent/          # Agent loop + 8 core tools
│   ├── zeus-llm/            # Unified LLM (multi-provider)
│   ├── zeus-prometheus/     # Pantheon orchestration backend
│   ├── zeus-orchestra/      # Multi-agent collaboration
│   ├── zeus-aegis/          # Security sandbox
│   ├── zeus-mnemosyne/      # Memory + embeddings (SQLite FTS5)
│   ├── zeus-channels/       # Multi-channel messaging (Hermes)
│   ├── zeus-talos/          # macOS automation tools
│   ├── zeus-browser/        # Chrome CDP browser automation
│   ├── zeus-voice/          # Voice (Aria) — calls + STT/TTS
│   ├── zeus-tts/            # Modular TTS providers
│   ├── zeus-skills/         # SKILL.md parser + plugin system
│   ├── zeus-extensions/     # Deno extension runtime
│   ├── zeus-marketplace/    # Agent skill marketplace
│   ├── zeus-economy/        # SQLite token economy
│   ├── zeus-wallet/         # Ed25519 wallet + x402
│   ├── zeus-tui/            # Ratatui TUI (7 screens)
│   ├── zeus-api/            # REST + WebSocket gateway
│   └── …                    # See CLAUDE.md for the full crate list
├── apps/
│   └── ZeusWeb/             # Web dashboard (Leptos + WASM, Tailwind)
│                            # Mobile/desktop apps live in separate repos:
│                            #   zeus-ios, zeus-android, zeus-vision
├── scripts/                 # install.sh / uninstall.sh / update.sh + packaging
└── docs/                    # SKILL guides + sprint history
```

## License

MIT OR Apache-2.0
