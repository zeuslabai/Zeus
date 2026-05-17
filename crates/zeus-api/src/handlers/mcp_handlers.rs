//! MCP (Model Context Protocol) server management handlers.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::info;

use crate::SharedState;

#[derive(Debug, Deserialize)]
pub struct AddMcpServerRequest {
    pub name: String,
    pub transport: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
}

/// List MCP server connections
pub async fn list_mcp_servers(State(state): State<SharedState>) -> Json<Value> {
    let state = state.read().await;

    // Read MCP config from config file if it exists
    let mcp_config_path = dirs::home_dir()
        .unwrap_or_default()
        .join(".zeus")
        .join("mcp.json");

    let servers = if mcp_config_path.exists() {
        match tokio::fs::read_to_string(&mcp_config_path).await {
            Ok(content) => match serde_json::from_str::<Value>(&content) {
                Ok(val) => {
                    if let Some(servers) = val.get("servers").and_then(|s| s.as_array()) {
                        servers.clone()
                    } else {
                        Vec::new()
                    }
                }
                Err(_) => Vec::new(),
            },
            Err(_) => Vec::new(),
        }
    } else {
        Vec::new()
    };

    let _ = &state.config; // acknowledge state usage
    Json(json!({ "servers": servers }))
}

/// Add/connect a new MCP server
pub async fn add_mcp_server(
    State(_state): State<SharedState>,
    Json(req): Json<AddMcpServerRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let mcp_config_path = dirs::home_dir()
        .unwrap_or_default()
        .join(".zeus")
        .join("mcp.json");

    // Load existing config or create new
    let mut config = if mcp_config_path.exists() {
        match tokio::fs::read_to_string(&mcp_config_path).await {
            Ok(content) => {
                serde_json::from_str::<Value>(&content).unwrap_or(json!({"servers": []}))
            }
            Err(_) => json!({"servers": []}),
        }
    } else {
        json!({"servers": []})
    };

    let id = req.name.to_lowercase().replace(' ', "-");

    let server_entry = json!({
        "id": id,
        "name": req.name,
        "transport": req.transport,
        "command": req.command,
        "args": req.args,
        "env": req.env,
        "status": "configured"
    });

    // Upsert into servers array — replace existing entry with the same id, or append.
    if let Some(servers) = config.get_mut("servers").and_then(|s| s.as_array_mut()) {
        if let Some(pos) = servers
            .iter()
            .position(|s| s.get("id").and_then(|v| v.as_str()) == Some(&id))
        {
            servers[pos] = server_entry;
        } else {
            servers.push(server_entry);
        }
    }

    // Save config
    if let Some(parent) = mcp_config_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }
    tokio::fs::write(
        &mcp_config_path,
        serde_json::to_string_pretty(&config).unwrap_or_default(),
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    info!("Added MCP server: {}", id);

    Ok(Json(json!({
        "success": true,
        "id": id,
        "message": format!("MCP server '{}' added", req.name)
    })))
}

/// Delete/disconnect an MCP server
pub async fn delete_mcp_server(
    State(_state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let mcp_config_path = dirs::home_dir()
        .unwrap_or_default()
        .join(".zeus")
        .join("mcp.json");

    if !mcp_config_path.exists() {
        return Err((
            StatusCode::NOT_FOUND,
            format!("MCP server not found: {}", id),
        ));
    }

    let content = tokio::fs::read_to_string(&mcp_config_path)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut config: Value = serde_json::from_str(&content)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let removed = if let Some(servers) = config.get_mut("servers").and_then(|s| s.as_array_mut()) {
        let before = servers.len();
        servers.retain(|s| s.get("id").and_then(|i| i.as_str()) != Some(&id));
        servers.len() < before
    } else {
        false
    };

    if !removed {
        return Err((
            StatusCode::NOT_FOUND,
            format!("MCP server not found: {}", id),
        ));
    }

    tokio::fs::write(
        &mcp_config_path,
        serde_json::to_string_pretty(&config).unwrap_or_default(),
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    info!("Removed MCP server: {}", id);

    Ok(Json(json!({
        "success": true,
        "id": id,
        "message": format!("MCP server '{}' removed", id)
    })))
}

/// List tools from a specific MCP server
pub async fn list_mcp_server_tools(
    State(_state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let mcp_config_path = dirs::home_dir()
        .unwrap_or_default()
        .join(".zeus")
        .join("mcp.json");

    if !mcp_config_path.exists() {
        return Err((
            StatusCode::NOT_FOUND,
            format!("MCP server not found: {}", id),
        ));
    }

    let content = tokio::fs::read_to_string(&mcp_config_path)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let config: Value = serde_json::from_str(&content)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let server = config
        .get("servers")
        .and_then(|s| s.as_array())
        .and_then(|servers| {
            servers
                .iter()
                .find(|s| s.get("id").and_then(|i| i.as_str()) == Some(&id))
        });

    if server.is_none() {
        return Err((
            StatusCode::NOT_FOUND,
            format!("MCP server not found: {}", id),
        ));
    }

    // Tools would come from a live MCP connection; for now return stored tools or empty
    let tools = server
        .and_then(|s| s.get("tools"))
        .and_then(|t| t.as_array())
        .cloned()
        .unwrap_or_default();

    Ok(Json(json!({
        "server_id": id,
        "tools": tools
    })))
}

/// POST /v1/mcp/tools/:tool/test — Test an MCP tool with sample input
///
/// For core Zeus tools: executes the tool and returns the result.
/// For MCP-only tools: validates input against the tool's schema from mcp.json.
pub async fn test_mcp_tool(
    State(state): State<SharedState>,
    Path(tool): Path<String>,
    Json(req): Json<Value>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let input = req.get("arguments").cloned().unwrap_or(json!({}));
    info!("Test MCP tool '{}' with input: {:?}", tool, input);

    // Check if this is a core Zeus tool — if so, execute it directly
    let state_guard = state.read().await;
    let core_names: Vec<String> = state_guard
        .tools
        .schemas()
        .iter()
        .map(|s| s.name.clone())
        .collect();
    drop(state_guard);

    if core_names.contains(&tool) {
        match zeus_agent::execute_tool(&tool, input.clone()).await {
            Ok(output) => {
                return Ok(Json(json!({
                    "tool": tool,
                    "status": "success",
                    "output": output,
                    "source": "core",
                })));
            }
            Err(e) => {
                return Ok(Json(json!({
                    "tool": tool,
                    "status": "error",
                    "error": e.to_string(),
                    "source": "core",
                })));
            }
        }
    }

    // Check MCP servers config for tool existence
    let mcp_path = dirs::home_dir()
        .unwrap_or_default()
        .join(".zeus")
        .join("mcp.json");

    if !mcp_path.exists() {
        return Err((
            StatusCode::NOT_FOUND,
            format!(
                "Tool '{}' not found in core tools or MCP servers (no mcp.json)",
                tool
            ),
        ));
    }

    let content = tokio::fs::read_to_string(&mcp_path).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to read mcp.json: {e}"),
        )
    })?;

    let servers: Vec<Value> = serde_json::from_str(&content)
        .or_else(|_| {
            // Also try { "servers": [...] } format
            serde_json::from_str::<Value>(&content).map(|v| {
                v.get("servers")
                    .and_then(|s| s.as_array())
                    .cloned()
                    .unwrap_or_default()
            })
        })
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to parse mcp.json: {e}"),
            )
        })?;

    // Find the tool in MCP server configs
    for server in &servers {
        let server_name = server
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("unknown");
        if let Some(tools) = server.get("tools").and_then(|t| t.as_array()) {
            for t in tools {
                let tname = t.get("name").and_then(|n| n.as_str()).unwrap_or("");
                if tname == tool {
                    // Validate required params if inputSchema is available
                    let mut missing = Vec::new();
                    if let Some(schema) = t.get("inputSchema").or_else(|| t.get("parameters"))
                        && let Some(required) = schema.get("required").and_then(|r| r.as_array())
                    {
                        for req_field in required {
                            if let Some(field) = req_field.as_str()
                                && input.get(field).is_none()
                            {
                                missing.push(field.to_string());
                            }
                        }
                    }

                    if !missing.is_empty() {
                        return Ok(Json(json!({
                            "tool": tool,
                            "status": "validation_error",
                            "server": server_name,
                            "missing_fields": missing,
                            "source": "mcp",
                        })));
                    }

                    return Ok(Json(json!({
                        "tool": tool,
                        "status": "validated",
                        "server": server_name,
                        "description": t.get("description").and_then(|d| d.as_str()).unwrap_or(""),
                        "source": "mcp",
                        "note": "Tool exists and input validates. Runtime execution requires a connected MCP server.",
                    })));
                }
            }
        }
    }

    Err((
        StatusCode::NOT_FOUND,
        format!(
            "Tool '{}' not found in core tools or any configured MCP server",
            tool
        ),
    ))
}
