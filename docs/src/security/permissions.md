# Permissions & Approvals

Zeus includes an approval system that requires human confirmation before executing potentially dangerous operations. This is managed by the `ApprovalManager` in `zeus-aegis`.

## How It Works

When the agent loop is about to execute a tool, it checks with the `ApprovalManager`:

1. **Pattern matching** -- The tool name and arguments are checked against dangerous patterns (e.g., `rm -rf`, `sudo`, `DROP TABLE`).
2. **Tool-level rules** -- Certain tools can be configured to always require approval, regardless of their arguments.
3. **Approval request** -- If a match is found, an `ApprovalRequest` is created and the execution is paused.
4. **Waiting** -- The agent waits for a human to approve or deny the request, up to a configurable timeout.
5. **Resolution** -- The operation proceeds if approved, is cancelled if denied, or times out and is cancelled.

## Approval Request

Each pending approval contains:

| Field | Description |
|-------|-------------|
| `id` | Unique identifier for the request |
| `tool_name` | Name of the tool being executed (e.g., `shell`) |
| `command` | The shell command (if applicable) |
| `args` | Full tool arguments as JSON |
| `timestamp` | When the request was created |

## Approval Outcomes

| Outcome | Description |
|---------|-------------|
| `Approved` | The operation proceeds normally. |
| `Denied(reason)` | The operation is cancelled with a reason string. |
| `Timeout` | No response within the timeout period; the operation is cancelled. |

## Timeout

The default timeout is 300 seconds (5 minutes). If no human responds within this window, the approval request is automatically denied. The timeout is configurable via the `with_timeout()` builder method.

## Dangerous Patterns

The `ApprovalManager` checks shell commands against a list of string patterns. If any pattern is found within the command string, approval is required. Default patterns include:

- `rm ` (with variations like `-rf`, `-r`)
- `sudo`
- `DROP TABLE` / `DROP DATABASE`
- `chmod`
- `mkfs`
- `dd if=`
- `shutdown` / `reboot`

You can add custom patterns when constructing the `ApprovalManager`.

## Tools Requiring Approval

In addition to pattern-based checks, specific tools can be configured to always require approval. This is set by providing a list of tool names to the `ApprovalManager`:

```rust
let manager = ApprovalManager::new(
    vec!["rm".to_string(), "sudo".to_string()],  // patterns
    vec!["shell".to_string()],                     // tools always requiring approval
);
```

## API Endpoints

Approvals can be managed through the REST API:

### List Pending Approvals

```
GET /v1/approvals
```

Returns all pending approval requests with their IDs, tool names, commands, and timestamps.

### Approve a Request

```
POST /v1/approvals/:id/approve
```

Approves the pending request, allowing the tool execution to proceed.

### Deny a Request

```
POST /v1/approvals/:id/deny
```

Denies the pending request. An optional JSON body can include a reason:

```json
{
    "reason": "Command is too destructive for this environment"
}
```

## TUI Integration

When running the TUI, pending approvals appear as notifications. You can approve or deny them directly from the terminal interface without switching to the API.

## Channel Integration

When running in gateway mode with messaging channels enabled, approval requests can be sent to a configured channel (e.g., Telegram, Slack) and responded to via channel messages. This allows remote approval of operations when you are not at the terminal.

## WebSocket Broadcast

Approval events are broadcast over the WebSocket connection (`GET /v1/ws`), allowing real-time UIs (such as the iOS or Desktop app) to display approval requests and send responses.
