# API Reference

Zeus exposes **105+ REST API endpoints** through its HTTP gateway, organized into logical groups. The API server is launched with `zeus serve` or `zeus gateway`.

## Base URL

```
http://localhost:8080
```

Use `zeus serve -p <port>` to customize the port.

## Request / Response Format

All request and response bodies use **JSON** (`Content-Type: application/json`). Endpoints that accept a body expect a JSON object; endpoints that return data respond with a JSON object or array.

## Authentication

Authentication is **optional** and configurable. When enabled, include a Bearer token in the `Authorization` header:

```
Authorization: Bearer <token>
```

Tokens are obtained via the [OAuth / Auth endpoints](./configuration.md#authentication).

## CORS

The API server includes CORS middleware that allows cross-origin requests, making it suitable for browser-based frontends and the iOS mobile app.

## Error Responses

Errors return an appropriate HTTP status code with a JSON body:

```json
{
  "error": "Description of what went wrong"
}
```

The OpenAI-compatible endpoints use the OpenAI error format:

```json
{
  "error": {
    "message": "Description of what went wrong",
    "type": "invalid_request_error",
    "code": null
  }
}
```

## Endpoint Groups

| Group | Endpoints | Description |
|-------|-----------|-------------|
| [Health & Status](./health-status.md) | 5 | Server health, diagnostics, resource stats |
| [Chat & Streaming](./chat-streaming.md) | 4 | Send messages, OpenAI-compatible API, WebSocket |
| [Sessions](./sessions.md) | 12 | Session CRUD, replay, audit, branching |
| [Tools](./tools.md) | 2 | List and execute tools |
| [Memory & Knowledge](./memory.md) | 11 | Workspace files, memory search, context journals |
| [Agents](./agents.md) | 9 | Agent definitions, spawning, agent-as-API chat |
| [Channels](./channels-api.md) | 7 | Messaging channel CRUD and connectivity testing |
| [Webhooks](./webhooks.md) | 6 | Inbound receivers and outbound webhook registrations |
| [Configuration & Auth](./configuration.md) | 11 | Config management, provider testing, OAuth |
| [Analytics & Security](./analytics-security.md) | 15 | Costs, tokens, threats, permissions, approvals |
| [Skills, MCP & Extensions](./skills-extensions.md) | 14 | Skill plugins, MCP servers, extensions lifecycle |
| [Projects, Teams & Network](./projects-teams.md) | 16 | Projects, teams, delegations, routing, discovery |
