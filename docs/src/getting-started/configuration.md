# Configuration

Zeus is configured through a TOML file at `~/.zeus/config.toml`. The file is created automatically by `zeus onboard` or on first run with sensible defaults.

## Minimal Configuration

The only required setting is the model to use. Everything else has working defaults.

```toml
model = "anthropic/claude-sonnet-4-20250514"
```

## Model String Format

Models are specified as `provider/model-name`. The provider prefix tells Zeus which backend and API key to use:

| Provider | Example Model String |
|----------|---------------------|
| Anthropic | `anthropic/claude-sonnet-4-20250514` |
| OpenAI | `openai/gpt-4o` |
| Ollama | `ollama/llama3.2` |
| OpenRouter | `openrouter/anthropic/claude-3.5-sonnet` |
| Google | `google/gemini-2.0-flash` |
| Groq | `groq/llama-3.3-70b-versatile` |
| Mistral | `mistral/mistral-large-latest` |
| Together | `together/meta-llama/Meta-Llama-3.1-405B-Instruct-Turbo` |
| Fireworks | `fireworks/accounts/fireworks/models/llama-v3p1-405b-instruct` |
| Azure | `azure/gpt-4o` |
| Bedrock | `bedrock/anthropic.claude-3-5-sonnet-20241022-v2:0` |

Each provider requires its corresponding API key environment variable. See [Environment Variables](../configuration/environment-variables.md) for the full list.

## Key Settings

```toml
# LLM model (required)
model = "anthropic/claude-sonnet-4-20250514"

# Workspace directory for memory files
workspace = "~/.zeus/workspace"

# Session storage directory (JSONL files)
sessions = "~/.zeus/sessions"

# Maximum tool execution iterations per agent turn
max_iterations = 20
```

## TUI Settings

```toml
[tui]
theme = "dark"       # "dark" or "light"
vim_mode = false     # Enable vim-style keybindings
```

## Workspace Directory

Zeus stores its persistent memory and configuration in the workspace directory (default `~/.zeus/workspace/`):

```
~/.zeus/workspace/
  AGENTS.md       # System prompt
  SOUL.md         # Personality definition
  USER.md         # User context and preferences
  HEARTBEAT.md    # Proactive tasks for the heartbeat loop
  memory/
    MEMORY.md     # Long-term facts
  daily/
    YYYY-MM-DD.md # Daily notes
```

These files are read by the agent on every turn to build context. You can edit them directly or use the `zeus memory` commands:

```bash
zeus memory show                  # Display current context
zeus memory remember "I prefer dark mode"  # Add a fact to MEMORY.md
zeus memory note "Met with team"  # Add an entry to today's daily note
```

## Subsystem Configuration

Advanced subsystems are all optional. They are enabled by adding their configuration sections:

```toml
# Advanced memory with SQLite FTS5 and vector embeddings
[mnemosyne]
db_path = "~/.zeus/mnemosyne.db"
enable_fts = true

# Documentation engine
[athena]
vault_path = "~/Obsidian/Zeus"

# Security sandboxing
[aegis]
level = "standard"    # "off", "standard", or "strict"

# Notification routing
[hermes]
default_channel = "console"

# Cognitive engine
[nous]
enable_learning = true

# macOS automation
[talos]
enable_applescript = true
```

## Channel Configuration

Each messaging adapter has its own configuration block. For example:

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

[channels.email]
smtp_host = "smtp.gmail.com"
imap_host = "imap.gmail.com"
username = "you@gmail.com"
password = "app-password"
```

See the [Channels](../channels/README.md) section for per-adapter setup guides.

## Viewing Current Configuration

To inspect your active configuration (with secrets redacted):

```bash
zeus config
```

To include API keys in the output:

```bash
zeus config --show-secrets
```

## Full Reference

For the complete configuration schema including all subsystem options, see [config.toml Reference](../configuration/config-toml.md).
