# Environment Variables

Zeus uses environment variables for API keys and credentials. These are never stored in `config.toml` -- they are read from the shell environment at runtime.

## LLM Provider Keys

| Variable | Required For |
|----------|-------------|
| `ANTHROPIC_API_KEY` | Anthropic Claude models |
| `OPENAI_API_KEY` | OpenAI GPT models |
| `OPENROUTER_API_KEY` | OpenRouter models |
| `GOOGLE_API_KEY` | Google Gemini models |
| `GROQ_API_KEY` | Groq models |
| `MISTRAL_API_KEY` | Mistral AI models |
| `TOGETHER_API_KEY` | Together AI models |
| `FIREWORKS_API_KEY` | Fireworks AI models |

You only need the key for the provider you are using. For example, if your model is `anthropic/claude-sonnet-4-20250514`, you only need `ANTHROPIC_API_KEY`.

## Azure OpenAI

| Variable | Required For |
|----------|-------------|
| `AZURE_OPENAI_API_KEY` | Azure OpenAI authentication |
| `AZURE_OPENAI_ENDPOINT` | Azure OpenAI resource URL (e.g., `https://your-resource.openai.azure.com`) |
| `AZURE_OPENAI_DEPLOYMENT` | Azure OpenAI deployment name |

All three variables are required when using the `azure/` provider prefix.

## AWS Bedrock

| Variable | Required For |
|----------|-------------|
| `AWS_ACCESS_KEY_ID` | AWS authentication |
| `AWS_SECRET_ACCESS_KEY` | AWS authentication |
| `AWS_REGION` | AWS Bedrock region (default: `us-east-1`) |

Both the access key and secret key are required when using the `bedrock/` provider prefix. The region defaults to `us-east-1` if not set.

## Ollama

| Variable | Required For |
|----------|-------------|
| `OLLAMA_HOST` | Custom Ollama server URL (optional) |

Ollama runs locally and does not require an API key. The `OLLAMA_HOST` variable overrides the default URL (`http://localhost:11434`). This can also be set via `[ollama] url` in `config.toml`.

## Matrix

| Variable | Required For |
|----------|-------------|
| `MATRIX_HOMESERVER` | Matrix homeserver URL (e.g., `https://matrix.org`) |
| `MATRIX_USER` | Matrix username |
| `MATRIX_PASSWORD` | Matrix password |

Used by the Matrix channel adapter. Supports password login with automatic token restore for subsequent connections.

## Twilio (Voice Calls)

| Variable | Required For |
|----------|-------------|
| `TWILIO_ACCOUNT_SID` | Twilio account identifier |
| `TWILIO_AUTH_TOKEN` | Twilio API authentication |
| `TWILIO_PHONE_NUMBER` | Twilio caller ID (outbound phone number) |

Used by the voice call subsystem (`zeus-voice`) for outbound calls and incoming call webhooks.

## Setting Environment Variables

### Shell Profile

Add exports to your `~/.bashrc`, `~/.zshrc`, or equivalent:

```bash
export ANTHROPIC_API_KEY="sk-ant-..."
```

### Per-Session

Set variables before running Zeus:

```bash
ANTHROPIC_API_KEY="sk-ant-..." zeus chat "Hello"
```

### Daemon

When running Zeus as a launchd daemon, environment variables must be set in the launchd plist or via `launchctl setenv`:

```bash
launchctl setenv ANTHROPIC_API_KEY "sk-ant-..."
```

## Checking Configuration

Run `zeus doctor` to verify that required environment variables are set for your configured provider:

```bash
zeus doctor
```

The diagnostics output will flag any missing API keys for the provider specified in your `model` config.
