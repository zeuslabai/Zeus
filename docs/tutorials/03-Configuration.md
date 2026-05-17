# Configuration

Zeus stores all configuration in `~/.zeus/config.toml`. This tutorial covers every section.

## Config File Location

```
~/.zeus/
├── config.toml          # Main configuration
├── .env                 # API keys and secrets (auto-loaded)
├── workspace/           # Workspace files (AGENTS.md, SOUL.md, etc.)
├── sessions/            # Chat session history (JSONL)
├── memory.db            # Mnemosyne SQLite database
├── pantheon.db          # Pantheon mission database
└── audit.log            # Security audit log
```

## Core Settings

```toml
# LLM model (provider/model-name format)
model = "anthropic/claude-sonnet-4-20250514"

# File paths
workspace = "~/.zeus/workspace"
sessions = "~/.zeus/sessions"

# Agent behavior
max_iterations = 20           # Max tool-use loops per request
max_subagent_iterations = 15  # Max iterations for spawned subagents
```

### Extended Thinking

For models that support it (Claude Sonnet/Opus), enable extended thinking:

```toml
thinking_level = "medium"   # "low", "medium", "high", "xhigh"
```

## TUI Settings

```toml
[tui]
theme = "dark"       # "dark" or "light"
vim_mode = false     # true = vim-style navigation in chat
```

## OAuth (Claude Pro/Max)

If you have a Claude Pro or Max subscription and want to use OAuth instead of API keys:

```toml
[auth]
use_oauth = false    # Set to true after running /login
```

## Ollama (Local Models)

```toml
[ollama]
url = "http://localhost:11434"
# preferred_model = "llama3.2"     # Auto-select this model
```

Zeus auto-discovers all models available on your Ollama instance.

## Subsystems

All subsystems are optional. Omit a section to disable it.

### Memory (Mnemosyne)

```toml
[mnemosyne]
db_path = "~/.zeus/memory.db"
enable_fts = true                           # Full-text search via SQLite FTS5
max_messages_per_session = 10000
# embedding_host = "http://gpu-box:11434"   # Pin embeddings to a GPU server
```

See [[06-Memory]] for usage.

### Documentation (Athena)

```toml
[athena]
vault_path = "~/Obsidian/Zeus"    # Obsidian vault for generated docs
```

### Security (Aegis)

```toml
[aegis]
sandbox_level = "standard"   # "none", "basic", "standard", "strict", "paranoid"
audit_path = "~/.zeus/audit.log"
permissions = ["*"]           # Tool permissions
network_allowlist = ["*"]     # URL allowlist
```

See [[15-Security]] for details.

### Cognitive Engine (Nous)

```toml
[nous]
enable_intent = true      # Intent recognition
enable_learning = true    # Learn from interactions
```

See [[18-Cognitive-Engine]].

### Orchestration (Prometheus)

```toml
[prometheus]
enable_heartbeat = false
heartbeat_interval_secs = 300
enable_cognitive = false
```

### Notifications (Hermes)

```toml
[hermes]
default_channels = ["console"]   # Where to send notifications
```

### macOS Automation (Talos)

```toml
[talos]
calendar = true
notes = true
reminders = true
contacts = true
browser = true
system = true
network = true
```

See [[17-macOS-Automation]].

### Web Search

```toml
[search]
provider = "duckduckgo"    # "brave" or "duckduckgo"
# api_key = "BSA..."       # Required for Brave Search
max_results = 5
```

### Session Compaction

```toml
[session_compaction]
max_context_tokens = 180000
compaction_threshold = 0.8    # Compact when 80% of context used
```

### Gateway

```toml
[gateway]
host = "127.0.0.1"
port = 8080
public_url = "https://gt.zeuslab.ai"
enable_channels = true
enable_cron = true
enable_heartbeat = true
enable_api = true
```

See [[12-Gateway]].

### MCP Server

```toml
[mcp_server]
enable_mnemosyne = true    # Enable memory in MCP mode
```

See [[10-MCP-Integration]].

### Hooks

```toml
[hooks]
logging = true
notifications = false
memory_save = false
```

## Environment Variables

Store secrets in `~/.zeus/.env` (auto-loaded):

```bash
ANTHROPIC_API_KEY=sk-ant-...
OPENAI_API_KEY=sk-...
OPENROUTER_API_KEY=sk-or-...
DISCORD_BOT_TOKEN=...
TELEGRAM_API_ID=...
TELEGRAM_API_HASH=...
TELEGRAM_PHONE=+1234567890
```

| Variable | Required For |
|----------|-------------|
| `ANTHROPIC_API_KEY` | Anthropic Claude models |
| `OPENAI_API_KEY` | OpenAI GPT models, Whisper STT |
| `OPENROUTER_API_KEY` | OpenRouter models |
| `OLLAMA_HOST` | Custom Ollama URL (default: localhost:11434) |
| `BRAVE_API_KEY` | Brave Search |
| `GROQ_API_KEY` | Groq TTS |

> ⚠️ **All secrets go in `~/.zeus/.env`** — never in `config.toml` or shell environment files. The `.env` file is auto-loaded by Zeus on startup.

## Workspace Files

The workspace at `~/.zeus/workspace/` contains files Zeus reads on every interaction:

```
workspace/
├── AGENTS.md        # System prompt — who Zeus is
├── SOUL.md          # Personality and style guidelines
├── USER.md          # User context and preferences
├── HEARTBEAT.md     # Proactive tasks for the heartbeat loop
├── memory/
│   └── MEMORY.md    # Long-term facts
└── daily/
    └── YYYY-MM-DD.md  # Daily notes
```

Edit these files to customize Zeus's behavior:
- **AGENTS.md** — Change the system prompt to define Zeus's role
- **SOUL.md** — Adjust personality (formal, casual, verbose, concise)
- **USER.md** — Tell Zeus about yourself (preferences, projects, tech stack)

## What's Next

→ [[04-Chat-and-Conversations]] — Start chatting with Zeus
→ [[12-Gateway]] — Run Zeus as a production daemon
