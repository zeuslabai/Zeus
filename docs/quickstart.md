# Zeus Quick-Start Guide

Get Zeus running in under 5 minutes. This guide covers installation, configuration, your first chat, tool execution, and launching the gateway.

## Prerequisites

- **Rust 1.86+** via [rustup](https://rustup.rs/)
- **An LLM API key** (Anthropic, OpenAI, Ollama, or any of the 11 supported providers)
- **macOS or Linux** (FreeBSD also supported for server deployments)

## Install

### From source (recommended)

```bash
git clone git@github.com:zeuslabai/Zeus.git
cd Zeus
cargo build --release
cp target/release/zeus /usr/local/bin/
```

### Using the install script

```bash
curl -sSL https://raw.githubusercontent.com/zeuslabai/Zeus/main/scripts/install.sh | bash
```

The installer downloads (or builds) the binary, places it in `/usr/local/bin/`, and optionally configures Zeus as an MCP server for Claude Code.

### Verify

```bash
zeus --version
zeus doctor
```

`zeus doctor` checks your config, workspace, credentials, Ollama connectivity, session health, and more (17 diagnostic checks).

## Configure

Zeus stores configuration at `~/.zeus/config.toml`. Create it with the onboarding wizard:

```bash
zeus onboard
```

Or create it manually:

```toml
# ~/.zeus/config.toml
model = "anthropic/claude-sonnet-4-20250514"
workspace = "~/.zeus/workspace"
sessions = "~/.zeus/sessions"
max_iterations = 20

[tui]
theme = "dark"
vim_mode = false
```

### Set your API key

API keys are read from environment variables. Add to your shell profile:

```bash
# Pick your provider
export ANTHROPIC_API_KEY="sk-ant-..."      # Anthropic Claude
export OPENAI_API_KEY="sk-..."             # OpenAI GPT
export GOOGLE_API_KEY="AIza..."            # Google Gemini
export GROQ_API_KEY="gsk_..."              # Groq
export OPENROUTER_API_KEY="sk-or-..."      # OpenRouter (multi-provider)
```

Or for local models, just install [Ollama](https://ollama.com/) and set:

```toml
model = "ollama/llama3.2"

[ollama]
url = "http://localhost:11434"
```

### Model string format

All models follow the `provider/model-name` pattern:

```
anthropic/claude-sonnet-4-20250514
openai/gpt-4o
ollama/llama3.2
google/gemini-2.0-flash
groq/llama-3.3-70b-versatile
openrouter/anthropic/claude-3.5-sonnet
```

## First chat

### Single message mode

```bash
zeus chat "What is the capital of France?"
```

With streaming output:

```bash
zeus chat -s "Explain how DNS works in 3 sentences"
```

### Execute a tool directly

```bash
zeus tool list_dir '{"path":"."}'
zeus tool shell '{"command":"uname -a"}'
zeus tool web_fetch '{"url":"https://httpbin.org/get"}'
```

Zeus ships with 8 core tools (`read_file`, `write_file`, `edit_file`, `list_dir`, `shell`, `web_fetch`, `spawn`, `message`) plus 193 macOS automation tools via Talos and 11 browser automation tools.

## Launch the TUI

```bash
zeus
```

The terminal UI has 10 screens: Chat, Tools, Memory, Agents, Status, Help, Settings, Teams, Extensions, and Sandbox. Navigate with Tab/Shift-Tab or number keys.

## Launch the API server

```bash
zeus serve              # Default port 8080
zeus serve -p 3000      # Custom port
```

Test it:

```bash
curl http://localhost:8080/health
curl http://localhost:8080/v1/status
curl -X POST http://localhost:8080/v1/chat \
  -H "Content-Type: application/json" \
  -d '{"message": "Hello Zeus!"}'
```

## Launch the gateway (production)

The gateway combines the API server, channel adapters (Telegram, Discord, Slack, etc.), heartbeat, and cron scheduler:

```bash
zeus gateway
```

Selectively disable subsystems:

```bash
zeus gateway --no-channels    # API + cron only
zeus gateway --no-cron        # API + channels only
zeus gateway --no-channels --no-cron  # API only
```

## Install as a system service

### macOS (launchd)

```bash
zeus daemon install
zeus daemon start
zeus daemon status
```

### FreeBSD (rc.d)

```bash
sudo cp scripts/freebsd/zeus /usr/local/etc/rc.d/
sudo sysrc zeus_enable="YES"
sudo service zeus start
```

## Optional: channel adapters

Add messaging integrations in `config.toml`:

```toml
[channels.telegram]
api_id = 12345
api_hash = "your_api_hash"
phone = "+1234567890"

[channels.discord]
token = "your_bot_token"

[channels.slack]
bot_token = "xoxb-..."
app_token = "xapp-..."
```

Zeus supports 8 channel adapters: Telegram, Discord, Slack, Email, iMessage, WhatsApp, Signal, and Matrix.

## Optional: advanced subsystems

All optional — Zeus works without them:

```toml
# Semantic memory with SQLite FTS5 + vector embeddings
[mnemosyne]
db_path = "~/.zeus/mnemosyne.db"
enable_fts = true
embedding_host = "http://gpu-box:11434"  # optional: pin embeddings to GPU server

# Documentation engine (Obsidian integration)
[athena]
vault_path = "~/Obsidian/Zeus"

# Security sandboxing
[aegis]
level = "standard"

# Cognitive engine
[nous]
enable_learning = true

# macOS automation (193 tools)
[talos]
enable_applescript = true

# Cron scheduler
[scheduler]
enabled = true
max_concurrent_jobs = 4
```

## What to try next

- **Memory**: `zeus memory show`, `zeus memory remember "I prefer Python"`, `zeus memory note "Today I set up Zeus"`
- **Sessions**: `zeus session list`, `zeus session show <id>`
- **Skills**: Browse 52 bundled skills at `GET /v1/skills` or in the TUI Extensions screen
- **API reference**: See [docs/api-reference.md](./api-reference.md) for curl examples of all major endpoints
- **MCP integration**: Zeus works as an MCP server for Claude Code — run `zeus onboard` to configure

## Architecture at a glance

Zeus is a 28-crate Rust workspace (~59K lines, 1700+ tests):

```
zeus-core         Types, errors, config
zeus-llm          11 LLM providers, unified streaming
zeus-agent        Agent loop, 8 core tools, subagents
zeus-prometheus   Orchestration, planning, cron, cooking loop
zeus-nous         Cognitive engine (intent, reasoning, learning)
zeus-mnemosyne    FTS5 + vector memory
zeus-channels     8 messaging adapters
zeus-talos        193 macOS automation tools
zeus-browser      Chrome CDP automation (11 tools)
zeus-aegis        Security sandboxing
zeus-skills       SKILL.md parser, OpenClaw compatibility
zeus-api          REST API gateway (200+ routes)
zeus-tui          Terminal UI (10 screens)
zeus-ffi          Swift bindings (macOS/iOS apps)
```

5 frontends: TUI, macOS Desktop, iOS, Android (planned), Web (Leptos/WASM).
