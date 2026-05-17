# Introduction

Zeus is a full-featured autonomous AI assistant built in Rust. It combines persistent file-based memory, a unified LLM provider supporting 11 backends, 8 core tools, and a rich set of advanced subsystems into a single, cohesive agent loop. The project ships with three frontends -- a terminal UI, a macOS desktop app, and an iOS mobile app -- making it usable from the command line, the desktop, or on the go.

## Key Statistics

| Metric | Value |
|--------|-------|
| Lines of Rust | ~59,400 |
| Lines of Swift | ~2,300 |
| Workspace crates | 20 |
| Tests | 1,711 |
| Tools | 212 |
| LLM providers | 11 |
| Channel adapters | 8 |
| API routes | 95+ |
| Frontends | 3 (TUI, macOS Desktop, iOS) |

## Feature Highlights

- **Persistent Memory** -- File-based workspace memory with SQLite FTS5 full-text search and vector embeddings for semantic recall.
- **8 Core Tools** -- `read_file`, `write_file`, `edit_file`, `list_dir`, `shell`, `web_fetch`, `spawn` (background subagents), and `message` (channel routing).
- **Unified LLM Provider** -- A single interface supporting Anthropic, OpenAI, Ollama, OpenRouter, Google, Groq, Mistral, Together, Fireworks, Azure, and AWS Bedrock.
- **3 Frontends** -- A Ratatui-based TUI with 10 screens, a SwiftUI macOS desktop app with UniFFI bindings to Rust, and a SwiftUI iOS app connecting via REST and WebSocket.
- **Cognitive Engine (Nous)** -- Intent recognition, reasoning chains, meta-cognition, and learning from interactions, injected into the system prompt before every LLM call.
- **Multi-Channel Chat** -- 8 messaging adapters (Telegram, Discord, Slack, Email, iMessage, WhatsApp, Signal, Matrix) wired into the agent loop via the `message` tool.
- **Documentation Engine (Athena)** -- Automatic action logging, session summarization, Obsidian markdown generation, and Apple Notes integration.
- **Security Sandboxing (Aegis)** -- macOS Seatbelt profiles, command filtering, URL allowlisting, path restrictions, an approval system for sensitive operations, and audit logging.
- **macOS Automation (Talos)** -- 193 AppleScript-based tools across 22 categories covering system control, file management, git, calendar, contacts, Safari, Mail, iMessage, Music, UI automation, PDF manipulation, Bluetooth, network diagnostics, and more.
- **Browser Automation** -- Chrome DevTools Protocol integration with 11 tools for navigation, interaction, screenshots, JavaScript execution, and performance profiling.
- **Voice Calls** -- Twilio telephony provider with speech-to-text (Whisper, Groq) and text-to-speech (OpenAI, macOS `say`).
- **Autonomous Tool Execution (Cooking Loop)** -- The Prometheus orchestration layer decomposes complex tasks into plans, executes them iteratively with automatic context injection, and learns from outcomes via a feedback loop.

## How It Works

Zeus follows a straightforward data flow:

1. User input arrives through any frontend (TUI, API, CLI, Desktop, or iOS).
2. The agent builds context from the workspace, cognitive engine, and memory recall.
3. The security layer validates permissions before any tool execution.
4. The agent calls the configured LLM with streaming (5-minute timeout).
5. Tool calls in the LLM response are executed and results fed back.
6. Actions are logged, the session is persisted, and notifications are sent on errors or completions.
7. The response streams back to the user.

For complex, multi-step tasks, the Prometheus orchestration layer sits above the agent loop. It can plan, decompose, and iteratively execute tool chains with automatic retries and context injection -- the "cooking loop."

## Project Layout

The codebase is organized as a Cargo workspace with 20 crates. Each crate has a focused responsibility, from core types and LLM abstraction to security sandboxing and macOS automation. See the [Architecture Overview](./architecture/README.md) chapter for the full crate map and subsystem wiring.
