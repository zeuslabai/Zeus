# Audit Logging

Zeus maintains a tamper-evident audit trail that records every security-relevant action. Audit logging is handled by two components: the `AuditLog` in `zeus-aegis` for low-level security events, and `zeus-athena` for higher-level action logging (tool executions, messages, responses).

## Tamper-Evident Hash Chain

The `AuditLog` writes entries to a JSONL file (default: `~/.zeus/audit.log`). Each entry contains a SHA-256 hash of its contents combined with the hash of the previous entry, forming a hash chain. This makes it possible to detect if any entry has been modified, deleted, or reordered after the fact.

An audit entry contains:

| Field | Description |
|-------|-------------|
| `sequence` | Monotonically increasing sequence number |
| `timestamp` | UTC timestamp of the event |
| `event` | The event data (see event types below) |
| `prev_hash` | SHA-256 hash of the previous entry |
| `hash` | SHA-256 hash of this entry (computed from sequence + timestamp + event + prev_hash) |

## Event Types

The audit log records these event types:

| Event | Fields | Description |
|-------|--------|-------------|
| `secret_access` | key, operation | A secret was read from or written to the keychain |
| `tool_execution` | tool, args, success | A tool was executed (includes full arguments and outcome) |
| `network_request` | host, method, path | An outgoing HTTP request was made |
| `file_access` | path, operation | A file was read or written |
| `authentication` | channel, user, success | An authentication attempt on a messaging channel |
| `permission_check` | operation, allowed | A permission check was performed |
| `system` | event, details | A system-level event (startup, shutdown, config change, etc.) |

## Integrity Verification

The `AuditLog::verify()` method reads the entire audit log and validates the hash chain from the first entry to the last. If any entry has been tampered with, the verification fails and reports which sequence number is invalid.

```rust
let is_valid = aegis.verify_audit_log().await?;
```

This can also be triggered via the `zeus doctor` CLI command, which includes audit log integrity as one of its diagnostic checks.

## zeus-athena Action Logging

While `zeus-aegis` handles security-specific audit events, `zeus-athena` (the documentation engine) logs higher-level actions:

- **Tool executions** -- Every tool call, its arguments, and its result.
- **Messages** -- User and assistant messages in each session.
- **Responses** -- Full LLM responses including any thinking or reasoning.

Athena stores these in its own format (Obsidian markdown or Apple Notes), providing a human-readable record of everything the agent did and why.

## Session Audit API

The REST API provides two endpoints for inspecting a session's audit trail:

### Audit Trail

```
GET /v1/sessions/:id/audit
```

Returns the audit trail for a specific session: all tool calls, memory writes, and security events that occurred during that session. This combines data from both the `AuditLog` and Athena's action log.

### Tool Call Chain

```
GET /v1/sessions/:id/tools
```

Returns the chain of tool calls for a session in execution order. This is useful for understanding the agent's decision-making process -- which tools were called, in what order, with what arguments, and what results they produced. The output is structured as an execution graph.

## Session Replay

For a more complete view, the session replay endpoint provides the full chronological record:

```
GET /v1/sessions/:id/replay
```

Each entry includes:

| Field | Description |
|-------|-------------|
| `index` | Position in the session (0-based) |
| `timestamp` | When the entry was recorded |
| `role` | user, assistant, or tool |
| `content` | Message or result content |
| `tool_calls` | Tool calls made (if assistant message) |
| `tool_name` | Tool that produced this result (if tool message) |
| `tool_results` | Results from tool execution |
| `thinking` | Extracted thinking/reasoning (from `<thinking>` tags) |
| `token_count` | Estimated token count (chars/4 heuristic) |

A single turn can be retrieved by index:

```
GET /v1/sessions/:id/replay/:turn
```

## Configuration

Audit logging is configured in the `[aegis]` section:

```toml
[aegis]
audit_path = "~/.zeus/audit.log"
```

Athena action logging is configured in the `[athena]` section:

```toml
[athena]
vault_path = "~/Obsidian/Zeus"
```

Both are enabled by default when their respective subsystems are initialized.
