# Crate Map

The Zeus workspace contains 21 crates totaling approximately 59,400 lines of Rust. This page lists every crate with its size, purpose, and key exports.

## Full Listing

```
crates/
├── zeus-core/        1,503 lines   Types, errors, config, Message/ToolSchema types
├── zeus-llm/         2,677 lines   Unified LLM (11 providers)
├── zeus-memory/        477 lines   Workspace file-based memory
├── zeus-session/     1,254 lines   JSONL session storage + context manager + reset policies
├── zeus-agent/       3,614 lines   Agent loop + 8 core tools + subagents + subsystem wiring
├── zeus-tui/         4,777 lines   Ratatui TUI (10 screens)
├── zeus-api/         1,662 lines   REST API gateway + WebSocket streaming
├── zeus-mcp/           985 lines   Model Context Protocol support
├── zeus-nous/        3,412 lines   Cognitive engine (intent, reasoning, learning, autonomy)
├── zeus-prometheus/  6,868 lines   Brain/orchestration (planner, executor, heartbeat, cron, cooking loop)
├── zeus-channels/    6,102 lines   Messaging adapters (8 adapters)
├── zeus-hermes/        302 lines   Notification router
├── zeus-mnemosyne/   1,387 lines   Advanced memory with SQLite FTS5 + vector embeddings
├── zeus-athena/      1,548 lines   Documentation engine (Obsidian, Apple Notes)
├── zeus-aegis/       3,074 lines   Security sandboxing (Seatbelt, approvals, audit)
├── zeus-talos/      13,283 lines   macOS automation (193 tools across 22 categories)
├── zeus-browser/     1,425 lines   Chrome CDP browser automation (11 tools)
├── zeus-skills/      1,936 lines   SKILL.md parser + plugin system (OpenClaw compatibility)
├── zeus-voice/       1,941 lines   Voice calls (Twilio) + STT (Whisper, Groq) + TTS (OpenAI, macOS say)
├── zeus-ffi/         1,125 lines   UniFFI bindings for Swift (Desktop app)
└── zeus-agora/         486 lines   Agent skill marketplace (listings, transactions, wallets, protocol)
                     ─────────────
                     ~59,400 lines Rust + ~2,300 lines Swift
```

## Crate Details

### zeus-core (1,503 lines)

Foundation types shared by every other crate. Defines `Message`, `ToolCall`, `ToolResult`, `ToolSchema`, `Config`, and the `ZeusError` enum. All crates depend on zeus-core; it depends on nothing internal.

### zeus-llm (2,677 lines)

Unified LLM client that abstracts 11 providers behind a single `LlmClient` interface with streaming support. Provider selection is driven by the `provider/model-name` string in config. Supported providers: Anthropic, OpenAI, Ollama, OpenRouter, Google Gemini, Groq, Mistral, Together, Fireworks, Azure OpenAI, and AWS Bedrock.

### zeus-memory (477 lines)

Reads and writes the `~/.zeus/workspace/` directory tree: `AGENTS.md` (system prompt), `SOUL.md` (personality), `USER.md` (user context), `HEARTBEAT.md` (proactive tasks), `memory/MEMORY.md` (long-term facts), and `daily/YYYY-MM-DD.md` (daily notes). This is the simple, file-based memory layer. For advanced search, see zeus-mnemosyne.

### zeus-session (1,254 lines)

Persists conversation turns as JSONL files under `~/.zeus/sessions/`. Provides `ContextManager` for token-budget-aware context windowing and `ResetPolicy` for session lifecycle. Also contains the context journal subsystem that captures structured workflow state before context compaction.

### zeus-agent (3,614 lines)

The central hub. `Agent::run()` is the main loop: build context, call LLM, execute tool calls, repeat. Exposes 8 core tools (read_file, write_file, edit_file, list_dir, shell, web_fetch, spawn, message). `Agent::with_subsystems()` wires in Nous, Mnemosyne, Athena, Aegis, Hermes, and Channels. The `spawn` tool launches background subagents for parallel work.

### zeus-tui (4,777 lines)

Terminal UI built on [Ratatui](https://ratatui.rs/). 10 screens: Chat, Tools, Memory, Agents, Status, Help, Settings, Teams, Extensions, and Sandbox. Handles keyboard input, streaming response rendering, and vim-mode keybindings.

### zeus-api (1,662 lines)

Axum-based HTTP server exposing 95 REST endpoints plus WebSocket streaming. Includes an OpenAI-compatible `/v1/chat/completions` endpoint. Launched via `zeus serve` or as part of the gateway daemon.

### zeus-mcp (985 lines)

Implements the [Model Context Protocol](https://modelcontextprotocol.io/) for connecting external tool servers. Zeus can consume tools from MCP servers and expose its own tools as an MCP server.

### zeus-nous (3,412 lines)

Cognitive engine providing intent recognition, reasoning chains, meta-cognition, and learning from interactions. Its output is injected into the system prompt before each LLM call, giving the model awareness of its own reasoning state. Also contains the `CriticEngine` for rule-based execution evaluation and the `ConsolidationEngine` for background pattern extraction.

### zeus-prometheus (6,868 lines)

Orchestration layer that sits above the agent. For simple messages, it delegates directly to `agent.run()`. For complex tasks, it decomposes them with a planner, then executes steps through the **cooking loop** -- an iterative cycle of LLM calls and tool executions with automatic context injection from Mnemosyne. Also provides a heartbeat scheduler for proactive tasks, a cron engine with SQLite persistence, a `GoalStack` for persistent goal hierarchy, and a `FeedbackLoop` for strategy learning.

### zeus-channels (6,102 lines)

Eight messaging platform adapters unified behind the `ChannelAdapter` trait. Each adapter uses the platform's real SDK or protocol:

| Adapter   | Protocol                                  |
|-----------|-------------------------------------------|
| Telegram  | grammers-client (MTProto)                 |
| Discord   | serenity (Gateway + HTTP)                 |
| Slack     | reqwest (Web API) + tokio-tungstenite (Socket Mode) |
| Email     | lettre (SMTP) + async-imap (IMAP IDLE)   |
| iMessage  | AppleScript bridge (macOS only)           |
| WhatsApp  | Cloud API via reqwest                     |
| Signal    | signal-cli (JSON-RPC subprocess)          |
| Matrix    | matrix-sdk v0.16 (native Rust)            |

`ChannelManager` routes outbound messages and collects inbound messages via mpsc channels. Additional features include message chunking, streaming delivery, channel policies, a media pipeline, and a pairing manager.

### zeus-hermes (302 lines)

Lightweight notification router. Sends alerts on errors and task completions through configured channels. The smallest crate in the workspace.

### zeus-mnemosyne (1,387 lines)

Advanced memory backed by SQLite. Provides FTS5 full-text search and vector embeddings (via Ollama nomic-embed-text or OpenAI text-embedding-3-small) for semantic search. Hybrid search merges BM25 FTS5 scores with cosine similarity using a weighted blend. Supports Working, Episodic, and Semantic memory types with a hierarchical search strategy.

### zeus-athena (1,548 lines)

Documentation engine that logs tool executions, messages, and responses as structured actions. Generates Obsidian-flavored markdown with cross-reference links. Integrates with Apple Notes on macOS. Provides session summarization for long conversations.

### zeus-aegis (3,074 lines)

Security subsystem. Enforces macOS Seatbelt sandbox profiles, filters shell commands against an allowlist, validates web_fetch URLs, restricts file paths, and manages an approval workflow for sensitive operations. All decisions are audit-logged.

### zeus-talos (13,283 lines)

The largest crate. Provides 193 macOS automation tools organized across 22 categories, all driven by AppleScript. Categories include System (43 tools), Files (13), Git (15), Calendar (7), Notes (9), Reminders (8), Contacts (6), Safari (14), Mail (10), iMessage (8), Music (10), UI Automation (15), PDF (5), Bluetooth (6), Defaults (6), Network (3), Telegram (5), Search (1), Voice (2), and Homebrew (4).

### zeus-browser (1,425 lines)

Chrome DevTools Protocol automation. Connects to headless or visible Chrome and exposes 11 tools: navigate, click, type, get_text, screenshot, execute_js, console_logs, network_intercept, performance_metrics, scroll, and wait.

### zeus-skills (1,936 lines)

Parses `SKILL.md` files (a declarative format for defining agent capabilities), manages a permission system for skill execution, and integrates with the ClawHub registry for OpenClaw-compatible plugin distribution.

### zeus-voice (1,941 lines)

Voice calls via the `VoiceCallProvider` trait (Twilio implementation for outbound calls, DTMF, and incoming webhooks). Speech-to-text through Whisper (local) and Groq (cloud). Text-to-speech through OpenAI TTS and the macOS `say` command.

### zeus-ffi (1,125 lines)

UniFFI scaffolding that exposes selected Rust APIs to Swift. Used by the ZeusDesktop macOS app to call into the Rust core directly (no network round-trip). Built via `scripts/build-zeus-ffi.sh` which produces a universal binary and XCFramework.

### zeus-agora (486 lines)

Agent skill marketplace. Provides four core components: `SkillListing` for agents to advertise capabilities with pricing, input/output schemas, and performance statistics; `SkillTransaction` for end-to-end purchase tracking through states (Pending, InProgress, Completed, Failed, Refunded); `AgentWallet` for credit balances with spend/earn operations and lifetime totals; and `AgentIdentity`/`AgentCapability` types for HTTP-based agent discovery and inter-agent protocol negotiation.
