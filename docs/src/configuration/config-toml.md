# config.toml Reference

The Zeus configuration file lives at `~/.zeus/config.toml`. All fields are optional and have sensible defaults.

## Top-Level Settings

```toml
model = "anthropic/claude-sonnet-4-20250514"
workspace = "~/.zeus/workspace"
sessions = "~/.zeus/sessions"
max_iterations = 20
```

| Field | Default | Description |
|-------|---------|-------------|
| `model` | `"anthropic/claude-sonnet-4-20250514"` | LLM model in `provider/model-name` format |
| `workspace` | `"~/.zeus/workspace"` | Path to workspace directory containing memory and prompt files |
| `sessions` | `"~/.zeus/sessions"` | Path to session storage directory (JSONL files) |
| `max_iterations` | `20` | Maximum tool-call iterations per agent run before stopping |

## Model String Format

The `model` field uses the format `provider/model-name`. Zeus supports 11 LLM providers:

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

The provider prefix determines which API client and authentication method Zeus uses. Everything after the first `/` is passed as the model name to that provider's API.

For OpenRouter, the model name itself may contain slashes (e.g., `openrouter/anthropic/claude-3.5-sonnet`).

## TUI Settings

```toml
[tui]
theme = "dark"
vim_mode = false
```

| Field | Default | Description |
|-------|---------|-------------|
| `theme` | `"dark"` | Color theme for the terminal UI |
| `vim_mode` | `false` | Enable vim-style keybindings in the TUI |

## Authentication

```toml
[auth]
use_oauth = false
```

| Field | Default | Description |
|-------|---------|-------------|
| `use_oauth` | `false` | Use OAuth flow for provider authentication instead of API keys |

## Ollama

```toml
[ollama]
url = "http://localhost:11434"
```

| Field | Default | Description |
|-------|---------|-------------|
| `url` | `"http://localhost:11434"` | URL of the Ollama server. Can also be set via `OLLAMA_HOST` env var |

## Mnemosyne (Advanced Memory)

```toml
[mnemosyne]
db_path = "~/.zeus/mnemosyne.db"
enable_fts = true
```

| Field | Default | Description |
|-------|---------|-------------|
| `db_path` | `"~/.zeus/mnemosyne.db"` | Path to the SQLite database for advanced memory |
| `enable_fts` | `true` | Enable FTS5 full-text search indexing |

Mnemosyne provides hybrid search combining BM25 full-text search with cosine-similarity vector embeddings for semantic recall.

## Athena (Documentation Engine)

```toml
[athena]
vault_path = "~/Obsidian/Zeus"
```

| Field | Default | Description |
|-------|---------|-------------|
| `vault_path` | `"~/Obsidian/Zeus"` | Path to the Obsidian vault for documentation output |

Athena logs tool executions, messages, and responses as actions, and generates Obsidian-compatible markdown with cross-reference linking.

## Aegis (Security)

```toml
[aegis]
level = "standard"
```

| Field | Default | Description |
|-------|---------|-------------|
| `level` | `"standard"` | Security level: controls sandboxing, command filtering, and approval requirements |

Aegis provides macOS Seatbelt sandboxing, shell command filtering, URL allowlisting, and an approval system for sensitive operations.

## Hermes (Notifications)

```toml
[hermes]
default_channel = "console"
```

| Field | Default | Description |
|-------|---------|-------------|
| `default_channel` | `"console"` | Default notification channel for errors and task completions |

## Nous (Cognitive Engine)

```toml
[nous]
enable_learning = true
```

| Field | Default | Description |
|-------|---------|-------------|
| `enable_learning` | `true` | Enable learning from interactions for improved intent recognition |

## Talos (macOS Automation)

```toml
[talos]
enable_applescript = true
```

| Field | Default | Description |
|-------|---------|-------------|
| `enable_applescript` | `true` | Enable AppleScript-based macOS automation tools (193 tools) |

## Channel Configuration

Channels are configured under `[channels.<name>]` sections. Each channel type requires different credentials.

### Telegram

```toml
[channels.telegram]
api_id = 12345
api_hash = "your_api_hash"
phone = "+1234567890"
```

### Discord

```toml
[channels.discord]
token = "your_bot_token"
```

### Slack

```toml
[channels.slack]
bot_token = "xoxb-..."
app_token = "xapp-..."
```

### Email

```toml
[channels.email]
smtp_host = "smtp.gmail.com"
imap_host = "imap.gmail.com"
username = "you@gmail.com"
password = "app-password"
```

See the [Channels](../channels/README.md) section for detailed setup instructions for each channel adapter.

## Full Example

```toml
model = "anthropic/claude-sonnet-4-20250514"
workspace = "~/.zeus/workspace"
sessions = "~/.zeus/sessions"
max_iterations = 20

[tui]
theme = "dark"
vim_mode = false

[auth]
use_oauth = false

[ollama]
url = "http://localhost:11434"

[mnemosyne]
db_path = "~/.zeus/mnemosyne.db"
enable_fts = true

[athena]
vault_path = "~/Obsidian/Zeus"

[aegis]
level = "standard"

[hermes]
default_channel = "console"

[nous]
enable_learning = true

[talos]
enable_applescript = true

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
