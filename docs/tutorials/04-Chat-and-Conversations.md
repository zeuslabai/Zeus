# Chat and Conversations

Zeus supports multiple chat modes: single messages, streaming, interactive TUI, and API-based conversations. All chats are persisted as sessions.

## Single Message Mode

Send a message, get a response, exit:

```bash
zeus chat "What is the capital of France?"
```

With streaming (tokens appear as they're generated):

```bash
zeus chat -s "Explain quantum computing in simple terms"
```

## Sessions

Every conversation creates a session. Sessions are stored as JSONL files in `~/.zeus/sessions/`.

### List Sessions

```bash
zeus session list
```

### View a Session

```bash
zeus session show <session-id>
```

### Export to Markdown

```bash
zeus session export <session-id> output.md
```

### Session Compaction

Long conversations are automatically compacted when they approach the context window limit. Zeus summarizes older messages to keep the most relevant context available.

Configure in `config.toml`:

```toml
[session_compaction]
max_context_tokens = 180000
compaction_threshold = 0.8    # Compact at 80% of max context
```

## Tool Calls in Chat

Zeus can use tools during chat. When it decides to use a tool, you'll see the tool call and result inline:

```bash
$ zeus chat -s "What files are in my home directory?"
```

Output:
```
🔧 list_dir(path: "~")
   → Documents/  Downloads/  Desktop/  ...

Your home directory contains the following folders: Documents, Downloads, Desktop, ...
```

Zeus decides autonomously when to use tools based on your request. It can chain multiple tool calls in sequence.

## Multi-Turn Conversations

In the TUI (see [[07-TUI]]), conversations are multi-turn by default. Each message adds to the session context.

Via the API (see [[08-API-Server]]), use `session_id` to continue a conversation:

```bash
# Start a conversation
curl -X POST http://localhost:3001/v1/chat \
  -H "Content-Type: application/json" \
  -d '{"message": "My name is Alice"}'

# Continue it (use the session_id from the first response)
curl -X POST http://localhost:3001/v1/chat \
  -H "Content-Type: application/json" \
  -d '{"message": "What is my name?", "session_id": "ses_abc123"}'
```

## Context and Memory

Zeus has two kinds of memory:

1. **Session context** — The current conversation. Automatically managed with compaction.
2. **Long-term memory** — Facts stored in the workspace. Persists across sessions.

```bash
# Store a fact
zeus memory remember "I prefer Python over JavaScript"

# The next conversation will know this
zeus chat "What's my preferred language?"
```

See [[06-Memory]] for the full memory system guide.

## Subagents (Spawn)

Zeus can spawn background subagents to handle parallel work:

```bash
zeus chat "Research the top 5 programming languages and summarize each one"
```

If the task is parallelizable, Zeus may use the `spawn` tool to create subagents that work concurrently and report back.

## Tips

- **Be specific**: "List the Rust files in src/" works better than "show me some files"
- **Chain requests**: "Read README.md and summarize the key features" — Zeus will read the file then summarize
- **Iterate**: Follow up with "make it shorter" or "add more detail" in the TUI
- **Use streaming**: `-s` flag gives faster feedback on long responses

## What's Next

→ [[05-Tools]] — Explore all 212 tools
→ [[07-TUI]] — Master the interactive terminal UI
