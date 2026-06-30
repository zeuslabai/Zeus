# Common Patterns (Zeus / Rust)

## New Feature Approach

1. Search Zeus codebase for existing patterns (grep crates/ first)
2. Check for existing traits/types in zeus-core before duplicating
3. Use existing crates (reqwest, rusqlite, tokio, serde) before adding new deps
4. Follow existing module structure in the relevant crate

## Design Patterns

### Trait-Based Tool Dispatch

Zeus tools implement a common trait for uniform dispatch:

```rust
#[async_trait]
pub trait TalosTool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn schema(&self) -> serde_json::Value;
    async fn execute(&self, args: serde_json::Value) -> Result<String>;
}
```

New tools should implement this pattern, not ad-hoc dispatch.

### Upsert Pattern (avoid bloat)

When storing named entities (agents, MCP servers, skills), always upsert by ID/name:

```rust
// Find existing entry by id, replace in-place; otherwise push
if let Some(pos) = collection.iter().position(|e| e.id == new_entry.id) {
    collection[pos] = new_entry;
} else {
    collection.push(new_entry);
}
```

This pattern fixed MCP server dedup (`17dbad91`) and is needed for agent registry (S20-1).

### API Response Format

All Zeus API endpoints return consistent JSON:
- Success: `{ "data": ... }`
- Error: `{ "error": "message" }` with appropriate HTTP status

### Config-First Design

Never hardcode service URLs or paths. Use:
- `config.workspace` for workspace paths
- `std::env::var("ZEUS_*")` for service URLs (see hardcoded URL sprint)
- `DeploymentConfig` for fleet configuration

## Skeleton / Starting Point

When adding a new channel adapter:
- Implement `ChannelAdapter` trait from zeus-channels
- Register in `ChannelManager`
- Add env var validation to `zeus doctor`

When adding a new Talos tool:
- Implement `TalosTool` trait
- Register in the appropriate category module
- Export from `zeus_talos::` public API
