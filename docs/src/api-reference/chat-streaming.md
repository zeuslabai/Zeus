# Chat & Streaming

Send messages to the agent and receive responses via REST or WebSocket. Includes an OpenAI-compatible ChatCompletion endpoint.

## Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/v1/chat` | Send message (non-streaming) |
| `POST` | `/v1/chat/completions` | OpenAI ChatCompletion (streaming & non-streaming) |
| `GET` | `/v1/models` | List available models (OpenAI format) |
| `GET` | `/v1/ws` | WebSocket streaming |

---

## POST `/v1/chat`

Send a message to the agent and receive the full response when complete.

**Request Body**

```json
{
  "message": "What files are in the current directory?",
  "session_id": "optional-session-uuid"
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `message` | string | Yes | The user message |
| `session_id` | string | No | Existing session ID. A new session is created if omitted. |

**Response** `200 OK`

```json
{
  "response": "Here are the files in the current directory...",
  "session_id": "a1b2c3d4-...",
  "tool_calls": [
    {
      "tool": "list_dir",
      "arguments": { "path": "." },
      "result": "file1.txt\nfile2.rs\n..."
    }
  ]
}
```

---

## POST `/v1/chat/completions`

OpenAI-compatible ChatCompletion endpoint. Supports both streaming and non-streaming modes.

**Request Body**

```json
{
  "model": "anthropic/claude-sonnet-4-20250514",
  "messages": [
    { "role": "system", "content": "You are a helpful assistant." },
    { "role": "user", "content": "Hello!" }
  ],
  "stream": false,
  "temperature": 0.7,
  "max_tokens": 1024
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `model` | string | Yes | Model identifier (routed through Zeus LLM pipeline) |
| `messages` | array | Yes | Array of `{role, content}` objects |
| `stream` | boolean | No | Enable SSE streaming (default: `false`) |
| `temperature` | number | No | Sampling temperature |
| `max_tokens` | number | No | Maximum tokens in response |

### Non-Streaming Response `200 OK`

```json
{
  "id": "chatcmpl-abc123",
  "object": "chat.completion",
  "created": 1707000000,
  "model": "anthropic/claude-sonnet-4-20250514",
  "choices": [
    {
      "index": 0,
      "message": { "role": "assistant", "content": "Hello! How can I help?" },
      "finish_reason": "stop"
    }
  ],
  "usage": {
    "prompt_tokens": 12,
    "completion_tokens": 8,
    "total_tokens": 20
  }
}
```

### Streaming Response

When `stream: true`, the response is sent as Server-Sent Events (SSE):

```
data: {"id":"chatcmpl-abc123","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"role":"assistant","content":"Hello"},"finish_reason":null}]}

data: {"id":"chatcmpl-abc123","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"content":"!"},"finish_reason":null}]}

data: {"id":"chatcmpl-abc123","object":"chat.completion.chunk","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}

data: [DONE]
```

### Error Response

```json
{
  "error": {
    "message": "Invalid model format",
    "type": "invalid_request_error",
    "code": null
  }
}
```

---

## GET `/v1/models`

List available models in OpenAI-compatible format.

**Response** `200 OK`

```json
{
  "object": "list",
  "data": [
    {
      "id": "anthropic/claude-sonnet-4-20250514",
      "object": "model",
      "created": 1707000000,
      "owned_by": "anthropic"
    }
  ]
}
```

---

## GET `/v1/ws`

WebSocket endpoint for real-time streaming interaction with the agent.

### Connection

```
ws://localhost:8080/v1/ws
```

### Client Messages

Send a JSON message to start a chat interaction:

```json
{
  "type": "chat",
  "message": "What is the weather today?",
  "session_id": "optional-session-uuid"
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `type` | string | Yes | Message type. Use `"chat"`. |
| `message` | string | Yes | The user message |
| `session_id` | string | No | Existing session ID |

### Server Messages

The server sends a sequence of JSON messages as the agent processes the request:

**Text chunk** -- streamed token from the LLM response:

```json
{
  "type": "text_chunk",
  "content": "Here are "
}
```

**Tool call** -- the agent is invoking a tool:

```json
{
  "type": "tool_call",
  "tool": "shell",
  "arguments": { "command": "ls -la" }
}
```

**Tool result** -- output from the tool execution:

```json
{
  "type": "tool_result",
  "tool": "shell",
  "result": "total 42\ndrwxr-xr-x ..."
}
```

**Response complete** -- the final assembled response:

```json
{
  "type": "response_complete",
  "content": "Here are the files in the directory..."
}
```

**Finished** -- the interaction is complete:

```json
{
  "type": "finished",
  "session_id": "a1b2c3d4-..."
}
```

**Error** -- an error occurred:

```json
{
  "type": "error",
  "message": "LLM request timed out after 300s"
}
```

### Message Sequence

A typical interaction follows this sequence:

```
Client  -> { type: "chat", message: "..." }
Server  <- { type: "text_chunk", content: "..." }   (repeated)
Server  <- { type: "tool_call", ... }                (if tools used)
Server  <- { type: "tool_result", ... }              (if tools used)
Server  <- { type: "text_chunk", content: "..." }    (continued)
Server  <- { type: "response_complete", content: "..." }
Server  <- { type: "finished", session_id: "..." }
```
