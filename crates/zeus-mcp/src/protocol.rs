//! MCP Protocol types
//!
//! JSON-RPC 2.0 based protocol for Model Context Protocol

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// MCP Request (JSON-RPC 2.0)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpRequest {
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

impl McpRequest {
    pub fn new(method: impl Into<String>, params: Value) -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static REQUEST_ID: AtomicU64 = AtomicU64::new(1);

        Self {
            jsonrpc: "2.0".to_string(),
            id: Some(Value::Number(
                REQUEST_ID.fetch_add(1, Ordering::Relaxed).into(),
            )),
            method: method.into(),
            params,
        }
    }
}

/// MCP Response (JSON-RPC 2.0)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<McpError>,
}

impl McpResponse {
    pub fn success(id: Option<Value>, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn error(id: Option<Value>, error: McpError) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(error),
        }
    }
}

/// MCP Error
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl McpError {
    pub fn parse_error() -> Self {
        Self {
            code: -32700,
            message: "Parse error".to_string(),
            data: None,
        }
    }

    pub fn invalid_request(msg: impl Into<String>) -> Self {
        Self {
            code: -32600,
            message: msg.into(),
            data: None,
        }
    }

    pub fn method_not_found(method: &str) -> Self {
        Self {
            code: -32601,
            message: format!("Method not found: {}", method),
            data: None,
        }
    }

    pub fn invalid_params(msg: impl Into<String>) -> Self {
        Self {
            code: -32602,
            message: msg.into(),
            data: None,
        }
    }

    pub fn internal_error(msg: impl Into<String>) -> Self {
        Self {
            code: -32603,
            message: msg.into(),
            data: None,
        }
    }

    pub fn tool_error(msg: impl Into<String>) -> Self {
        Self {
            code: -32000,
            message: msg.into(),
            data: None,
        }
    }
}

/// MCP Methods supported by this server
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpMethod {
    /// Initialize connection
    Initialize,
    /// List available tools
    ListTools,
    /// Call a tool
    CallTool,
    /// List available resources
    ListResources,
    /// Read a resource
    ReadResource,
    /// List available prompts
    ListPrompts,
    /// Get a prompt
    GetPrompt,
    /// MCP notification (no response expected per JSON-RPC spec)
    Notification,
    /// Unknown method
    Unknown,
}

impl From<&str> for McpMethod {
    fn from(s: &str) -> Self {
        match s {
            "initialize" => McpMethod::Initialize,
            "tools/list" => McpMethod::ListTools,
            "tools/call" => McpMethod::CallTool,
            "resources/list" => McpMethod::ListResources,
            "resources/read" => McpMethod::ReadResource,
            "prompts/list" => McpMethod::ListPrompts,
            "prompts/get" => McpMethod::GetPrompt,
            s if s.starts_with("notifications/") => McpMethod::Notification,
            _ => McpMethod::Unknown,
        }
    }
}

/// Tool definition for MCP
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

/// Resource definition for MCP
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceDefinition {
    pub uri: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "mimeType", skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

/// Prompt definition for MCP
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptDefinition {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Vec<PromptArgument>>,
}

/// Prompt argument
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptArgument {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // -- McpRequest tests ---------------------------------------------------

    #[test]
    fn test_request_new() {
        let req = McpRequest::new("tools/list", json!({}));
        assert_eq!(req.jsonrpc, "2.0");
        assert_eq!(req.method, "tools/list");
        assert!(req.id.is_some());
    }

    #[test]
    fn test_request_unique_ids() {
        let r1 = McpRequest::new("a", json!({}));
        let r2 = McpRequest::new("b", json!({}));
        assert_ne!(r1.id, r2.id);
    }

    #[test]
    fn test_request_serialization() {
        let req = McpRequest::new("initialize", json!({"key": "value"}));
        let json_str = serde_json::to_string(&req).expect("should serialize to JSON");
        let de: McpRequest = serde_json::from_str(&json_str).expect("should parse successfully");
        assert_eq!(de.jsonrpc, "2.0");
        assert_eq!(de.method, "initialize");
    }

    #[test]
    fn test_request_from_json() {
        let json =
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"read_file"}}"#;
        let req: McpRequest = serde_json::from_str(json).expect("should parse successfully");
        assert_eq!(req.method, "tools/call");
        assert_eq!(req.params["name"], "read_file");
    }

    #[test]
    fn test_request_no_params() {
        let json = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
        let req: McpRequest = serde_json::from_str(json).expect("should parse successfully");
        assert_eq!(req.params, Value::Null);
    }

    #[test]
    fn test_request_null_id() {
        let json = r#"{"jsonrpc":"2.0","method":"notification","params":{}}"#;
        let req: McpRequest = serde_json::from_str(json).expect("should parse successfully");
        assert!(req.id.is_none());
    }

    // -- McpResponse tests --------------------------------------------------

    #[test]
    fn test_response_success() {
        let resp = McpResponse::success(Some(Value::Number(1.into())), json!({"status": "ok"}));
        assert_eq!(resp.jsonrpc, "2.0");
        assert!(resp.result.is_some());
        assert!(resp.error.is_none());
    }

    #[test]
    fn test_response_error() {
        let resp = McpResponse::error(
            Some(Value::Number(1.into())),
            McpError::method_not_found("test"),
        );
        assert!(resp.result.is_none());
        assert!(resp.error.is_some());
        assert_eq!(resp.error.expect("operation should succeed").code, -32601);
    }

    #[test]
    fn test_response_success_serialization() {
        let resp = McpResponse::success(Some(Value::Number(1.into())), json!({"ok": true}));
        let json = serde_json::to_string(&resp).expect("should serialize to JSON");
        assert!(json.contains("result"));
        assert!(!json.contains("error"));
    }

    #[test]
    fn test_response_error_serialization() {
        let resp = McpResponse::error(Some(Value::Number(1.into())), McpError::parse_error());
        let json = serde_json::to_string(&resp).expect("should serialize to JSON");
        assert!(json.contains("error"));
        assert!(!json.contains("result"));
    }

    #[test]
    fn test_response_roundtrip() {
        let resp = McpResponse::success(Some(Value::Number(42.into())), json!({"tools": []}));
        let json = serde_json::to_string(&resp).expect("should serialize to JSON");
        let de: McpResponse = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(de.id, Some(Value::Number(42.into())));
        assert!(de.result.is_some());
    }

    // -- McpError tests -----------------------------------------------------

    #[test]
    fn test_error_parse_error() {
        let e = McpError::parse_error();
        assert_eq!(e.code, -32700);
        assert_eq!(e.message, "Parse error");
        assert!(e.data.is_none());
    }

    #[test]
    fn test_error_invalid_request() {
        let e = McpError::invalid_request("bad format");
        assert_eq!(e.code, -32600);
        assert_eq!(e.message, "bad format");
    }

    #[test]
    fn test_error_method_not_found() {
        let e = McpError::method_not_found("foo/bar");
        assert_eq!(e.code, -32601);
        assert!(e.message.contains("foo/bar"));
    }

    #[test]
    fn test_error_invalid_params() {
        let e = McpError::invalid_params("missing name");
        assert_eq!(e.code, -32602);
        assert_eq!(e.message, "missing name");
    }

    #[test]
    fn test_error_internal() {
        let e = McpError::internal_error("crash");
        assert_eq!(e.code, -32603);
    }

    #[test]
    fn test_error_tool_error() {
        let e = McpError::tool_error("tool failed");
        assert_eq!(e.code, -32000);
        assert_eq!(e.message, "tool failed");
    }

    #[test]
    fn test_error_serialization() {
        let e = McpError::method_not_found("test");
        let json = serde_json::to_string(&e).expect("should serialize to JSON");
        let de: McpError = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(de.code, -32601);
    }

    // -- McpMethod tests ----------------------------------------------------

    #[test]
    fn test_method_from_str() {
        assert_eq!(McpMethod::from("initialize"), McpMethod::Initialize);
        assert_eq!(McpMethod::from("tools/list"), McpMethod::ListTools);
        assert_eq!(McpMethod::from("tools/call"), McpMethod::CallTool);
        assert_eq!(McpMethod::from("resources/list"), McpMethod::ListResources);
        assert_eq!(McpMethod::from("resources/read"), McpMethod::ReadResource);
        assert_eq!(McpMethod::from("prompts/list"), McpMethod::ListPrompts);
        assert_eq!(McpMethod::from("prompts/get"), McpMethod::GetPrompt);
        assert_eq!(McpMethod::from("notifications/initialized"), McpMethod::Notification);
        assert_eq!(McpMethod::from("notifications/progress"), McpMethod::Notification);
        assert_eq!(McpMethod::from("notifications/cancelled"), McpMethod::Notification);
        assert_eq!(McpMethod::from("unknown"), McpMethod::Unknown);
        assert_eq!(McpMethod::from(""), McpMethod::Unknown);
    }

    #[test]
    fn test_method_equality() {
        assert_eq!(McpMethod::Initialize, McpMethod::Initialize);
        assert_ne!(McpMethod::Initialize, McpMethod::ListTools);
    }

    // -- ToolDefinition tests -----------------------------------------------

    #[test]
    fn test_tool_definition_serialization() {
        let td = ToolDefinition {
            name: "read_file".to_string(),
            description: "Read a file".to_string(),
            input_schema: json!({"type": "object", "properties": {"path": {"type": "string"}}}),
        };
        let json = serde_json::to_string(&td).expect("should serialize to JSON");
        let de: ToolDefinition = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(de.name, "read_file");
        assert!(json.contains("inputSchema"));
    }

    // -- ResourceDefinition tests -------------------------------------------

    #[test]
    fn test_resource_definition_serialization() {
        let rd = ResourceDefinition {
            uri: "file:///workspace/AGENTS.md".to_string(),
            name: "AGENTS.md".to_string(),
            description: Some("Agent config".to_string()),
            mime_type: Some("text/markdown".to_string()),
        };
        let json = serde_json::to_string(&rd).expect("should serialize to JSON");
        let de: ResourceDefinition =
            serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(de.name, "AGENTS.md");
        assert!(json.contains("mimeType"));
    }

    #[test]
    fn test_resource_definition_minimal() {
        let rd = ResourceDefinition {
            uri: "file:///test".to_string(),
            name: "test".to_string(),
            description: None,
            mime_type: None,
        };
        let json = serde_json::to_string(&rd).expect("should serialize to JSON");
        assert!(!json.contains("description"));
        assert!(!json.contains("mimeType"));
    }

    // -- PromptDefinition tests ---------------------------------------------

    #[test]
    fn test_prompt_definition_serialization() {
        let pd = PromptDefinition {
            name: "summarize".to_string(),
            description: Some("Summarize content".to_string()),
            arguments: Some(vec![PromptArgument {
                name: "content".to_string(),
                description: Some("Content to summarize".to_string()),
                required: Some(true),
            }]),
        };
        let json = serde_json::to_string(&pd).expect("should serialize to JSON");
        let de: PromptDefinition = serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(de.name, "summarize");
        assert_eq!(de.arguments.expect("operation should succeed").len(), 1);
    }

    #[test]
    fn test_prompt_definition_minimal() {
        let pd = PromptDefinition {
            name: "simple".to_string(),
            description: None,
            arguments: None,
        };
        let json = serde_json::to_string(&pd).expect("should serialize to JSON");
        assert!(!json.contains("description"));
        assert!(!json.contains("arguments"));
    }
}
