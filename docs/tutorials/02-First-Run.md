# First Run

After [[01-Installation|installing Zeus]], this tutorial walks you through the setup wizard and your first interaction.

## The Setup Wizard

On first launch, Zeus runs the interactive onboarding wizard:

```bash
zeus
```

The wizard walks you through:

1. **Provider selection** — Pick your LLM provider (Anthropic, OpenAI, Ollama, OpenRouter, etc.)
2. **API key** — Enter your API key (stored in `~/.zeus/.env`)
3. **Model choice** — Select a model (e.g., `anthropic/claude-sonnet-4-20250514`)
4. **Workspace** — Initialize the workspace directory at `~/.zeus/workspace/`

After setup, Zeus opens the TUI (Terminal User Interface).

You can re-run the wizard anytime:

```bash
zeus onboard
```

## Set Your API Key

API keys are read from environment variables. Add one to your shell profile:

```bash
# Pick one provider:
export ANTHROPIC_API_KEY="sk-ant-..."       # Anthropic Claude
export OPENAI_API_KEY="sk-..."              # OpenAI GPT
export OPENROUTER_API_KEY="sk-or-..."       # OpenRouter (multi-provider)
export GOOGLE_API_KEY="AIza..."             # Google Gemini
export GROQ_API_KEY="gsk_..."              # Groq

# Or use local models — no API key needed:
# Just have Ollama running at localhost:11434
```

> 💡 **Tip**: Store keys in `~/.zeus/.env` and they'll be loaded automatically. Never put API keys in `config.toml`.

## Your First Chat

### Single Message (Quick)

```bash
zeus chat "What can you do?"
```

Zeus sends the message to your configured LLM, prints the response, and exits.

### With Streaming

```bash
zeus chat -s "Explain how DNS works in 3 sentences"
```

The `-s` flag streams tokens as they arrive — you see the response build in real time.

### Interactive TUI

```bash
zeus
```

This opens the full terminal interface. Type your message at the bottom and press Enter. See [[07-TUI]] for the full guide.

## Run a Tool

Zeus has 212 tools. Try one directly:

```bash
# List files in the current directory
zeus tool list_dir '{"path":"."}'

# Run a shell command
zeus tool shell '{"command":"uname -a"}'

# Fetch a web page
zeus tool web_fetch '{"url":"https://httpbin.org/get"}'
```

## Check Your Setup

```bash
# Show current configuration
zeus config

# Run full diagnostics
zeus doctor

# Show config including (masked) API keys
zeus config --show-secrets
```

## Model String Format

All models use the `provider/model-name` pattern:

| Provider | Example | API Key Variable |
|----------|---------|-----------------|
| Anthropic | `anthropic/claude-sonnet-4-20250514` | `ANTHROPIC_API_KEY` |
| OpenAI | `openai/gpt-4o` | `OPENAI_API_KEY` |
| Ollama | `ollama/llama3.2` | *(none — local)* |
| OpenRouter | `openrouter/anthropic/claude-3.5-sonnet` | `OPENROUTER_API_KEY` |
| Google | `google/gemini-2.0-flash` | `GOOGLE_API_KEY` |
| Groq | `groq/llama-3.3-70b-versatile` | `GROQ_API_KEY` |
| Mistral | `mistral/mistral-large-latest` | `MISTRAL_API_KEY` |

If you omit the provider prefix, Zeus auto-detects from the model name (e.g., `claude-*` → Anthropic).

## What's Next

→ [[03-Configuration]] — Deep dive into config.toml and workspace files
→ [[04-Chat-and-Conversations]] — Sessions, context, conversation management
→ [[07-TUI]] — Master the terminal interface
