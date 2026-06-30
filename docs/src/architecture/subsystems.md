# Subsystems

Zeus is built around a minimal agent loop that can run standalone. Every advanced capability is an optional **subsystem** wired into specific hook points in that loop. Subsystems are initialized in `Agent::with_subsystems()` and stored as `Option` fields on the `Agent` struct -- if a subsystem is not configured, its hook is simply skipped.

This page describes each subsystem, where it hooks into the agent loop, and what it does at that hook point.

## Subsystem Overview

| Subsystem    | Crate            | Hook Point                          | Purpose                                      |
|--------------|------------------|-------------------------------------|----------------------------------------------|
| Nous         | zeus-nous        | Before LLM call (system prompt)     | Cognitive context injection                  |
| Mnemosyne    | zeus-mnemosyne   | After session.add() + before LLM    | Memory storage and recall                    |
| Athena       | zeus-athena      | After tool execution + after response | Action logging and documentation             |
| Aegis        | zeus-aegis       | Before tool execution               | Permission checks and sandboxing             |
| Hermes       | zeus-hermes      | After errors + after task completion | Notification routing                         |
| Prometheus   | zeus-prometheus  | Above the agent (wrapper)           | Orchestration, planning, cooking loop        |
| Channels     | zeus-channels    | Via the `message` tool              | Platform messaging adapters                  |

## Nous -- Cognitive Engine

**Hook point:** System prompt construction, before each LLM call.

Nous provides the agent with self-awareness about its own reasoning process. Before every call to the LLM, the agent queries Nous for cognitive context, which is appended to the system prompt. This context includes:

- **Intent analysis** -- What the user appears to be asking for, parsed into structured intent categories.
- **Reasoning state** -- The current chain of reasoning, including what the agent has tried and what it has learned so far in this session.
- **Meta-cognition** -- Self-reflective observations about the agent's own performance (e.g., "I have been repeating the same approach -- consider an alternative").
- **Learning outcomes** -- Patterns extracted from past interactions that are relevant to the current task.

The Nous subsystem also contains the **CriticEngine**, which evaluates tool executions after the fact (success/partial/failure with a quality score), and the **ConsolidationEngine**, which runs as a background task extracting patterns from accumulated experience and applying decay to stale knowledge.

**Integration path:** `Agent` holds an `Option<Arc<Nous>>`. During context construction, `nous.cognitive_context()` is called and the result is concatenated into the system prompt string.

## Mnemosyne -- Advanced Memory

**Hook point:** After `session.add()` (storage) and during context construction (recall).

Mnemosyne is the long-term memory system backed by SQLite. It operates at two moments in the agent loop:

**Storage:** After each message is added to the session, Mnemosyne stores it with metadata. If vector embeddings are enabled, the message is also embedded (via Ollama nomic-embed-text or OpenAI text-embedding-3-small) for semantic search.

**Recall:** During context construction (step 3 of the data flow), the `MemoryInjector` searches Mnemosyne for memories relevant to the current conversation. It uses hybrid search that merges:
- **BM25 FTS5 scores** -- SQLite full-text search for keyword matching.
- **Cosine similarity** -- Vector distance for semantic matching.

Results are weighted and the top matches are injected into the context window alongside workspace files and session history.

Mnemosyne supports three memory types with different retrieval characteristics:
- **Working memory** -- Short-lived, high-priority items (active goals, current task state).
- **Episodic memory** -- Past interactions and their outcomes.
- **Semantic memory** -- Extracted facts and patterns (populated by the ConsolidationEngine in Nous).

**Integration path:** `Agent` holds an `Option<Arc<tokio::sync::Mutex<Mnemosyne>>>`. The async mutex is necessary because Mnemosyne's SQLite connection is synchronous underneath, wrapped in `Arc<Mutex<rusqlite::Connection>>`.

## Athena -- Documentation Engine

**Hook point:** After tool executions, after messages, and after responses.

Athena observes everything the agent does and logs it as structured documentation. It serves as both an audit trail and a knowledge base generator.

At each hook point, Athena records:
- **Tool executions** -- Which tool was called, with what arguments, and what the result was.
- **Messages** -- Both user messages and agent responses, with timestamps.
- **Responses** -- Final agent output, tagged with the session and turn number.

This data is written as Obsidian-flavored markdown to the configured vault path (`~/.zeus/athena/` or a custom Obsidian vault). Cross-reference links connect related entries (e.g., a tool call links to the session that triggered it). On macOS, Athena can also push documentation to Apple Notes.

Athena additionally provides **session summarization** -- when a long conversation is being compacted, Athena can generate a summary of what was accomplished.

**Integration path:** `Agent` holds an `Option<Arc<Athena>>`. After each tool call result is collected and after the final response is assembled, the agent calls `athena.log_action()`.

## Aegis -- Security Sandbox

**Hook point:** Before tool execution.

Aegis is the gatekeeper. Every tool call must pass through Aegis before it executes. The security checks are:

- **Path restrictions** -- File operations (read_file, write_file, edit_file, list_dir) are checked against allowed directory patterns. By default, the agent can access the current working directory and the Zeus workspace. Attempts to read `/etc/shadow` or write to `/usr/bin/` are blocked.
- **Command filtering** -- The `shell` tool submits its command string to Aegis for validation. Commands are checked against an allowlist of safe patterns. Destructive commands (e.g., `rm -rf /`) are blocked unless explicitly approved.
- **URL allowlisting** -- The `web_fetch` tool submits its target URL. Aegis checks it against permitted domains. This prevents the LLM from exfiltrating data to arbitrary endpoints.
- **Seatbelt sandboxing** -- On macOS, Aegis can apply Seatbelt profiles that restrict the agent process at the OS level (file system access, network access, process execution).
- **Approval workflow** -- Operations that do not match the allowlist but are not outright blocked can be held for user approval. The TUI shows an approval prompt; the API exposes `/v1/approvals` endpoints for programmatic approval.

All decisions (allow, block, pending approval) are audit-logged with timestamps, the requesting tool, and the reason for the decision.

**Integration path:** `Agent` holds an `Option<Arc<Aegis>>`. Before dispatching any tool call, the agent calls `aegis.check_permission()`. If the check returns `Denied`, the tool result is set to an error message and no execution occurs.

## Hermes -- Notification Router

**Hook point:** After errors and after task completions.

Hermes is the simplest subsystem -- 302 lines. Its job is to send notifications when something noteworthy happens:

- **Errors** -- When a tool execution fails, when the LLM returns an error, or when a subsystem encounters a problem, Hermes sends an alert.
- **Task completions** -- When Prometheus finishes executing a planned task, or when a long-running autonomous operation completes, Hermes notifies the user.

Notifications are routed through the configured default channel (console output, Telegram, Discord, or any other channel adapter). This is particularly useful when Zeus is running autonomously (via the gateway daemon or heartbeat scheduler) and the user is not watching the TUI.

**Integration path:** `Agent` holds an `Option<Arc<Hermes>>`. Error handlers and task completion callbacks invoke `hermes.notify()`.

## Prometheus -- Orchestration

**Hook point:** Above the agent (wrapper, not inside the loop).

Prometheus does not hook into the agent loop -- it wraps it. The distinction matters: Nous, Mnemosyne, Athena, Aegis, and Hermes are called from within `Agent::run()`. Prometheus calls `Agent::run()` from the outside.

For simple messages (a direct question, a single-step request), Prometheus passes the input straight through to the agent. For complex tasks, it activates one of several orchestration strategies:

### Planner

Decomposes a complex request into a sequence of sub-tasks. Each sub-task is a focused prompt that the agent can handle in one or a few tool-call iterations. The planner tracks dependencies between sub-tasks and executes them in order.

### Cooking Loop

An iterative execution cycle for tasks that require sustained, multi-step work:

1. Send the current sub-task to `agent.run()`
2. Collect the result
3. Query Mnemosyne for relevant context to inject into the next iteration
4. Evaluate the result with the CriticEngine (success/partial/failure, quality score)
5. Update the FeedbackLoop with the outcome (adjusts strategy estimates)
6. If the task is not complete, formulate the next sub-task and loop back to step 1

The cooking loop continues until the goal is achieved, a maximum iteration count is reached, or the CriticEngine determines that progress has stalled.

### Heartbeat Scheduler

A periodic trigger (configurable interval) that checks `HEARTBEAT.md` for proactive tasks. If tasks are defined, Prometheus initiates an agent run to work on them. This enables Zeus to operate autonomously without user interaction.

### Cron Engine

SQLite-backed cron scheduler for recurring tasks. Supports standard cron expressions. Jobs are persisted so they survive restarts.

### Goal Stack

A persistent hierarchy of goals stored in SQLite. Goals are created when Prometheus decides to plan a complex task, updated as sub-tasks complete, and marked done when the CriticEngine confirms success. Active goals are surfaced in the system prompt via `active_goals_summary()` so the LLM is aware of the broader context.

### Feedback Loop

Learns from execution outcomes over time. Tracks which strategies work well for which types of tasks, estimates completion times, and adjusts the planner's approach based on historical success rates.

**Integration path:** The TUI, API, and CLI check if Prometheus is configured. If so, user input goes to `prometheus.process()` first. Prometheus decides whether to delegate directly to the agent or to orchestrate. The agent itself does not know whether it is being called directly or from within a Prometheus cooking loop.

## Channels -- Messaging Platform Adapters

**Hook point:** Via the `message` core tool.

Channels are not a traditional subsystem with a hook in the agent loop. Instead, they are exposed through the `message` tool. When the LLM calls `message` with a target channel (e.g., `telegram`, `discord`, `slack`), the agent routes the call to the `ChannelManager`, which dispatches it to the appropriate adapter.

Eight adapters are available, each implementing the `ChannelAdapter` trait:

| Adapter   | Outbound              | Inbound                    |
|-----------|-----------------------|----------------------------|
| Telegram  | Send via MTProto      | Poll/webhook via grammers  |
| Discord   | Send via HTTP API     | Gateway events via serenity|
| Slack     | Send via Web API      | Socket Mode events         |
| Email     | Send via SMTP (lettre)| Receive via IMAP IDLE      |
| iMessage  | Send via AppleScript  | Read via AppleScript       |
| WhatsApp  | Send via Cloud API    | Webhook callbacks          |
| Signal    | Send via signal-cli   | JSON-RPC events            |
| Matrix    | Send via matrix-sdk   | Sync loop events           |

The `ChannelManager` also handles:
- **Message chunking** -- Splitting long messages to fit platform limits.
- **Streaming delivery** -- Forwarding token-by-token streaming to platforms that support editing messages in place.
- **Channel policies** -- Rate limiting, retry logic, and fallback routing.
- **Media pipeline** -- Handling attachments, images, and file uploads across platforms.
- **Pairing manager** -- Linking inbound messages from a platform user to a Zeus session.

Inbound messages from channels are collected via an mpsc receiver and can trigger new agent runs, enabling Zeus to operate as a chatbot on any supported platform.

**Integration path:** `Agent` holds an `Option<Arc<ChannelManager>>`. The `message` tool handler calls `channel_manager.send()`. The gateway daemon starts inbound listeners for configured channels and routes received messages into new agent runs.
