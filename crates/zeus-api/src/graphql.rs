//! GraphQL API layer for Zeus
//!
//! Exposes a unified GraphQL endpoint at `/v1/graphql`:
//! - **Query**: sessions, session, tools, memory_search, agents, agent_status,
//!              channels, channel_status
//! - **Mutation**: execute_tool, remember, spawn_agent
//! - **Subscription**: messages (real-time via AppState::gql_broadcast),
//!                     plan_updates (Prometheus step events)
//!
//! Uses `async-graphql` with `async-graphql-axum` integration.

use async_graphql::{Context, Object, Schema, SimpleObject, Subscription};
use std::path::PathBuf;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt as TokioStreamExt;

use crate::SharedState;
use crate::websocket::PlanEvent;

// ── Output types ──────────────────────────────────────────────────────────────

/// A Zeus session summary
#[derive(SimpleObject, Clone)]
pub struct SessionInfo {
    pub id: String,
    pub created: String,
    pub message_count: i32,
}

/// A registered tool schema
#[derive(SimpleObject, Clone)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
}

/// Result of executing a tool
#[derive(SimpleObject, Clone)]
pub struct ToolExecuteResult {
    pub success: bool,
    pub output: Option<String>,
    pub error: Option<String>,
}

/// A memory search result
#[derive(SimpleObject, Clone)]
pub struct MemoryResult {
    pub id: String,
    pub content: String,
    pub score: Option<f64>,
}

/// Agent information
#[derive(SimpleObject, Clone)]
pub struct AgentInfo {
    pub id: String,
    pub name: String,
    pub status: String,
    pub model: String,
    pub role: String,
}

/// Result of spawning an agent
#[derive(SimpleObject, Clone)]
pub struct AgentSpawnResult {
    pub agent_id: String,
    pub name: String,
    pub spawned_at: String,
}

/// Channel status information
#[derive(SimpleObject, Clone)]
pub struct ChannelInfo {
    pub id: String,
    pub channel_type: String,
    pub name: String,
    pub status: String,
    pub enabled: bool,
}

/// Real-time message event (subscription payload)
#[derive(SimpleObject, Clone)]
pub struct MessageEvent {
    pub id: String,
    pub content: String,
    pub source: String,
    pub timestamp: String,
}

/// Plan step event (subscription payload)
#[derive(SimpleObject, Clone)]
pub struct PlanStepEvent {
    pub plan_id: String,
    pub step_id: i32,
    pub status: String,
    pub progress_pct: f64,
    pub output: String,
}

// ── Broadcast channel ─────────────────────────────────────────────────────────

/// Broadcast channel for real-time message events delivered to GraphQL subscriptions.
///
/// Store one instance in `AppState`; call `send()` from inbound/chat handlers
/// to push events to all active `messages` subscribers.
#[derive(Clone)]
pub struct GqlBroadcast(pub std::sync::Arc<tokio::sync::broadcast::Sender<MessageEvent>>);

impl GqlBroadcast {
    /// Create a new broadcast channel with the given buffer capacity.
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = tokio::sync::broadcast::channel(capacity);
        Self(std::sync::Arc::new(tx))
    }

    /// Publish a message event to all active subscriptions (non-blocking; dropped if no receivers).
    pub fn send(&self, event: MessageEvent) {
        let _ = self.0.send(event);
    }
}

// ── Query ─────────────────────────────────────────────────────────────────────

pub struct QueryRoot;

#[Object]
impl QueryRoot {
    /// List sessions (paginated)
    async fn sessions(
        &self,
        ctx: &Context<'_>,
        #[graphql(default = 0)] offset: i32,
        #[graphql(default = 20)] limit: i32,
    ) -> async_graphql::Result<Vec<SessionInfo>> {
        let state = ctx.data_unchecked::<SharedState>().read().await;
        let sessions = zeus_session::Session::list(&state.config.sessions)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;

        let skip = offset.max(0) as usize;
        let take = limit.max(1).min(zeus_core::MAX_PAGE_LIMIT as i32) as usize;
        let mut result = Vec::new();

        for (id, created) in sessions.into_iter().skip(skip).take(take) {
            let count = zeus_session::Session::load(&state.config.sessions, &id)
                .await
                .map(|s| s.messages.len() as i32)
                .unwrap_or(0);
            result.push(SessionInfo {
                id,
                created: created.to_rfc3339(),
                message_count: count,
            });
        }
        Ok(result)
    }

    /// Get a single session by ID
    async fn session(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> async_graphql::Result<Option<SessionInfo>> {
        let state = ctx.data_unchecked::<SharedState>().read().await;
        match zeus_session::Session::load(&state.config.sessions, &id).await {
            Ok(s) => Ok(Some(SessionInfo {
                id: s.id.clone(),
                created: s.created.to_rfc3339(),
                message_count: s.messages.len() as i32,
            })),
            Err(_) => Ok(None),
        }
    }

    /// List all registered tools
    async fn tools(&self, ctx: &Context<'_>) -> Vec<ToolSchema> {
        let state = ctx.data_unchecked::<SharedState>().read().await;
        state
            .tools
            .schemas()
            .iter()
            .map(|s| ToolSchema {
                name: s.name.clone(),
                description: s.description.clone(),
            })
            .collect()
    }

    /// Search memory (uses Mnemosyne semantic search when available, falls back to file scan)
    async fn memory_search(
        &self,
        ctx: &Context<'_>,
        query: String,
        #[graphql(default = 10)] limit: i32,
    ) -> async_graphql::Result<Vec<MemoryResult>> {
        let limit = limit.max(1).min(zeus_core::MAX_PAGE_LIMIT_SMALL as i32) as usize;
        let state = ctx.data_unchecked::<SharedState>().read().await;

        // Semantic search via Mnemosyne
        if let Some(ref mnemosyne) = state.mnemosyne {
            let mn = mnemosyne.clone();
            drop(state);
            let hits = mn
                .semantic_search(&query, limit)
                .await
                .map_err(|e| async_graphql::Error::new(e.to_string()))?;
            return Ok(hits
                .iter()
                .map(|r| MemoryResult {
                    id: r.id.to_string(),
                    content: r.content.clone(),
                    score: Some(r.score as f64),
                })
                .collect());
        }

        // File-based fallback
        let root = state.workspace.root().to_path_buf();
        drop(state);
        if !root.exists() {
            return Ok(vec![]);
        }
        let query_lower = query.to_lowercase();
        let mut results = Vec::new();
        search_memory_files(&root, &query_lower, limit, &mut results);
        Ok(results)
    }

    /// List all configured agents
    async fn agents(&self) -> Vec<AgentInfo> {
        read_agents_from_dir(&agents_dir())
    }

    /// Get the runtime status of a spawned agent instance
    async fn agent_status(&self, ctx: &Context<'_>, id: String) -> Option<AgentInfo> {
        let state = ctx.data_unchecked::<SharedState>().read().await;
        state.agent_registry.get(&id).map(|inst| AgentInfo {
            id: inst.agent_id.clone(),
            name: inst.name.clone(),
            status: "spawned".to_string(),
            model: String::new(),
            role: String::new(),
        })
    }

    /// List all configured channels
    async fn channels(&self, ctx: &Context<'_>) -> Vec<ChannelInfo> {
        let state = ctx.data_unchecked::<SharedState>().read().await;
        state
            .channel_store
            .list()
            .await
            .into_iter()
            .map(channel_to_info)
            .collect()
    }

    /// Get status of a specific channel by ID
    async fn channel_status(&self, ctx: &Context<'_>, id: String) -> Option<ChannelInfo> {
        let state = ctx.data_unchecked::<SharedState>().read().await;
        state
            .channel_store
            .list()
            .await
            .into_iter()
            .find(|ch| ch.id == id)
            .map(channel_to_info)
    }
}

// ── Mutation ──────────────────────────────────────────────────────────────────

pub struct MutationRoot;

#[Object]
impl MutationRoot {
    /// Execute a registered tool by name with JSON arguments
    async fn execute_tool(
        &self,
        ctx: &Context<'_>,
        name: String,
        arguments: Option<async_graphql::Json<serde_json::Value>>,
    ) -> ToolExecuteResult {
        let args = arguments.map(|a| a.0).unwrap_or(serde_json::Value::Null);
        let state = ctx.data_unchecked::<SharedState>().read().await;
        match state.tools.execute(&name, args).await {
            Ok(output) => ToolExecuteResult {
                success: true,
                output: Some(output.to_string()),
                error: None,
            },
            Err(e) => ToolExecuteResult {
                success: false,
                output: None,
                error: Some(e.to_string()),
            },
        }
    }

    /// Store a fact in the workspace memory
    async fn remember(&self, ctx: &Context<'_>, fact: String) -> async_graphql::Result<bool> {
        let state = ctx.data_unchecked::<SharedState>().read().await;
        state
            .workspace
            .remember(&fact)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        Ok(true)
    }

    /// Spawn a configured agent by its ID
    async fn spawn_agent(
        &self,
        ctx: &Context<'_>,
        agent_id: String,
    ) -> async_graphql::Result<AgentSpawnResult> {
        let mut state = ctx.data_unchecked::<SharedState>().write().await;
        state
            .agent_registry
            .spawn(&agent_id)
            .await
            .map_err(|e| async_graphql::Error::new(e))?;
        let inst = state
            .agent_registry
            .get(&agent_id)
            .ok_or_else(|| async_graphql::Error::new("Agent spawned but not found in registry"))?;
        Ok(AgentSpawnResult {
            agent_id: inst.agent_id.clone(),
            name: inst.name.clone(),
            spawned_at: inst.spawned_at.to_rfc3339(),
        })
    }
}

// ── Subscription ──────────────────────────────────────────────────────────────

pub struct SubscriptionRoot;

#[Subscription]
impl SubscriptionRoot {
    /// Stream of real-time message events.
    ///
    /// Events are published by calling `AppState::gql_broadcast.send(MessageEvent { ... })`
    /// from any handler (e.g. inbound chat, channel adapters).
    async fn messages(
        &self,
        ctx: &Context<'_>,
    ) -> impl futures::Stream<Item = MessageEvent> {
        let rx = {
            let state = ctx.data_unchecked::<SharedState>().read().await;
            state.gql_broadcast.0.subscribe()
        };
        BroadcastStream::new(rx).filter_map(|r| r.ok())
    }

    /// Stream of Prometheus plan execution step updates.
    async fn plan_updates(
        &self,
        ctx: &Context<'_>,
    ) -> impl futures::Stream<Item = PlanStepEvent> {
        let rx = {
            let state = ctx.data_unchecked::<SharedState>().read().await;
            state.plan_broadcast.subscribe()
        };
        BroadcastStream::new(rx).filter_map(|r| {
            match r {
                Ok(PlanEvent::StepUpdate(u)) => Some(PlanStepEvent {
                    plan_id: u.plan_id,
                    step_id: u.step_id as i32,
                    status: u.status,
                    progress_pct: u.progress_pct,
                    output: u.output,
                }),
                _ => None,
            }
        })
    }
}

// ── Schema ────────────────────────────────────────────────────────────────────

/// The Zeus GraphQL schema type
pub type ZeusSchema = Schema<QueryRoot, MutationRoot, SubscriptionRoot>;

/// Build the GraphQL schema wired to the shared app state.
///
/// The schema holds a clone of `SharedState` so resolvers can access all app data.
pub fn build_schema(state: SharedState) -> ZeusSchema {
    Schema::build(QueryRoot, MutationRoot, SubscriptionRoot)
        .data(state)
        .finish()
}

// ── GraphiQL playground ───────────────────────────────────────────────────────

/// Serve the GraphiQL in-browser IDE at GET /v1/graphql
pub async fn graphiql_handler() -> impl axum::response::IntoResponse {
    axum::response::Html(
        async_graphql::http::GraphiQLSource::build()
            .endpoint("/v1/graphql")
            .subscription_endpoint("/v1/graphql/ws")
            .finish(),
    )
}

// ── Private helpers ───────────────────────────────────────────────────────────

fn agents_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/home"))
        .join(".zeus")
        .join("agents")
}

fn read_agents_from_dir(dir: &PathBuf) -> Vec<AgentInfo> {
    let mut agents = Vec::new();
    if !dir.exists() {
        return agents;
    }
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                        agents.push(AgentInfo {
                            id: val["id"].as_str().unwrap_or("").to_string(),
                            name: val["name"].as_str().unwrap_or("").to_string(),
                            status: val["status"].as_str().unwrap_or("active").to_string(),
                            model: val["model"].as_str().unwrap_or("").to_string(),
                            role: val["role"].as_str().unwrap_or("").to_string(),
                        });
                    }
                }
            }
        }
    }
    agents
}

fn channel_to_info(ch: crate::channels::Channel) -> ChannelInfo {
    ChannelInfo {
        id: ch.id,
        channel_type: ch.channel_type.to_string(),
        name: ch.name,
        status: if ch.enabled {
            "connected".to_string()
        } else {
            "disabled".to_string()
        },
        enabled: ch.enabled,
    }
}

fn search_memory_files(
    dir: &std::path::Path,
    query: &str,
    limit: usize,
    results: &mut Vec<MemoryResult>,
) {
    if results.len() >= limit {
        return;
    }
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if results.len() >= limit {
                break;
            }
            let path = entry.path();
            if path.is_dir() {
                search_memory_files(&path, query, limit, results);
            } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if content.to_lowercase().contains(query) {
                        let snippet: String = content
                            .lines()
                            .find(|l| l.to_lowercase().contains(query))
                            .unwrap_or("")
                            .chars()
                            .take(200)
                            .collect();
                        results.push(MemoryResult {
                            id: path.to_string_lossy().to_string(),
                            content: snippet,
                            score: None,
                        });
                    }
                }
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use async_graphql::EmptySubscription;

    // Helper: minimal schema with no-op context (for type/serialization tests)
    fn minimal_schema() -> Schema<QueryRoot, MutationRoot, EmptySubscription> {
        Schema::build(QueryRoot, MutationRoot, EmptySubscription).finish()
    }

    #[test]
    fn test_schema_builds() {
        let _schema = minimal_schema();
    }

    #[test]
    fn test_session_info_serialization() {
        let info = SessionInfo {
            id: "sess-1".to_string(),
            created: "2026-01-01T00:00:00Z".to_string(),
            message_count: 5,
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("sess-1"));
        assert!(json.contains("5"));
    }

    #[test]
    fn test_tool_execute_result_success() {
        let r = ToolExecuteResult {
            success: true,
            output: Some("hello".to_string()),
            error: None,
        };
        assert!(r.success);
        assert_eq!(r.output.as_deref(), Some("hello"));
    }

    #[test]
    fn test_tool_execute_result_failure() {
        let r = ToolExecuteResult {
            success: false,
            output: None,
            error: Some("not found".to_string()),
        };
        assert!(!r.success);
        assert_eq!(r.error.as_deref(), Some("not found"));
    }

    #[test]
    fn test_channel_info_status() {
        let enabled = ChannelInfo {
            id: "ch1".to_string(),
            channel_type: "telegram".to_string(),
            name: "Bot".to_string(),
            status: "connected".to_string(),
            enabled: true,
        };
        let disabled = ChannelInfo {
            id: "ch2".to_string(),
            channel_type: "slack".to_string(),
            name: "Workspace".to_string(),
            status: "disabled".to_string(),
            enabled: false,
        };
        assert_eq!(enabled.status, "connected");
        assert_eq!(disabled.status, "disabled");
    }

    #[test]
    fn test_gql_broadcast_send_with_no_receivers() {
        let broadcast = GqlBroadcast::new(16);
        // Should not panic even when there are no receivers
        broadcast.send(MessageEvent {
            id: "msg-1".to_string(),
            content: "hello".to_string(),
            source: "telegram".to_string(),
            timestamp: "2026-01-01T00:00:00Z".to_string(),
        });
    }

    #[test]
    fn test_gql_broadcast_send_and_receive() {
        let broadcast = GqlBroadcast::new(16);
        let mut rx = broadcast.0.subscribe();
        let event = MessageEvent {
            id: "msg-2".to_string(),
            content: "world".to_string(),
            source: "discord".to_string(),
            timestamp: "2026-01-01T00:00:00Z".to_string(),
        };
        broadcast.send(event.clone());
        let received = rx.try_recv().expect("should receive event");
        assert_eq!(received.id, "msg-2");
        assert_eq!(received.content, "world");
    }

    #[test]
    fn test_memory_result_fields() {
        let r = MemoryResult {
            id: "path/to/file.md".to_string(),
            content: "relevant snippet".to_string(),
            score: Some(0.95),
        };
        assert_eq!(r.score, Some(0.95));
    }

    #[test]
    fn test_plan_step_event_fields() {
        let e = PlanStepEvent {
            plan_id: "plan-abc".to_string(),
            step_id: 2,
            status: "completed".to_string(),
            progress_pct: 66.6,
            output: "step done".to_string(),
        };
        assert_eq!(e.step_id, 2);
        assert!((e.progress_pct - 66.6).abs() < 0.01);
    }

    #[test]
    fn test_agent_spawn_result_fields() {
        let r = AgentSpawnResult {
            agent_id: "agent-1".to_string(),
            name: "Researcher".to_string(),
            spawned_at: "2026-01-01T00:00:00Z".to_string(),
        };
        assert_eq!(r.agent_id, "agent-1");
    }

    #[test]
    fn test_search_memory_files_empty_dir() {
        let tmp = std::env::temp_dir().join("zeus_gql_test_empty");
        std::fs::create_dir_all(&tmp).unwrap();
        let mut results = Vec::new();
        search_memory_files(&tmp, "anything", 10, &mut results);
        assert!(results.is_empty());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_search_memory_files_finds_match() {
        let tmp = std::env::temp_dir().join("zeus_gql_test_match");
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("notes.md"), "This is a test of zeus memory search").unwrap();
        let mut results = Vec::new();
        search_memory_files(&tmp, "zeus memory", 10, &mut results);
        assert!(!results.is_empty());
        assert!(results[0].content.contains("zeus memory"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_search_memory_files_respects_limit() {
        let tmp = std::env::temp_dir().join("zeus_gql_test_limit");
        std::fs::create_dir_all(&tmp).unwrap();
        for i in 0..5 {
            std::fs::write(tmp.join(format!("note{}.md", i)), "keyword match here").unwrap();
        }
        let mut results = Vec::new();
        search_memory_files(&tmp, "keyword", 2, &mut results);
        assert!(results.len() <= 2);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[tokio::test]
    async fn test_schema_introspection_query() {
        let schema = minimal_schema();
        let result = schema
            .execute("{ __schema { queryType { name } } }")
            .await;
        assert!(result.errors.is_empty());
        let data = result.data.into_json().unwrap();
        assert_eq!(data["__schema"]["queryType"]["name"], "QueryRoot");
    }

    #[tokio::test]
    async fn test_schema_query_type_fields() {
        let schema = minimal_schema();
        let result = schema
            .execute("{ __type(name: \"QueryRoot\") { fields { name } } }")
            .await;
        assert!(result.errors.is_empty());
        let data = result.data.into_json().unwrap();
        let fields: Vec<String> = data["__type"]["fields"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["name"].as_str().unwrap().to_string())
            .collect();
        assert!(fields.contains(&"sessions".to_string()));
        assert!(fields.contains(&"tools".to_string()));
        assert!(fields.contains(&"agents".to_string()));
        assert!(fields.contains(&"channels".to_string()));
        assert!(fields.contains(&"memorySearch".to_string()));
        assert!(fields.contains(&"agentStatus".to_string()));
        assert!(fields.contains(&"channelStatus".to_string()));
        assert!(fields.contains(&"session".to_string()));
    }

    #[tokio::test]
    async fn test_mutation_type_fields() {
        let schema = minimal_schema();
        let result = schema
            .execute("{ __type(name: \"MutationRoot\") { fields { name } } }")
            .await;
        assert!(result.errors.is_empty());
        let data = result.data.into_json().unwrap();
        let fields: Vec<String> = data["__type"]["fields"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["name"].as_str().unwrap().to_string())
            .collect();
        assert!(fields.contains(&"executeTool".to_string()));
        assert!(fields.contains(&"remember".to_string()));
        assert!(fields.contains(&"spawnAgent".to_string()));
    }
}
