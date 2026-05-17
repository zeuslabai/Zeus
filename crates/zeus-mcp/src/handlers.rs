//! MCP Request Handlers
//!
//! Handles MCP protocol methods and tool execution

use std::sync::Arc;

use serde_json::{Value, json};
use tracing::{debug, info};
use zeus_agent::ToolRegistry;
use zeus_core::ToolSchema;
use zeus_memory::Workspace;
use zeus_mnemosyne::MemoryStore;

use crate::agents::McpAgentManager;
use crate::protocol::{
    McpError, McpMethod, McpRequest, McpResponse, PromptDefinition, ResourceDefinition,
    ToolDefinition,
};

/// Agent tool names handled separately from ToolRegistry
const AGENT_TOOLS: &[&str] = &["list_agents", "spawn_agent", "agent_status"];

/// Graph memory tool names handled via Mnemosyne
const MEMORY_GRAPH_TOOLS: &[&str] = &["memory_graph", "memory_communities", "memory_graph_search"];

/// Network messaging tool names (agent-to-agent communication)
const NETWORK_TOOLS: &[&str] = &["agent_send_message", "agent_broadcast", "agent_invoke"];

/// Tool handler for MCP requests
pub struct ToolHandler {
    tools: ToolRegistry,
    workspace: Option<Workspace>,
    agent_manager: Option<Arc<tokio::sync::Mutex<McpAgentManager>>>,
    memory_store: Option<Arc<std::sync::Mutex<MemoryStore>>>,
}

impl ToolHandler {
    /// Create a new tool handler with default tool registry
    pub fn new() -> Self {
        Self {
            tools: ToolRegistry::with_defaults(),
            workspace: None,
            agent_manager: None,
            memory_store: None,
        }
    }

    /// Create handler with workspace access
    pub fn with_workspace(workspace: Workspace) -> Self {
        Self {
            tools: ToolRegistry::with_defaults(),
            workspace: Some(workspace),
            agent_manager: None,
            memory_store: None,
        }
    }

    /// Create handler with a pre-built tool registry
    pub fn with_registry(tools: ToolRegistry, workspace: Option<Workspace>) -> Self {
        Self {
            tools,
            workspace,
            agent_manager: None,
            memory_store: None,
        }
    }

    /// Attach an agent manager for spawn/list/status tools
    pub fn with_agents(mut self, mgr: Arc<tokio::sync::Mutex<McpAgentManager>>) -> Self {
        self.agent_manager = Some(mgr);
        self
    }

    /// Attach a Mnemosyne memory store for graph memory tools
    pub fn with_mnemosyne(mut self, store: Arc<std::sync::Mutex<MemoryStore>>) -> Self {
        self.memory_store = Some(store);
        self
    }

    /// Handle an MCP request
    pub async fn handle(&self, request: McpRequest) -> McpResponse {
        let method = McpMethod::from(request.method.as_str());

        debug!("Handling MCP method: {:?}", method);

        match method {
            McpMethod::Initialize => self.handle_initialize(request).await,
            McpMethod::ListTools => self.handle_list_tools(request).await,
            McpMethod::CallTool => self.handle_call_tool(request).await,
            McpMethod::ListResources => self.handle_list_resources(request).await,
            McpMethod::ReadResource => self.handle_read_resource(request).await,
            McpMethod::ListPrompts => self.handle_list_prompts(request).await,
            McpMethod::GetPrompt => self.handle_get_prompt(request).await,
            McpMethod::Notification => {
                // MCP notifications (e.g. notifications/initialized) require no response.
                // Return empty success to avoid error -32601 in Claude Code logs.
                McpResponse::success(request.id, serde_json::json!({}))
            }
            McpMethod::Unknown => {
                McpResponse::error(request.id, McpError::method_not_found(&request.method))
            }
        }
    }

    /// Handle initialize request
    async fn handle_initialize(&self, request: McpRequest) -> McpResponse {
        info!("MCP client initializing");

        McpResponse::success(
            request.id,
            json!({
                "protocolVersion": "2024-11-05",
                "serverInfo": {
                    "name": "zeus-mcp",
                    "version": crate::VERSION
                },
                "capabilities": {
                    "tools": {},
                    "resources": {},
                    "prompts": {}
                }
            }),
        )
    }

    /// Handle tools/list request
    async fn handle_list_tools(&self, request: McpRequest) -> McpResponse {
        let mut schemas = self.tools.schemas();

        // Append agent tool schemas if agent manager is configured
        if self.agent_manager.is_some() {
            schemas.extend(McpAgentManager::agent_tool_schemas());
        }

        // Append graph memory tool schemas if Mnemosyne is configured
        if self.memory_store.is_some() {
            schemas.extend(Self::memory_graph_tool_schemas());
        }

        // Always include network messaging tools
        schemas.extend(Self::network_tool_schemas());

        let tools: Vec<ToolDefinition> =
            schemas.iter().map(|s| self.schema_to_tool_def(s)).collect();

        McpResponse::success(request.id, json!({ "tools": tools }))
    }

    /// Handle tools/call request
    async fn handle_call_tool(&self, request: McpRequest) -> McpResponse {
        let name = match request.params.get("name").and_then(|n| n.as_str()) {
            Some(n) => n.to_string(),
            None => {
                return McpResponse::error(
                    request.id,
                    McpError::invalid_params("Missing 'name' parameter"),
                );
            }
        };

        let arguments = request
            .params
            .get("arguments")
            .cloned()
            .unwrap_or(Value::Object(Default::default()));

        debug!("Calling tool: {} with args: {:?}", name, arguments);

        // Route agent tools to McpAgentManager
        if AGENT_TOOLS.contains(&name.as_str()) {
            if let Some(ref mgr) = self.agent_manager {
                let mut mgr = mgr.lock().await;
                match mgr.execute_tool(&name, arguments).await {
                    Ok(output) => {
                        return McpResponse::success(
                            request.id,
                            json!({
                                "content": [{
                                    "type": "text",
                                    "text": output
                                }]
                            }),
                        );
                    }
                    Err(e) => {
                        return McpResponse::error(request.id, McpError::tool_error(e));
                    }
                }
            } else {
                return McpResponse::error(
                    request.id,
                    McpError::tool_error("Agent manager not configured".to_string()),
                );
            }
        }

        // Route graph memory tools to Mnemosyne
        if MEMORY_GRAPH_TOOLS.contains(&name.as_str()) {
            if let Some(ref store) = self.memory_store {
                let store = store.lock().map_err(|e| {
                    McpError::tool_error(format!("Memory store lock poisoned: {}", e))
                });
                match store {
                    Ok(store) => {
                        let result = Self::execute_memory_tool(&store, &name, &arguments);
                        return match result {
                            Ok(output) => McpResponse::success(
                                request.id,
                                json!({
                                    "content": [{
                                        "type": "text",
                                        "text": output
                                    }]
                                }),
                            ),
                            Err(e) => {
                                McpResponse::error(request.id, McpError::tool_error(e.to_string()))
                            }
                        };
                    }
                    Err(e) => {
                        return McpResponse::error(request.id, e);
                    }
                }
            } else {
                return McpResponse::error(
                    request.id,
                    McpError::tool_error("Mnemosyne memory store not configured".to_string()),
                );
            }
        }

        // Route network messaging tools
        if NETWORK_TOOLS.contains(&name.as_str()) {
            let result = Self::execute_network_tool(&name, &arguments).await;
            return match result {
                Ok(output) => McpResponse::success(
                    request.id,
                    json!({
                        "content": [{
                            "type": "text",
                            "text": output
                        }]
                    }),
                ),
                Err(e) => McpResponse::error(request.id, McpError::tool_error(e)),
            };
        }

        match self.tools.execute(&name, arguments).await {
            Ok(output) => {
                // Check for structured MCP content (e.g. image blocks from analyze_image).
                // Tools return a JSON string with "_mcp_content" to pass rich content
                // (images, embeds) directly to Claude Code's multimodal vision.
                if let Ok(parsed) = serde_json::from_str::<Value>(&output)
                    && let Some(content) = parsed.get("_mcp_content")
                    && content.is_array()
                {
                    return McpResponse::success(request.id, json!({ "content": content }));
                }
                McpResponse::success(
                    request.id,
                    json!({
                        "content": [{
                            "type": "text",
                            "text": output
                        }]
                    }),
                )
            }
            Err(e) => McpResponse::error(request.id, McpError::tool_error(e.to_string())),
        }
    }

    /// Handle resources/list request
    async fn handle_list_resources(&self, request: McpRequest) -> McpResponse {
        let mut resources: Vec<ResourceDefinition> = vec![];

        // Add workspace files as resources if available
        if let Some(workspace) = &self.workspace {
            resources.push(ResourceDefinition {
                uri: format!("file://{}/AGENTS.md", workspace.root().display()),
                name: "AGENTS.md".to_string(),
                description: Some("System prompt and agent configuration".to_string()),
                mime_type: Some("text/markdown".to_string()),
            });
            resources.push(ResourceDefinition {
                uri: format!("file://{}/SOUL.md", workspace.root().display()),
                name: "SOUL.md".to_string(),
                description: Some("Personality configuration".to_string()),
                mime_type: Some("text/markdown".to_string()),
            });
            resources.push(ResourceDefinition {
                uri: format!("file://{}/USER.md", workspace.root().display()),
                name: "USER.md".to_string(),
                description: Some("User context and preferences".to_string()),
                mime_type: Some("text/markdown".to_string()),
            });
            resources.push(ResourceDefinition {
                uri: format!("file://{}/memory/MEMORY.md", workspace.root().display()),
                name: "MEMORY.md".to_string(),
                description: Some("Long-term memory storage".to_string()),
                mime_type: Some("text/markdown".to_string()),
            });
        }

        McpResponse::success(request.id, json!({ "resources": resources }))
    }

    /// Handle resources/read request
    async fn handle_read_resource(&self, request: McpRequest) -> McpResponse {
        let uri = match request.params.get("uri").and_then(|u| u.as_str()) {
            Some(u) => u,
            None => {
                return McpResponse::error(
                    request.id,
                    McpError::invalid_params("Missing 'uri' parameter"),
                );
            }
        };

        let Some(workspace) = &self.workspace else {
            return McpResponse::error(
                request.id,
                McpError::invalid_params("No workspace configured for resource access"),
            );
        };

        // Extract path from file:// URI and derive relative workspace path.
        // Delegate all validation (traversal, symlink) to Workspace::read().
        let abs_path = uri.strip_prefix("file://").unwrap_or(uri);
        let abs_path = std::path::Path::new(abs_path);

        let rel_path = match abs_path.strip_prefix(workspace.root()) {
            Ok(rel) => rel.to_string_lossy().to_string(),
            Err(_) => {
                // Path doesn't start with workspace root — try normalizing
                // in case it contains .. components
                let mut normalized = std::path::PathBuf::new();
                for component in abs_path.components() {
                    match component {
                        std::path::Component::ParentDir => {
                            normalized.pop();
                        }
                        other => normalized.push(other),
                    }
                }
                match normalized.strip_prefix(workspace.root()) {
                    Ok(rel) => rel.to_string_lossy().to_string(),
                    Err(_) => {
                        return McpResponse::error(
                            request.id,
                            McpError::invalid_params("Access denied: path outside workspace"),
                        );
                    }
                }
            }
        };

        // Use workspace.read() which validates the path (traversal + symlink checks)
        match workspace.read(&rel_path).await {
            Ok(content) => McpResponse::success(
                request.id,
                json!({
                    "contents": [{
                        "uri": uri,
                        "mimeType": "text/markdown",
                        "text": content
                    }]
                }),
            ),
            Err(e) => McpResponse::error(
                request.id,
                McpError::internal_error(format!("Failed to read resource: {}", e)),
            ),
        }
    }

    /// Handle prompts/list request
    async fn handle_list_prompts(&self, request: McpRequest) -> McpResponse {
        let prompts = vec![
            PromptDefinition {
                name: "summarize".to_string(),
                description: Some("Summarize content".to_string()),
                arguments: Some(vec![crate::protocol::PromptArgument {
                    name: "content".to_string(),
                    description: Some("Content to summarize".to_string()),
                    required: Some(true),
                }]),
            },
            PromptDefinition {
                name: "analyze".to_string(),
                description: Some("Analyze code or text".to_string()),
                arguments: Some(vec![crate::protocol::PromptArgument {
                    name: "content".to_string(),
                    description: Some("Content to analyze".to_string()),
                    required: Some(true),
                }]),
            },
        ];

        McpResponse::success(request.id, json!({ "prompts": prompts }))
    }

    /// Handle prompts/get request
    async fn handle_get_prompt(&self, request: McpRequest) -> McpResponse {
        let name = match request.params.get("name").and_then(|n| n.as_str()) {
            Some(n) => n,
            None => {
                return McpResponse::error(
                    request.id,
                    McpError::invalid_params("Missing 'name' parameter"),
                );
            }
        };

        let arguments = request
            .params
            .get("arguments")
            .cloned()
            .unwrap_or(Value::Object(Default::default()));

        let content = arguments
            .get("content")
            .and_then(|c| c.as_str())
            .unwrap_or("");

        let prompt_text = match name {
            "summarize" => format!(
                "Please provide a concise summary of the following content:\n\n{}",
                content
            ),
            "analyze" => format!(
                "Please analyze the following content and provide insights:\n\n{}",
                content
            ),
            _ => {
                return McpResponse::error(
                    request.id,
                    McpError::invalid_params(format!("Unknown prompt: {}", name)),
                );
            }
        };

        McpResponse::success(
            request.id,
            json!({
                "messages": [{
                    "role": "user",
                    "content": {
                        "type": "text",
                        "text": prompt_text
                    }
                }]
            }),
        )
    }

    /// Convert ToolSchema to MCP ToolDefinition
    fn schema_to_tool_def(&self, schema: &ToolSchema) -> ToolDefinition {
        // ToolSchema.parameters is already a JSON schema object
        ToolDefinition {
            name: schema.name.clone(),
            description: schema.description.clone(),
            input_schema: schema.parameters.clone(),
        }
    }

    /// Tool schemas for graph memory tools exposed via MCP.
    pub fn memory_graph_tool_schemas() -> Vec<ToolSchema> {
        vec![
            ToolSchema {
                name: "memory_graph".to_string(),
                description: "Traverse the entity knowledge graph. Given an entity name or ID, returns its graph neighborhood: connected entities, relationships, and communities.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "entity": {
                            "type": "string",
                            "description": "Entity name to look up (e.g. 'Alice', 'Zeus')"
                        },
                        "entity_id": {
                            "type": "integer",
                            "description": "Entity ID (alternative to name)"
                        },
                        "depth": {
                            "type": "integer",
                            "description": "Max traversal depth in hops (default: 2, max: 4)",
                            "default": 2
                        }
                    }
                }),
            },
            ToolSchema {
                name: "memory_communities".to_string(),
                description: "List all detected entity communities (clusters of related entities). Shows community names, member counts, hub/bridge roles, and summaries.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "community_id": {
                            "type": "integer",
                            "description": "Optional: get details for a specific community ID"
                        }
                    }
                }),
            },
            ToolSchema {
                name: "memory_graph_search".to_string(),
                description: "Graph-augmented memory search. Expands the query using knowledge graph relationships for richer recall, then returns results with graph context.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "Search query text"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Maximum results to return (default: 10)",
                            "default": 10
                        }
                    },
                    "required": ["query"]
                }),
            },
        ]
    }

    /// Execute a graph memory tool against the Mnemosyne store.
    fn execute_memory_tool(
        store: &MemoryStore,
        name: &str,
        arguments: &Value,
    ) -> std::result::Result<String, String> {
        match name {
            "memory_graph" => Self::exec_memory_graph(store, arguments),
            "memory_communities" => Self::exec_memory_communities(store, arguments),
            "memory_graph_search" => Self::exec_memory_graph_search(store, arguments),
            _ => Err(format!("Unknown memory tool: {}", name)),
        }
    }

    /// Execute memory_graph: entity graph traversal
    fn exec_memory_graph(store: &MemoryStore, args: &Value) -> std::result::Result<String, String> {
        let depth = args
            .get("depth")
            .and_then(|v| v.as_u64())
            .unwrap_or(2)
            .min(4) as u32;

        // Resolve entity by name or ID
        let entity_id = if let Some(id) = args.get("entity_id").and_then(|v| v.as_i64()) {
            id
        } else if let Some(name) = args.get("entity").and_then(|v| v.as_str()) {
            // Search entities by name
            let entities = store.get_entities(10_000).map_err(|e| e.to_string())?;
            let name_lower = name.to_lowercase();
            entities
                .iter()
                .find(|e| e.canonical_name.to_lowercase() == name_lower)
                .or_else(|| {
                    entities
                        .iter()
                        .find(|e| e.aliases.iter().any(|a| a.to_lowercase() == name_lower))
                })
                .map(|e| e.id)
                .ok_or_else(|| format!("Entity '{}' not found", name))?
        } else {
            return Err("Provide either 'entity' (name) or 'entity_id'".to_string());
        };

        let traversal = store
            .get_entity_graph(entity_id, depth)
            .map_err(|e| e.to_string())?;

        // Format the graph context
        let graph_text = store
            .format_graph_context(entity_id, depth)
            .map_err(|e| e.to_string())?;

        // Check community membership
        let community = store
            .get_entity_community(entity_id)
            .map_err(|e| e.to_string())?;

        let result = json!({
            "entity": {
                "id": traversal.origin.id,
                "name": traversal.origin.canonical_name,
                "type": traversal.origin.entity_type,
                "mention_count": traversal.origin.mention_count,
            },
            "nodes": traversal.nodes.iter().map(|n| json!({
                "name": n.entity.canonical_name,
                "type": n.entity.entity_type,
                "relationship": n.relationship_type.as_label(),
                "depth": n.depth,
            })).collect::<Vec<_>>(),
            "edge_count": traversal.edges.len(),
            "community": community.map(|c| json!({
                "id": c.id,
                "name": c.name,
                "entity_count": c.entity_count,
            })),
            "graph_context": graph_text,
        });

        serde_json::to_string_pretty(&result).map_err(|e| e.to_string())
    }

    /// Execute memory_communities: list or detail communities
    fn exec_memory_communities(
        store: &MemoryStore,
        args: &Value,
    ) -> std::result::Result<String, String> {
        if let Some(community_id) = args.get("community_id").and_then(|v| v.as_i64()) {
            // Detail for a specific community
            let members = store
                .get_community_entities(community_id)
                .map_err(|e| e.to_string())?;
            let summary = zeus_mnemosyne::community::community_summary(store, community_id);

            let result = json!({
                "community_id": community_id,
                "summary": summary,
                "members": members.iter().map(|(entity, role)| json!({
                    "id": entity.id,
                    "name": entity.canonical_name,
                    "type": entity.entity_type,
                    "role": role,
                    "mention_count": entity.mention_count,
                })).collect::<Vec<_>>(),
            });

            return serde_json::to_string_pretty(&result).map_err(|e| e.to_string());
        }

        // List all communities
        let communities = store.get_communities().map_err(|e| e.to_string())?;
        let rel_count = store.relationship_count().map_err(|e| e.to_string())?;
        let rel_types = store.get_relationship_types().map_err(|e| e.to_string())?;

        let result = json!({
            "communities": communities.iter().map(|c| json!({
                "id": c.id,
                "name": c.name,
                "description": c.description,
                "entity_count": c.entity_count,
            })).collect::<Vec<_>>(),
            "community_count": communities.len(),
            "total_relationships": rel_count,
            "relationship_types": rel_types.iter().map(|rt| json!({
                "type": rt.relationship_type,
                "count": rt.count,
            })).collect::<Vec<_>>(),
        });

        serde_json::to_string_pretty(&result).map_err(|e| e.to_string())
    }

    /// Execute memory_graph_search: graph-augmented search
    fn exec_memory_graph_search(
        store: &MemoryStore,
        args: &Value,
    ) -> std::result::Result<String, String> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Missing required 'query' parameter".to_string())?;

        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;

        let results = zeus_mnemosyne::graph_search::graph_augmented_search(store, query, limit)
            .map_err(|e| e.to_string())?;

        let result = json!({
            "query": query,
            "result_count": results.len(),
            "results": results.iter().map(|r| json!({
                "content": r.result.content,
                "score": r.result.score,
                "session_id": r.result.session_id,
                "memory_type": format!("{:?}", r.result.memory_type),
                "graph_context": r.context_text,
                "entities": r.graph_context.entities.iter().map(|e| json!({
                    "name": e.canonical_name,
                    "type": e.entity_type,
                })).collect::<Vec<_>>(),
                "community": r.graph_context.community,
            })).collect::<Vec<_>>(),
        });

        serde_json::to_string_pretty(&result).map_err(|e| e.to_string())
    }

    // -- Network messaging tools ----------------------------------------------

    /// Tool schemas for inter-agent network messaging.
    pub fn network_tool_schemas() -> Vec<ToolSchema> {
        vec![
            ToolSchema {
                name: "agent_send_message".to_string(),
                description: "Send a message to another Zeus agent on the local network. The message is delivered to the peer's gateway, which routes it to the agent inbox (standalone mode) or tmux session if available.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "host": {
                            "type": "string",
                            "description": "Target peer IP address (e.g. '192.168.1.100')"
                        },
                        "port": {
                            "type": "integer",
                            "description": "Target peer port (default: 8080)",
                            "default": 8080
                        },
                        "from_agent": {
                            "type": "string",
                            "description": "Your agent name (e.g. '@zeus_bot')"
                        },
                        "to_agent": {
                            "type": "string",
                            "description": "Target agent name (optional)"
                        },
                        "content": {
                            "type": "string",
                            "description": "Message content to send"
                        }
                    },
                    "required": ["host", "from_agent", "content"]
                }),
            },
            ToolSchema {
                name: "agent_broadcast".to_string(),
                description: "Broadcast a message to ALL Zeus agents discovered via mDNS on the local network. The message is sent to every known peer's gateway.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "from_agent": {
                            "type": "string",
                            "description": "Your agent name (e.g. '@zeus_bot')"
                        },
                        "content": {
                            "type": "string",
                            "description": "Message content to broadcast"
                        }
                    },
                    "required": ["from_agent", "content"]
                }),
            },
            ToolSchema {
                name: "agent_invoke".to_string(),
                description: "Invoke a method on a connected fleet agent via WebSocket (request-response). The agent must be connected to the hub via /v1/ws/nodes. Unlike agent_send_message (fire-and-forget), this waits for the agent's response.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "node_id": {
                            "type": "string",
                            "description": "Target agent's node ID (e.g. '@zeus100')"
                        },
                        "method": {
                            "type": "string",
                            "description": "Method to invoke (e.g. 'ping', 'status', 'shell', 'tmux_send')"
                        },
                        "params": {
                            "type": "object",
                            "description": "Parameters for the method (optional)",
                            "default": {}
                        },
                        "timeout": {
                            "type": "integer",
                            "description": "Timeout in seconds (default: 30)",
                            "default": 30
                        }
                    },
                    "required": ["node_id", "method"]
                }),
            },
        ]
    }

    /// Execute a network messaging tool.
    async fn execute_network_tool(
        name: &str,
        arguments: &Value,
    ) -> std::result::Result<String, String> {
        match name {
            "agent_send_message" => Self::exec_agent_send(arguments).await,
            "agent_broadcast" => Self::exec_agent_broadcast(arguments).await,
            "agent_invoke" => Self::exec_agent_invoke(arguments).await,
            _ => Err(format!("Unknown network tool: {}", name)),
        }
    }

    /// Execute agent_send_message: POST to a peer's /v1/network/messages
    async fn exec_agent_send(args: &Value) -> std::result::Result<String, String> {
        let host = args
            .get("host")
            .and_then(|v| v.as_str())
            .ok_or("Missing required 'host' parameter")?;
        let port = args.get("port").and_then(|v| v.as_u64()).unwrap_or(8080);
        let from_agent = args
            .get("from_agent")
            .and_then(|v| v.as_str())
            .ok_or("Missing required 'from_agent' parameter")?;
        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or("Missing required 'content' parameter")?;
        let to_agent = args.get("to_agent").and_then(|v| v.as_str());

        let local_ip = std::env::var("ZEUS_HOST_IP").unwrap_or_else(|_| {
            hostname::get()
                .ok()
                .and_then(|h| h.to_str().map(String::from))
                .unwrap_or_else(|| "127.0.0.1".to_string())
        });

        let url = format!("http://{}:{}/v1/network/messages", host, port);
        let mut payload = serde_json::json!({
            "from_agent": from_agent,
            "from_host": local_ip,
            "content": content,
        });
        if let Some(to) = to_agent {
            payload["to_agent"] = json!(to);
        }

        let client = reqwest::Client::new();
        let resp = client
            .post(&url)
            .json(&payload)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| format!("Failed to reach {}: {}", url, e))?;

        let status = resp.status();
        let body: Value = resp
            .json()
            .await
            .unwrap_or(json!({"error": "invalid response"}));

        if status.is_success() {
            let msg_id = body
                .get("message_id")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let delivered = body
                .get("delivered")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            Ok(format!(
                "Message sent to {} ({}:{}). ID: {}, delivered: {}",
                to_agent.unwrap_or("default agent"),
                host,
                port,
                msg_id,
                delivered
            ))
        } else {
            Err(format!("Peer returned HTTP {}: {}", status, body))
        }
    }

    /// Execute agent_invoke: POST to local gateway's /v1/nodes/:id/invoke
    async fn exec_agent_invoke(args: &Value) -> std::result::Result<String, String> {
        let node_id = args
            .get("node_id")
            .and_then(|v| v.as_str())
            .ok_or("Missing required 'node_id' parameter")?;
        let method = args
            .get("method")
            .and_then(|v| v.as_str())
            .ok_or("Missing required 'method' parameter")?;
        let params = args.get("params").cloned().unwrap_or(json!({}));
        let timeout = args.get("timeout").and_then(|v| v.as_u64()).unwrap_or(30);

        let gateway_port = std::env::var("ZEUS_API_PORT")
            .ok()
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(8080);
        let url = format!(
            "http://127.0.0.1:{}/v1/nodes/{}/invoke",
            gateway_port, node_id
        );

        let payload = serde_json::json!({
            "method": method,
            "params": params,
            "timeout": timeout,
        });

        let client = reqwest::Client::new();
        let resp = client
            .post(&url)
            .json(&payload)
            .timeout(std::time::Duration::from_secs(timeout + 5)) // slightly longer than invoke timeout
            .send()
            .await
            .map_err(|e| format!("Failed to reach local gateway: {}", e))?;

        let status = resp.status();
        let body: Value = resp
            .json()
            .await
            .unwrap_or(json!({"error": "invalid response"}));

        if status.is_success() && body.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
            let result = body.get("result").cloned().unwrap_or(Value::Null);
            Ok(format!(
                "Invoke '{}' on '{}' succeeded: {}",
                method,
                node_id,
                serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string())
            ))
        } else {
            let error = body
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            Err(format!(
                "Invoke '{}' on '{}' failed: {}",
                method, node_id, error
            ))
        }
    }

    /// Execute agent_broadcast: POST to local gateway's /v1/network/broadcast
    async fn exec_agent_broadcast(args: &Value) -> std::result::Result<String, String> {
        let from_agent = args
            .get("from_agent")
            .and_then(|v| v.as_str())
            .ok_or("Missing required 'from_agent' parameter")?;
        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or("Missing required 'content' parameter")?;

        // Broadcast via local gateway
        let gateway_port = std::env::var("ZEUS_API_PORT")
            .ok()
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(8080);
        let url = format!("http://127.0.0.1:{}/v1/network/broadcast", gateway_port);

        let payload = serde_json::json!({
            "from_agent": from_agent,
            "content": content,
        });

        let client = reqwest::Client::new();
        let resp = client
            .post(&url)
            .json(&payload)
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| format!("Failed to reach local gateway: {}", e))?;

        let status = resp.status();
        let body: Value = resp
            .json()
            .await
            .unwrap_or(json!({"error": "invalid response"}));

        if status.is_success() {
            let count = body
                .get("broadcast_count")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let success = body
                .get("success_count")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            Ok(format!(
                "Broadcast complete. Sent to {} peers, {} successful.",
                count, success
            ))
        } else {
            Err(format!("Broadcast failed (HTTP {}): {}", status, body))
        }
    }
}

impl Default for ToolHandler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_handler_new() {
        let _handler = ToolHandler::new(); // should not panic
    }

    #[test]
    fn test_handler_default() {
        let _handler = ToolHandler::default();
    }

    #[test]
    fn test_handler_with_workspace() {
        let ws = Workspace::new("/tmp/test-ws");
        let _handler = ToolHandler::with_workspace(ws);
    }

    #[test]
    fn test_handler_with_registry() {
        let registry = ToolRegistry::with_defaults();
        let _handler = ToolHandler::with_registry(registry, None);
    }

    #[test]
    fn test_handler_with_registry_and_workspace() {
        let registry = ToolRegistry::with_defaults();
        let ws = Workspace::new("/tmp/test-ws-reg");
        let _handler = ToolHandler::with_registry(registry, Some(ws));
    }

    #[test]
    fn test_handler_with_agents() {
        let config = zeus_core::Config::default();
        let mgr = Arc::new(tokio::sync::Mutex::new(McpAgentManager::new(config)));
        let handler = ToolHandler::new().with_agents(mgr);
        assert!(handler.agent_manager.is_some());
    }

    #[tokio::test]
    async fn test_list_tools_includes_agent_tools() {
        let config = zeus_core::Config::default();
        let mgr = Arc::new(tokio::sync::Mutex::new(McpAgentManager::new(config)));
        let handler = ToolHandler::new().with_agents(mgr);
        let req = McpRequest::new("tools/list", json!({}));
        let resp = handler.handle(req).await;

        let result = resp.result.expect("should succeed");
        let tools = result["tools"].as_array().expect("should be array");
        let names: Vec<&str> = tools
            .iter()
            .map(|t| t["name"].as_str().unwrap_or(""))
            .collect();
        assert!(names.contains(&"list_agents"));
        assert!(names.contains(&"spawn_agent"));
        assert!(names.contains(&"agent_status"));
    }

    #[tokio::test]
    async fn test_call_agent_tool_without_manager() {
        let handler = ToolHandler::new(); // no agent manager
        let req = McpRequest::new(
            "tools/call",
            json!({"name": "list_agents", "arguments": {}}),
        );
        let resp = handler.handle(req).await;
        assert!(resp.error.is_some());
        assert!(
            resp.error
                .unwrap()
                .message
                .contains("Agent manager not configured")
        );
    }

    #[tokio::test]
    async fn test_call_agent_list_with_manager() {
        let config = zeus_core::Config::default();
        let mgr = Arc::new(tokio::sync::Mutex::new(McpAgentManager::new(config)));
        let handler = ToolHandler::new().with_agents(mgr);
        let req = McpRequest::new(
            "tools/call",
            json!({"name": "list_agents", "arguments": {}}),
        );
        let resp = handler.handle(req).await;
        assert!(resp.result.is_some());
        let result = resp.result.unwrap();
        let text = result["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("[]")); // empty list
    }

    #[tokio::test]
    async fn test_handle_initialize() {
        let handler = ToolHandler::new();
        let req = McpRequest::new("initialize", json!({}));
        let resp = handler.handle(req).await;

        assert!(resp.result.is_some());
        assert!(resp.error.is_none());
        let result = resp.result.expect("operation should succeed");
        assert_eq!(result["protocolVersion"], "2024-11-05");
        assert_eq!(result["serverInfo"]["name"], "zeus-mcp");
    }

    #[tokio::test]
    async fn test_handle_list_tools() {
        let handler = ToolHandler::new();
        let req = McpRequest::new("tools/list", json!({}));
        let resp = handler.handle(req).await;

        assert!(resp.result.is_some());
        let result = resp.result.expect("operation should succeed");
        let tools = result["tools"].as_array().expect("should be an array");
        assert!(!tools.is_empty());
        // Each tool should have name, description, inputSchema
        let first = &tools[0];
        assert!(first.get("name").is_some());
        assert!(first.get("description").is_some());
        assert!(first.get("inputSchema").is_some());
    }

    #[tokio::test]
    async fn test_handle_call_tool_missing_name() {
        let handler = ToolHandler::new();
        let req = McpRequest::new("tools/call", json!({}));
        let resp = handler.handle(req).await;

        assert!(resp.error.is_some());
        assert_eq!(resp.error.expect("operation should succeed").code, -32602); // invalid params
    }

    #[tokio::test]
    async fn test_handle_list_resources_no_workspace() {
        let handler = ToolHandler::new();
        let req = McpRequest::new("resources/list", json!({}));
        let resp = handler.handle(req).await;

        assert!(resp.result.is_some());
        let result = resp.result.expect("operation should succeed");
        let resources = result["resources"].as_array().expect("should be an array");
        assert!(resources.is_empty()); // no workspace = no resources
    }

    #[tokio::test]
    async fn test_handle_list_resources_with_workspace() {
        let ws = Workspace::new("/tmp/zeus-test-ws");
        let handler = ToolHandler::with_workspace(ws);
        let req = McpRequest::new("resources/list", json!({}));
        let resp = handler.handle(req).await;

        assert!(resp.result.is_some());
        let result = resp.result.expect("operation should succeed");
        let resources = result["resources"].as_array().expect("should be an array");
        assert_eq!(resources.len(), 4);
        let names: Vec<&str> = resources
            .iter()
            .map(|r| r["name"].as_str().expect("should be a string"))
            .collect();
        assert!(names.contains(&"AGENTS.md"));
        assert!(names.contains(&"SOUL.md"));
        assert!(names.contains(&"USER.md"));
        assert!(names.contains(&"MEMORY.md"));
    }

    #[tokio::test]
    async fn test_handle_read_resource_no_workspace() {
        let handler = ToolHandler::new();
        let req = McpRequest::new("resources/read", json!({"uri": "file:///test"}));
        let resp = handler.handle(req).await;

        assert!(resp.error.is_some());
        assert!(
            resp.error
                .expect("operation should succeed")
                .message
                .contains("No workspace")
        );
    }

    #[tokio::test]
    async fn test_handle_read_resource_missing_uri() {
        let handler = ToolHandler::new();
        let req = McpRequest::new("resources/read", json!({}));
        let resp = handler.handle(req).await;

        assert!(resp.error.is_some());
        assert_eq!(resp.error.expect("operation should succeed").code, -32602);
    }

    #[tokio::test]
    async fn test_handle_list_prompts() {
        let handler = ToolHandler::new();
        let req = McpRequest::new("prompts/list", json!({}));
        let resp = handler.handle(req).await;

        assert!(resp.result.is_some());
        let result = resp.result.expect("operation should succeed");
        let prompts = result["prompts"].as_array().expect("should be an array");
        assert_eq!(prompts.len(), 2);
        let names: Vec<&str> = prompts
            .iter()
            .map(|p| p["name"].as_str().expect("should be a string"))
            .collect();
        assert!(names.contains(&"summarize"));
        assert!(names.contains(&"analyze"));
    }

    #[tokio::test]
    async fn test_handle_get_prompt_summarize() {
        let handler = ToolHandler::new();
        let req = McpRequest::new(
            "prompts/get",
            json!({"name": "summarize", "arguments": {"content": "test data"}}),
        );
        let resp = handler.handle(req).await;

        assert!(resp.result.is_some());
        let result = resp.result.expect("operation should succeed");
        let messages = result["messages"].as_array().expect("should be an array");
        assert_eq!(messages.len(), 1);
        let text = messages[0]["content"]["text"]
            .as_str()
            .expect("should be a string");
        assert!(text.contains("summary"));
        assert!(text.contains("test data"));
    }

    #[tokio::test]
    async fn test_handle_get_prompt_analyze() {
        let handler = ToolHandler::new();
        let req = McpRequest::new(
            "prompts/get",
            json!({"name": "analyze", "arguments": {"content": "code here"}}),
        );
        let resp = handler.handle(req).await;

        assert!(resp.result.is_some());
        let result = resp.result.expect("operation should succeed");
        let text = result["messages"][0]["content"]["text"]
            .as_str()
            .expect("should be a string");
        assert!(text.contains("analyze"));
        assert!(text.contains("code here"));
    }

    #[tokio::test]
    async fn test_handle_get_prompt_unknown() {
        let handler = ToolHandler::new();
        let req = McpRequest::new("prompts/get", json!({"name": "nonexistent"}));
        let resp = handler.handle(req).await;

        assert!(resp.error.is_some());
        assert!(
            resp.error
                .expect("operation should succeed")
                .message
                .contains("Unknown prompt")
        );
    }

    #[tokio::test]
    async fn test_handle_get_prompt_missing_name() {
        let handler = ToolHandler::new();
        let req = McpRequest::new("prompts/get", json!({}));
        let resp = handler.handle(req).await;

        assert!(resp.error.is_some());
        assert_eq!(resp.error.expect("operation should succeed").code, -32602);
    }

    #[tokio::test]
    async fn test_handle_unknown_method() {
        let handler = ToolHandler::new();
        let req = McpRequest::new("nonexistent/method", json!({}));
        let resp = handler.handle(req).await;

        assert!(resp.error.is_some());
        assert_eq!(resp.error.expect("operation should succeed").code, -32601); // method not found
    }

    // === Graph Memory Tool Tests ===

    fn handler_with_mnemosyne() -> ToolHandler {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_mcp_graph.db");
        let store = zeus_mnemosyne::MemoryStore::new(&db_path, true, false).unwrap();

        // Seed test entities and relationships
        let alice = store.upsert_entity("Alice", "person").unwrap();
        let zeus = store.upsert_entity("Zeus", "project").unwrap();
        let mnemosyne = store.upsert_entity("Mnemosyne", "component").unwrap();
        store
            .add_relationship(
                alice,
                zeus,
                zeus_mnemosyne::graph::RelationType::WorksOn,
                1.0,
            )
            .unwrap();
        store
            .add_relationship(
                mnemosyne,
                zeus,
                zeus_mnemosyne::graph::RelationType::PartOf,
                1.0,
            )
            .unwrap();

        // Detect communities so we have data
        zeus_mnemosyne::community::detect_communities(&store).unwrap();

        let store = Arc::new(std::sync::Mutex::new(store));
        // Leak the tempdir so DB persists for test duration
        std::mem::forget(dir);
        ToolHandler::new().with_mnemosyne(store)
    }

    #[test]
    fn test_memory_graph_tool_schemas() {
        let schemas = ToolHandler::memory_graph_tool_schemas();
        assert_eq!(schemas.len(), 3);
        let names: Vec<&str> = schemas.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"memory_graph"));
        assert!(names.contains(&"memory_communities"));
        assert!(names.contains(&"memory_graph_search"));
    }

    #[tokio::test]
    async fn test_list_tools_includes_memory_tools() {
        let handler = handler_with_mnemosyne();
        let req = McpRequest::new("tools/list", json!({}));
        let resp = handler.handle(req).await;

        let result = resp.result.expect("should succeed");
        let tools = result["tools"].as_array().expect("should be array");
        let names: Vec<&str> = tools
            .iter()
            .map(|t| t["name"].as_str().unwrap_or(""))
            .collect();
        assert!(
            names.contains(&"memory_graph"),
            "should include memory_graph"
        );
        assert!(
            names.contains(&"memory_communities"),
            "should include memory_communities"
        );
        assert!(
            names.contains(&"memory_graph_search"),
            "should include memory_graph_search"
        );
    }

    #[tokio::test]
    async fn test_memory_graph_by_name() {
        let handler = handler_with_mnemosyne();
        let req = McpRequest::new(
            "tools/call",
            json!({"name": "memory_graph", "arguments": {"entity": "Alice", "depth": 2}}),
        );
        let resp = handler.handle(req).await;
        assert!(resp.error.is_none(), "should succeed: {:?}", resp.error);

        let result = resp.result.unwrap();
        let text = result["content"][0]["text"].as_str().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(parsed["entity"]["name"], "Alice");
        assert!(parsed["nodes"].as_array().unwrap().len() >= 1);
    }

    #[tokio::test]
    async fn test_memory_graph_entity_not_found() {
        let handler = handler_with_mnemosyne();
        let req = McpRequest::new(
            "tools/call",
            json!({"name": "memory_graph", "arguments": {"entity": "NonexistentEntity"}}),
        );
        let resp = handler.handle(req).await;
        assert!(
            resp.error.is_some(),
            "should return error for unknown entity"
        );
    }

    #[tokio::test]
    async fn test_memory_graph_missing_args() {
        let handler = handler_with_mnemosyne();
        let req = McpRequest::new(
            "tools/call",
            json!({"name": "memory_graph", "arguments": {}}),
        );
        let resp = handler.handle(req).await;
        assert!(resp.error.is_some(), "should require entity or entity_id");
    }

    #[tokio::test]
    async fn test_memory_communities_list() {
        let handler = handler_with_mnemosyne();
        let req = McpRequest::new(
            "tools/call",
            json!({"name": "memory_communities", "arguments": {}}),
        );
        let resp = handler.handle(req).await;
        assert!(resp.error.is_none(), "should succeed: {:?}", resp.error);

        let result = resp.result.unwrap();
        let text = result["content"][0]["text"].as_str().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
        assert!(parsed["community_count"].as_u64().unwrap() >= 1);
        assert!(parsed["total_relationships"].as_u64().unwrap() >= 2);
    }

    #[tokio::test]
    async fn test_memory_graph_search_empty() {
        let handler = handler_with_mnemosyne();
        let req = McpRequest::new(
            "tools/call",
            json!({"name": "memory_graph_search", "arguments": {"query": "nonexistent topic xyz"}}),
        );
        let resp = handler.handle(req).await;
        assert!(
            resp.error.is_none(),
            "should succeed even with no results: {:?}",
            resp.error
        );

        let result = resp.result.unwrap();
        let text = result["content"][0]["text"].as_str().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(parsed["query"], "nonexistent topic xyz");
    }

    #[tokio::test]
    async fn test_memory_graph_search_missing_query() {
        let handler = handler_with_mnemosyne();
        let req = McpRequest::new(
            "tools/call",
            json!({"name": "memory_graph_search", "arguments": {}}),
        );
        let resp = handler.handle(req).await;
        assert!(resp.error.is_some(), "should require query parameter");
    }

    #[tokio::test]
    async fn test_memory_tool_without_store() {
        let handler = ToolHandler::new(); // no mnemosyne
        let req = McpRequest::new(
            "tools/call",
            json!({"name": "memory_graph", "arguments": {"entity": "Alice"}}),
        );
        let resp = handler.handle(req).await;
        assert!(resp.error.is_some());
        assert!(
            resp.error
                .unwrap()
                .message
                .contains("Mnemosyne memory store not configured")
        );
    }
}
