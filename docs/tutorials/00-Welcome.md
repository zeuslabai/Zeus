# Welcome to Zeus Tutorials

Zeus is an autonomous AI assistant built in Rust with 212 tools, 9 messaging channels, multi-agent orchestration, and native apps on 6 platforms. These tutorials walk you through every major workflow — from first install to running a multi-agent mission.

## How to Use These Tutorials

Each tutorial is self-contained and covers one workflow. They're numbered for suggested reading order, but you can jump to any topic. If you're using Obsidian, the `[[wikilinks]]` let you navigate between tutorials.

### Getting Started
- [[01-Installation]] — Build from source, install script, verify
- [[02-First-Run]] — Setup wizard, first chat, basic commands
- [[03-Configuration]] — config.toml deep dive, environment variables, workspace

### Daily Use
- [[04-Chat-and-Conversations]] — Single messages, streaming, sessions, context
- [[05-Tools]] — Using the 212 built-in tools from CLI and chat
- [[06-Memory]] — Workspace files, long-term facts, daily notes, search
- [[07-TUI]] — Terminal UI screens, keyboard shortcuts, vim mode

### Integrations
- [[08-API-Server]] — REST endpoints, WebSocket streaming, OpenAI compatibility
- [[09-Channels]] — Telegram, Discord, Slack, Email, iMessage, WhatsApp, Signal, Matrix
- [[10-MCP-Integration]] — Using Zeus as an MCP server with Claude Code
- [[11-Browser-Automation]] — Chrome DevTools Protocol, screenshots, page interaction

### Advanced
- [[12-Gateway]] — Production daemon with all subsystems
- [[13-Pantheon]] — Multi-agent missions, War Room chat, team assembly
- [[14-Skills]] — Browsing, creating, and publishing skills
- [[15-Security]] — Aegis sandbox, permissions, credential vault, audit
- [[16-Deployment]] — macOS launchd, FreeBSD rc.d, Linux systemd
- [[17-macOS-Automation]] — 193 Talos tools for calendar, notes, Mail, Music, Finder
- [[18-Cognitive-Engine]] — Nous intent recognition, learning, reasoning
- [[19-Native-Apps]] — macOS Desktop, iOS, visionOS, Web, Android frontends
- [[20-Troubleshooting]] — Common issues, zeus doctor, debugging

## Prerequisites

- **Rust 1.86+** via [rustup](https://rustup.rs/)
- **macOS 14+** (Sonoma) for full feature set (Linux/FreeBSD for server deployments)
- At least one LLM API key (Anthropic, OpenAI, OpenRouter) or a running Ollama instance

## Quick Links

- [GitHub Repository](https://github.com/zeuslabai/Zeus)
- [API Reference](../api-reference.md)
- [Quick Start](../quickstart.md)
