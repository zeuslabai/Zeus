# Data Flow

Every interaction with Zeus -- whether a typed message in the TUI, an HTTP request to the API, a command from the CLI, or a tap in the iOS app -- follows the same 10-step pipeline through the agent loop.

## The Pipeline

```
 User Input                                              Response
     │                                                      ▲
     ▼                                                      │
 ┌──────────────────────────────────────────────────────────────┐
 │ 1. Frontend (TUI / API / CLI / Desktop / iOS)            10. │
 └──────────────────────────┬───────────────────────────────────┘
                            │
                            ▼
 ┌──────────────────────────────────────────────────────────────┐
 │ 2. Route to Agent                                            │
 └──────────────────────────┬───────────────────────────────────┘
                            │
                            ▼
 ┌──────────────────────────────────────────────────────────────┐
 │ 3. Build Context (Workspace + Nous + Mnemosyne)              │
 └──────────────────────────┬───────────────────────────────────┘
                            │
                            ▼
 ┌──────────────────────────────────────────────────────────────┐
 │ 4. Aegis Permission Check                                    │
 └──────────────────────────┬───────────────────────────────────┘
                            │
                            ▼
 ┌──────────────────────────────────────────────────────────────┐
 │ 5. LLM Call (streaming, 5-min timeout)          ◄────┐       │
 └──────────────────────────┬───────────────────────┐   │       │
                            │                       │   │
                            ▼                       │   │
 ┌──────────────────────────────────────────┐       │   │
 │ 6. Parse Tool Calls (if any)             │       │   │
 └──────────────────────────┬───────────────┘       │   │
                            │                       │   │
                            ▼                       │   │
 ┌──────────────────────────────────────────┐       │   │
 │ 7. Execute Tools                         ├───────┘   │
 │    (core / Talos / Browser / Channels)   │   loop    │
 └──────────────────────────┬───────────────┘           │
                            │                           │
                            ▼                           │
 ┌──────────────────────────────────────────────────────┘
 │ 8. Persist (Athena log + Session JSONL + Mnemosyne)
 └──────────────────────────┬───────────────────────────┘
                            │
                            ▼
 ┌──────────────────────────────────────────────────────────────┐
 │ 9. Hermes Notifications                                      │
 └──────────────────────────────────────────────────────────────┘
```

## Step-by-Step

### 1. User input arrives at a frontend

Zeus has five entry points, all converging on the same agent:

| Frontend        | Transport                      | Crate         |
|-----------------|--------------------------------|---------------|
| TUI             | Direct function call           | zeus-tui      |
| REST API        | HTTP POST / WebSocket          | zeus-api      |
| CLI             | `zeus chat "message"`          | zeus-agent    |
| macOS Desktop   | UniFFI (Rust called from Swift)| zeus-ffi      |
| iOS             | REST + WebSocket to gateway    | zeus-api      |

### 2. Input routed to Agent

Regardless of the frontend, the user's message is wrapped in a `Message` struct (defined in zeus-core) and passed to `Agent::run()`. If Prometheus is active and determines the task is complex, it intercepts the message and runs its planner or cooking loop instead, calling back into the agent for each sub-step.

### 3. Agent builds context

Before calling the LLM, the agent assembles a context window:

- **Workspace files** -- `AGENTS.md` (system prompt), `SOUL.md` (personality), `USER.md` (user facts) are read from `~/.zeus/workspace/` by zeus-memory.
- **Nous cognitive context** -- The cognitive engine produces intent analysis, reasoning state, and meta-cognitive observations. This text is appended to the system prompt.
- **Mnemosyne recall** -- The advanced memory system searches for relevant past interactions using hybrid FTS5 + vector similarity. Matching memories are injected into context via `MemoryInjector`.
- **Session history** -- The `ContextManager` in zeus-session selects recent conversation turns that fit within the token budget.

### 4. Aegis validates permissions

Before any tool can execute (and before the LLM call itself, for prompt-injection checks), the Aegis security subsystem runs its validation pipeline:

- **Path restrictions** -- File operations are checked against allowed directories.
- **Command filtering** -- Shell commands are validated against the allowlist.
- **URL allowlisting** -- `web_fetch` targets are checked against permitted domains.
- **Approval workflow** -- Operations flagged as sensitive are held pending user approval (via the `/v1/approvals` API or TUI prompt).

If a check fails, the tool call is blocked and the LLM receives an error result explaining why.

### 5. Agent calls LLM with streaming

The agent sends the assembled context (system prompt + conversation history + available tool schemas) to the configured LLM provider via zeus-llm. The call streams tokens back with a 5-minute timeout to prevent hanging on unresponsive providers.

### 6. LLM response may include tool calls

The streamed response is parsed. If the LLM included tool call requests (in the provider's native format -- Anthropic tool_use blocks, OpenAI function calls, etc.), they are extracted as `ToolCall` structs. If no tool calls are present, the text response is the final answer.

### 7. Agent executes tools

Each tool call is dispatched to the appropriate handler:

| Tool source       | Examples                                         |
|--------------------|--------------------------------------------------|
| Core tools (8)     | read_file, write_file, edit_file, list_dir, shell, web_fetch, spawn, message |
| Talos (193)        | system_info, clipboard_read, safari_open_url, git_status, calendar_list_events |
| Browser (11)       | navigate, click, type, get_text, screenshot, execute_js |
| Channel adapters   | Routed through the `message` tool to Telegram, Discord, Slack, Email, iMessage, WhatsApp, Signal, or Matrix |
| MCP tools          | Tools from connected MCP servers                  |

Tool results are collected and fed back to the LLM. Steps 5 through 7 repeat in a loop (up to `max_iterations`, default 20) until the LLM produces a final text response with no further tool calls.

### 8. Persist state

After each iteration:

- **Athena** logs the tool execution, message content, and response as structured actions (written to the Obsidian vault and/or Apple Notes if configured).
- **Session** appends the turn to the JSONL session file via zeus-session.
- **Mnemosyne** stores the message for future recall, generating embeddings if vector search is enabled.

### 9. Hermes sends notifications

If the interaction resulted in an error or completed a tracked task, Hermes routes a notification through the configured channel (console, Telegram, Discord, etc.). This is especially useful for long-running autonomous tasks where the user is not actively watching the TUI.

### 10. Response streams back to user

The final text response (or the streaming token sequence) is delivered back through the same frontend that initiated the request -- rendered in the TUI chat panel, returned as an HTTP response body, printed to stdout for CLI mode, or pushed over WebSocket to the mobile app.

## The Tool Execution Loop

Steps 5-7 form the core iteration loop inside `Agent::run()`. A single user message may trigger multiple rounds of LLM calls and tool executions. For example, "find all TODO comments and create a summary file" might proceed as:

1. LLM calls `shell` to run `grep -r TODO .`
2. Agent executes the command, returns output
3. LLM calls `write_file` to create the summary
4. Agent writes the file, returns confirmation
5. LLM produces the final text response (no more tool calls)

The loop terminates when the LLM responds without tool calls or when `max_iterations` is reached.

## Prometheus Orchestration

For complex, multi-step tasks, the Prometheus crate wraps this entire pipeline. Its **cooking loop** works at a higher level:

1. Analyze the user's intent and decide whether to plan
2. Decompose the task into sub-steps
3. For each step, call `agent.run()` with a focused sub-prompt
4. Inject context from Mnemosyne between steps
5. Evaluate results with the CriticEngine
6. Adjust strategy based on FeedbackLoop outcomes
7. Continue until all steps are complete or the goal is achieved

This means a single user request can trigger dozens of agent loop iterations, each following the full 10-step pipeline.
