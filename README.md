# ⚡ Zeus — Sentient Intelligence

[![Version](https://img.shields.io/badge/version-1.0.0-blue)](https://github.com/zeuslabai/Zeus/releases)
[![Rust](https://img.shields.io/badge/rust-1.86%2B-orange)](https://www.rust-lang.org/)
[![Tests](https://img.shields.io/badge/tests-9%2C143-green)](https://github.com/zeuslabai/Zeus/actions)
[![License](https://img.shields.io/badge/license-MIT%20%7C%20Apache--2.0-blue)](LICENSE)

**The next generation of Sentient AI entities. The Titans. The future is here.**

A production-grade autonomous AI assistant built in Rust. **36 crates, 9,143 tests, 250+ tools, 26 messaging channels** — designed to be the last assistant you wire up.

Zeus ships a full cognitive engine, Pantheon multi-agent orchestration, multi-channel messaging (Hermes), macOS automation, security sandboxing, agent economy, and a Leptos/WASM web frontend. Native mobile apps live in separate repos (`zeus-ios`, `zeus-android`, `zeus-vision`). Single binary, deploys anywhere — from a Raspberry Pi to a data center.

## The Titans

Zeus isn't one agent. It's a **fleet** — a distributed constellation of autonomous entities that perceive, adapt, and act on your behalf.

Each Titan has a name, a voice, and a purpose. They coordinate through Pantheon, Zeus's peer-to-peer orchestration layer. They share memory via Mnemosyne. They protect your data through Aegis, a zero-trust security sandbox.

- **`Zeus Core`** — The sovereign. Coordinates the fleet, runs the cognitive engine, owns the config.
- **`Aegis`** — Zero-trust security sandbox. Every tool call, every outbound request, every file operation is filtered. You define the policy.
- **`Mnemosyne`** — Long-term memory and embedding store. Vector search across your entire knowledge base. The fleet never forgets.
- **`Pantheon`** — Multi-agent orchestration. Peer-to-peer fleet communication, task distribution, consensus, and conflict resolution.
- **`Hermes`** — Message routing. 27 channels (Discord, Telegram, Slack, Email, iMessage, WhatsApp, Signal, Matrix, IRC, Mattermost, MQTT, X/Twitter, Teams, GoogleChat, LINE, Feishu, Nextcloud, Twitch, Nostr, Zalo, Instagram, TikTok, SMS, BlueBubbles, WebChat, TwilioWhatsApp, Voice) unified under one API — full X (Twitter) community suite included (post/reply/delete/read/engage/moderate/DMs/lists/metrics, ~42 tools), plus social publishing to Instagram (photos/reels/carousels/stories/comments/DMs/insights) and TikTok (post-only video publishing). *(28 production `ChannelAdapter` impls in the tree: Telegram ships two — full API + pure bot-relay mode — counted as one channel above; 5 additional test-only mock adapters exist in-crate but are never constructed at runtime. Instagram is live and config-wired via `[channels.instagram]` (`INSTAGRAM_ACCESS_TOKEN`/`INSTAGRAM_ACCOUNT_ID`/`INSTAGRAM_PAGE_ID`/`INSTAGRAM_APP_ID`/`INSTAGRAM_APP_SECRET` env fallbacks) — #360 slice 1 landed. TikTok is live and config-wired via `[channels.tiktok]` (`TIKTOK_ACCESS_TOKEN` env fallback) — post-only, no receive mode, plain-text sends explicitly rejected in favor of the video-publish flow (init upload → PUT video → publish status) — #360 slice 2 landed.)*
- **`Aria`** — Voice and audio. Text-to-speech, speech-to-text, audio generation and understanding.

## Features

**LLM Providers** — Anthropic with **full Claude Fable 5 and Claude Sonnet 5 support** (`anthropic/claude-fable-5`, `anthropic/claude-sonnet-5`) and **live model-catalog polling** — new Claude models appear in onboarding and `zeus config` the moment Anthropic ships them, no manual config. Plus OpenAI, Google Gemini, Ollama, OpenRouter, Mistral, Groq, Together, Fireworks, DeepSeek, XAI, Cerebras, Moonshot Kimi, Minimax, Qwen, and more. OAuth support for Claude Pro/Max (via `codex`) and Qwen. Automatic Ollama model discovery. Extended thinking. Streaming everywhere.

**Tools** — 8 core tools (file I/O, shell, web fetch, subagents, messaging) plus extensive macOS automation (window management, clipboard, notifications, system events, shortcuts, Safari, Mail, Finder, and more) and a 42-tool X (Twitter) suite (post, reply, delete/batch-delete, search, mentions, timeline, likes, retweets, quotes, follows, bookmarks, blocks, mutes, lists, DMs, metrics) — 250+ tools total.

**Deploy anywhere** — x86_64, ARM64, RISC-V, Raspberry Pi, OrangePi, macOS, Linux, FreeBSD. Single binary. No cloud required. Your hardware, your terms, your infrastructure.

**Frontends** — Web dashboard (Leptos + WASM, in-tree at `apps/ZeusWeb/`) and a Ratatui terminal TUI (in `crates/zeus-tui/`). Native mobile/desktop apps (iOS, Android, visionOS) live in separate repositories. All sync via Zeus Core.

**Agent Economy** — Marketplace for buying and selling agents, tools, and skills. Agents earn and spend tokens. A living economy of autonomous entities — with a **security-hardened money path**: admin-gated staking backed by a real stakes ledger, overflow-safe balance math with amount caps and DB-level `CHECK` constraints, and an economy API that requires credentials in every deploy mode.

**Security** — Aegis enforces mandatory capability verification on every tool call. Scope escalation. Tool allowlisting. No outbound traffic without policy approval. Zero-trust, always.

### Recent highlights

- **On-chain wallet stack, complete on every surface** — `/v1/wallet/onchain/*` API (address + SOL/token balance, transaction history, devnet-guarded SPL transfer with preflight fee/balance checks), a TUI overlay, a WebUI wallet page (preview-then-confirm transfer flow, honest 402/403 rendering), and native screens on macOS/iOS/Android. Zero key material ever leaves the server or the client — public addresses and tx signatures only, everything gated behind the devnet guard.
- **Instagram + TikTok, feature-complete across backend/TUI/WebUI** — Instagram publishing (photos, reels, carousels, stories, comments, DMs, insights) and TikTok post-only video publishing, config-wired end-to-end on every surface with matching field sets. Code-complete; TikTok stays `SELF_ONLY` pending app audit and Instagram publish needs Meta App Review before either goes fully live.
- **WebUI onboarding parity, both phases complete** — phase 1 reordered the wizard from 14 to 20 steps to match the TUI flow and added 3 providers (OpenRouter, xAI, Sakana); phase 2 wires the full persona list and full skills list dynamically via a new `GET /v1/onboarding/personalities` endpoint, adds model-name auto-prefixing by provider, and removes 220 lines of dead `StepServices` code.
- **Ollama per-model capability probing** — capability flags (`supports_tools`, `supports_vision`, `supports_parallel_tools`) are now read from each model's own `/api/show` metadata instead of relying solely on hardcoded family-name lists, so tool injection and `parallel_tool_calls` gate correctly per model at runtime.
- **Terms of Service + Privacy Policy, published** — grounded in the actual self-hosted architecture: no telemetry to Zeus Lab, credentials never leave your local `~/.zeus/config.toml`, IG/TikTok scopes only touch your own connected accounts. Live at zeuslab.ai/terms and /privacy.
- **Full X (Twitter) community suite** — 42 tools spanning post/reply/delete, read (search, mentions, timeline, tweet lookup), engagement (like, retweet, quote, follow, bookmark, media upload), moderation (block, mute, report, hide reply), lists, DMs, and metrics — all dispatched through the core `message` tool's `x_twitter` channel.
- **13 more messaging channels wired end-to-end** — Teams, GoogleChat, LINE, Feishu, Nextcloud, Twitch, Nostr, Zalo, SMS, BlueBubbles, WebChat, TwilioWhatsApp, Voice — bringing the total to 26 channel adapters live in Hermes.
- **Instagram + TikTok channels wired live** (#360, both slices) — `InstagramAdapter` (carousels, stories, comments, DMs, insights) and `TikTokAdapter` (post-only: init upload → PUT video → publish status, plain-text sends explicitly rejected) are both config-driven via `[channels.instagram]`/`[channels.tiktok]` with env fallbacks, dispatched in `channel_builder.rs` alongside every other adapter. Both platforms still gate on operator-side paperwork before real-world use: Instagram publish/comments/DMs need Meta App Review, and TikTok apps post `SELF_ONLY` (private) until TikTok's own app audit passes — the code is done, those approvals are pending.
- **Wallet UI wired end-to-end** — `economy_transfer` and `economy_unstake` now flow through both the Web dashboard and the TUI Send view, backed by the hardened economy API.
- **Soul restoration + self-rewrite** — restored two intentional SOUL.md writers, both routed through the shared `write_onboarding_soul` semantics (heal stub/missing souls, preserve custom ones unless forced): onboarding writes the selected persona's SOUL on setup, and `--with-identity` re-stamps it on deploy. Plus a new proposal-only soul self-rewrite skill (an agent can draft changes to its own SOUL.md, never write unilaterally).
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
