# API Server

Zeus exposes a REST API for programmatic access. Start the server and integrate Zeus into any application.

## Start the Server

```bash
zeus serve              # Default port 3001
zeus serve -p 8080      # Custom port
```

## Health Check

```bash
curl http://localhost:3001/health
# {"status":"ok"}

curl http://localhost:3001/v1/status | jq
# Model, session info, subsystem health
```

## Chat

### Non-Streaming

```bash
curl -X POST http://localhost:3001/v1/chat \
  -H "Content-Type: application/json" \
  -d '{"message": "Hello, what can you do?"}'
```

### WebSocket Streaming

Connect to `ws://localhost:3001/v1/ws` for real-time streaming:

```json
// Send
{"type": "chat", "message": "Explain TCP/IP", "session_id": "optional-id"}

// Receive (in order)
{"type": "started"}
{"type": "text_chunk", "chunk": "TCP/IP is..."}
{"type": "text_chunk", "chunk": " a protocol..."}
{"type": "tool_call", "name": "web_fetch", "args": {"url": "..."}}
{"type": "tool_result", "name": "web_fetch", "success": true, "output": "..."}
{"type": "response_complete", "content": "full response text"}
{"type": "finished", "iterations": 2}
```

Test with `wscat`:

```bash
npm install -g wscat
wscat -c ws://localhost:3001/v1/ws
> {"type":"chat","message":"ping"}
```

## Sessions

```bash
# List all sessions
curl http://localhost:3001/v1/sessions | jq

# Create a new session
curl -X POST http://localhost:3001/v1/sessions | jq

# Get a specific session
curl http://localhost:3001/v1/sessions/<id> | jq
```

## Tools

```bash
# List all tools
curl http://localhost:3001/v1/tools | jq '.[].name'

# Execute a tool (NOTE: requires {"arguments":{...}} wrapper)
curl -X POST http://localhost:3001/v1/tools/list_dir \
  -H "Content-Type: application/json" \
  -d '{"arguments":{"path":"."}}'

curl -X POST http://localhost:3001/v1/tools/shell \
  -H "Content-Type: application/json" \
  -d '{"arguments":{"command":"whoami"}}'
```

## Memory

```bash
# Get workspace context
curl http://localhost:3001/v1/memory | jq

# Store a fact
curl -X POST http://localhost:3001/v1/memory/remember \
  -H "Content-Type: application/json" \
  -d '{"fact":"API test fact"}'

# Add a daily note
curl -X POST http://localhost:3001/v1/memory/note \
  -H "Content-Type: application/json" \
  -d '{"text":"Note from the API"}'

# Search memory (requires Mnemosyne)
curl -X POST http://localhost:3001/v1/memory/search \
  -H "Content-Type: application/json" \
  -d '{"query":"test","mode":"hybrid"}'
```

## Agents

```bash
# List agents
curl http://localhost:3001/v1/agents | jq

# Create an agent
curl -X POST http://localhost:3001/v1/agents \
  -H "Content-Type: application/json" \
  -d '{"name":"CodeReviewer","model":"anthropic/claude-haiku-4-5-20251001"}'

# Create from a persona template
curl http://localhost:3001/v1/personas | jq                         # List available personas
curl -X POST http://localhost:3001/v1/agents/from-persona/code-reviewer \
  -H "Content-Type: application/json" -d '{}'
```

## Skills

```bash
# List all skills (enriched with metadata)
curl http://localhost:3001/v1/skills | jq

# Get skill details
curl http://localhost:3001/v1/skills/<id> | jq

# Search skills
curl "http://localhost:3001/v1/skills/search?q=docker" | jq

# List categories
curl http://localhost:3001/v1/skills/categories | jq
```

## Security

```bash
# Threat log
curl http://localhost:3001/v1/security/threats | jq

# Permissions matrix
curl http://localhost:3001/v1/security/permissions | jq

# Pending approvals
curl http://localhost:3001/v1/approvals | jq
```

## Analytics

```bash
# Cost breakdown
curl http://localhost:3001/v1/analytics/costs | jq

# Token usage
curl http://localhost:3001/v1/analytics/tokens | jq
```

## OpenAI-Compatible Endpoint

Zeus provides a drop-in OpenAI-compatible API. Use any OpenAI client library:

```bash
curl http://localhost:3001/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "claude-haiku-4-5-20251001",
    "messages": [{"role": "user", "content": "ping"}],
    "stream": false
  }'
```

**Python example:**

```python
import openai

client = openai.OpenAI(
    base_url="http://localhost:3001/v1",
    api_key="any"  # Zeus doesn't require API key for local access
)

response = client.chat.completions.create(
    model="claude-haiku-4-5-20251001",
    messages=[{"role": "user", "content": "Hello!"}]
)
print(response.choices[0].message.content)
```

## Authentication

Optional bearer token. Configure in `config.toml`:

```toml
[gateway]
auth_token = "your-secret-token"
```

Then pass it in requests:

```bash
curl -H "Authorization: Bearer your-secret-token" \
  http://localhost:3001/v1/status
```

Health endpoints (`/`, `/health`) bypass auth.

## What's Next

→ [[09-Channels]] — Connect messaging platforms
→ [[12-Gateway]] — Run the full production daemon
→ [[13-Pantheon]] — Multi-agent orchestration API
