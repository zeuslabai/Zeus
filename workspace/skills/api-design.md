---
name: api-design
description: REST API design patterns for Zeus (zeus-api / Axum). Resource naming, status codes, error responses, pagination, and Zeus-specific conventions for the 95-route API gateway.
origin: ECC (adapted for Zeus/Axum)
---

# API Design (Zeus / Axum)

## Zeus API Conventions

Zeus uses Axum with 95 routes. All new endpoints must follow these conventions.

## URL Structure

```
# Existing pattern — follow it:
GET    /v1/agents               # list
POST   /v1/agents               # create
GET    /v1/agents/:id           # get one
PUT    /v1/agents/:id           # update
DELETE /v1/agents/:id           # delete
POST   /v1/agents/:id/send      # action on resource

# Sub-resources
GET    /v1/sessions/:id/messages
GET    /v1/sessions/:id/stats

# Actions (verb OK when not CRUD)
POST   /v1/agents/:id/spawn
POST   /v1/approvals/:id/approve
POST   /v1/approvals/:id/deny
```

## HTTP Status Codes

| Status | When |
|--------|------|
| 200 OK | Successful GET, PUT, DELETE |
| 201 Created | Successful POST that creates |
| 400 Bad Request | Invalid input, validation failure |
| 401 Unauthorized | Missing/invalid auth token |
| 403 Forbidden | Valid auth, insufficient permissions |
| 404 Not Found | Resource doesn't exist |
| 409 Conflict | Duplicate resource |
| 422 Unprocessable | Valid JSON but business rule violation |
| 500 Internal | Unexpected server error |

## Axum Handler Pattern

```rust
pub async fn list_agents(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    match state.list_agents().await {
        Ok(agents) => Json(agents).into_response(),
        Err(e) => {
            error!("list_agents error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR,
             Json(json!({"error": "internal error"}))).into_response()
        }
    }
}

pub async fn create_agent(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateAgentRequest>,
) -> impl IntoResponse {
    // Validate
    if req.name.is_empty() {
        return (StatusCode::BAD_REQUEST,
                Json(json!({"error": "name is required"}))).into_response();
    }
    // ... create
}
```

## Error Response Format

```json
{ "error": "human-readable message" }
```

Success response — return the resource directly or wrapped:
```json
{ "id": "...", "name": "...", "created_at": "..." }
```

For lists:
```json
[{ "id": "...", "name": "..." }, ...]
```

## Upsert Pattern (critical for named entities)

```rust
// When adding agents, MCP servers, etc. — always upsert by id/name
// See fix `17dbad91` (MCP dedup) and S20-1 (agent dedup)
if let Some(pos) = collection.iter().position(|e| e.id == new_entry.id) {
    collection[pos] = new_entry;  // update existing
} else {
    collection.push(new_entry);   // add new
}
```

## Route Registration

Add new routes in `zeus-api/src/handlers/mod.rs`:
```rust
.route("/v1/my_resource", get(list_my_resource).post(create_my_resource))
.route("/v1/my_resource/:id", get(get_my_resource).put(update_my_resource).delete(delete_my_resource))
```

## Auth

Zeus API uses `ZEUS_API_TOKEN` env var. Endpoints check the `Authorization: Bearer <token>` header. Don't bypass auth on new endpoints.

## Testing New Endpoints

```bash
BASE=http://localhost:3001
TOKEN=zeus-dev-token

curl -s $BASE/v1/my_resource \
  -H "Authorization: Bearer $TOKEN" | jq

curl -s -X POST $BASE/v1/my_resource \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"name":"test"}' | jq
```
