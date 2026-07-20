//! MCP Tool Bridge - Model Context Protocol Tool Integration
//!
//! Provides a bridge to MCP (Model Context Protocol) tools.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use zeus_core::Result;

/// MCP tool definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolDef {
    /// Tool name
    pub name: String,
    /// Tool description
    pub description: String,
    /// JSON schema for parameters
    pub parameters: Value,
    /// Optional metadata
    #[serde(default)]
    pub metadata: Option<Value>,
}

/// MCP tool execution result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolResult {
    pub success: bool,
    pub result: Option<Value>,
    pub error: Option<String>,
}

/// Bridge to MCP tools
pub struct McpBridge {
    tools: Vec<McpToolDef>,
    server_url: Option<String>,
    /// Shared HTTP client (connection pooling)
    client: reqwest::Client,
}

impl McpBridge {
    /// Create a new MCP bridge
    pub fn new() -> Self {
        Self {
            tools: Vec::new(),
            server_url: None,
            client: reqwest::Client::new(),
        }
    }

    /// Create with server URL
    pub fn with_server(server_url: String) -> Self {
        Self {
            tools: Vec::new(),
            server_url: Some(server_url),
            client: reqwest::Client::new(),
        }
    }

    /// List available tools
    pub fn list_tools(&self) -> Vec<McpToolDef> {
        self.tools.clone()
    }

    /// Register a tool
    pub fn register_tool(&mut self, tool: McpToolDef) {
        self.tools.push(tool);
    }

    /// Execute a tool by name
    pub async fn execute_tool(&self, name: &str, args: Value) -> Result<McpToolResult> {
        let tool = self
            .tools
            .iter()
            .find(|t| t.name == name)
            .ok_or_else(|| zeus_core::Error::Tool(format!("Tool not found: {}", name)))?;

        // Validate args against schema
        if let Err(e) = self.validate_args(&tool.parameters, &args) {
            return Ok(McpToolResult {
                success: false,
                result: None,
                error: Some(e),
            });
        }

        // Execute tool via MCP server
        if let Some(ref server_url) = self.server_url {
            self.execute_remote(server_url, name, args).await
        } else {
            // No server configured — cannot execute locally
            Ok(McpToolResult {
                success: false,
                result: None,
                error: Some(format!(
                    "No MCP server configured. Tool '{}' requires a remote server_url to execute.",
                    name
                )),
            })
        }
    }

    /// Execute tool on remote MCP server
    async fn execute_remote(
        &self,
        server_url: &str,
        tool_name: &str,
        args: Value,
    ) -> Result<McpToolResult> {
        let client = &self.client;
        let url = format!("{}/execute", server_url);

        let request = serde_json::json!({
            "tool": tool_name,
            "arguments": args
        });

        let response = client
            .post(&url)
            .json(&request)
            .send()
            .await
            .map_err(|e| zeus_core::Error::Tool(format!("MCP request failed: {}", e)))?;

        if !response.status().is_success() {
            return Ok(McpToolResult {
                success: false,
                result: None,
                error: Some(format!("Server returned: {}", response.status())),
            });
        }

        let result = response
            .json()
            .await
            .map_err(|e| zeus_core::Error::Tool(format!("Failed to parse response: {}", e)))?;

        Ok(result)
    }

    /// Validate arguments against a JSON Schema–style parameter definition.
    ///
    /// Checks:
    /// - `args` is an object (or null)
    /// - All `required` properties are present
    /// - Each supplied property matches its declared `type`
    /// - No unknown properties when `additionalProperties` is `false`
    fn validate_args(&self, schema: &Value, args: &Value) -> std::result::Result<(), String> {
        // Null args are treated as empty object
        if args.is_null() {
            // If schema has required fields, null is invalid
            if let Some(required) = schema.get("required").and_then(|v| v.as_array())
                && !required.is_empty()
            {
                return Err(format!(
                    "Missing required fields: {}",
                    required
                        .iter()
                        .filter_map(|v| v.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            return Ok(());
        }

        let obj = args
            .as_object()
            .ok_or_else(|| "Arguments must be an object".to_string())?;

        let properties = schema.get("properties").and_then(|v| v.as_object());

        // Check required fields
        if let Some(required) = schema.get("required").and_then(|v| v.as_array()) {
            for req in required {
                if let Some(key) = req.as_str()
                    && !obj.contains_key(key)
                {
                    return Err(format!("Missing required field: {key}"));
                }
            }
        }

        // Check additionalProperties
        if let Some(props) = properties
            && schema.get("additionalProperties") == Some(&Value::Bool(false))
        {
            for key in obj.keys() {
                if !props.contains_key(key) {
                    return Err(format!("Unknown property: {key}"));
                }
            }
        }

        // Validate types of supplied properties
        if let Some(props) = properties {
            for (key, value) in obj {
                if let Some(prop_schema) = props.get(key)
                    && let Some(expected_type) = prop_schema.get("type").and_then(|v| v.as_str())
                    && !value_matches_type(value, expected_type)
                {
                    return Err(format!(
                        "Property '{key}' expected type '{expected_type}', got {}",
                        json_type_name(value)
                    ));
                }
            }
        }

        Ok(())
    }

    /// Get tool by name
    pub fn get_tool(&self, name: &str) -> Option<&McpToolDef> {
        self.tools.iter().find(|t| t.name == name)
    }

    /// Remove a tool
    pub fn remove_tool(&mut self, name: &str) -> bool {
        if let Some(pos) = self.tools.iter().position(|t| t.name == name) {
            self.tools.remove(pos);
            true
        } else {
            false
        }
    }

    /// Clear all tools
    pub fn clear_tools(&mut self) {
        self.tools.clear();
    }

    /// Get number of registered tools
    pub fn tool_count(&self) -> usize {
        self.tools.len()
    }
}

/// Check if a serde_json::Value matches a JSON Schema type keyword.
fn value_matches_type(value: &Value, expected: &str) -> bool {
    match expected {
        "string" => value.is_string(),
        "number" => value.is_number(),
        "integer" => value.is_i64() || value.is_u64(),
        "boolean" => value.is_boolean(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        "null" => value.is_null(),
        _ => true, // Unknown type keyword — accept
    }
}

/// Human-readable type name for a JSON value.
fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

impl Default for McpBridge {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_tool() -> McpToolDef {
        McpToolDef {
            name: "test_tool".to_string(),
            description: "A test tool".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "arg1": {"type": "string"},
                    "arg2": {"type": "number"}
                }
            }),
            metadata: None,
        }
    }

    #[test]
    fn test_bridge_creation() {
        let bridge = McpBridge::new();
        assert_eq!(bridge.tool_count(), 0);
    }

    #[test]
    fn test_register_tool() {
        let mut bridge = McpBridge::new();
        let tool = create_test_tool();

        bridge.register_tool(tool.clone());

        assert_eq!(bridge.tool_count(), 1);
        assert_eq!(bridge.list_tools()[0].name, "test_tool");
    }

    #[test]
    fn test_get_tool() {
        let mut bridge = McpBridge::new();
        bridge.register_tool(create_test_tool());

        let tool = bridge.get_tool("test_tool");
        assert!(tool.is_some());
        assert_eq!(tool.expect("Failed to unwrap test tool").name, "test_tool");

        let missing = bridge.get_tool("nonexistent");
        assert!(missing.is_none());
    }

    #[test]
    fn test_remove_tool() {
        let mut bridge = McpBridge::new();
        bridge.register_tool(create_test_tool());

        assert_eq!(bridge.tool_count(), 1);

        let removed = bridge.remove_tool("test_tool");
        assert!(removed);
        assert_eq!(bridge.tool_count(), 0);

        let not_removed = bridge.remove_tool("nonexistent");
        assert!(!not_removed);
    }

    #[test]
    fn test_clear_tools() {
        let mut bridge = McpBridge::new();
        bridge.register_tool(create_test_tool());
        bridge.register_tool(create_test_tool());

        assert_eq!(bridge.tool_count(), 2);

        bridge.clear_tools();
        assert_eq!(bridge.tool_count(), 0);
    }

    #[tokio::test]
    async fn test_execute_tool_not_found() {
        let bridge = McpBridge::new();

        let result = bridge
            .execute_tool("nonexistent", serde_json::json!({}))
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_execute_tool_no_server() {
        let mut bridge = McpBridge::new();
        bridge.register_tool(create_test_tool());

        let result = bridge
            .execute_tool(
                "test_tool",
                serde_json::json!({
                    "arg1": "value",
                    "arg2": 42
                }),
            )
            .await;

        assert!(result.is_ok());
        let tool_result = result.expect("operation should return result");
        assert!(!tool_result.success, "should fail without server_url");
        assert!(tool_result.error.is_some());
        assert!(
            tool_result
                .error
                .as_ref()
                .unwrap()
                .contains("No MCP server configured"),
            "error should explain missing server"
        );
    }

    #[test]
    fn test_validate_args_valid() {
        let bridge = McpBridge::new();
        let schema = serde_json::json!({});
        let args = serde_json::json!({"key": "value"});

        let result = bridge.validate_args(&schema, &args);
        assert!(result.is_ok());
    }

    // ── Schema validation tests ──────────────────────────────────────

    #[test]
    fn test_validate_args_null_no_required() {
        let bridge = McpBridge::new();
        let schema = serde_json::json!({"type": "object", "properties": {}});
        assert!(bridge.validate_args(&schema, &Value::Null).is_ok());
    }

    #[test]
    fn test_validate_args_null_with_required() {
        let bridge = McpBridge::new();
        let schema = serde_json::json!({
            "type": "object",
            "required": ["name"],
            "properties": {"name": {"type": "string"}}
        });
        let err = bridge.validate_args(&schema, &Value::Null).unwrap_err();
        assert!(
            err.contains("name"),
            "error should mention the field: {err}"
        );
    }

    #[test]
    fn test_validate_args_not_object() {
        let bridge = McpBridge::new();
        let schema = serde_json::json!({});
        assert!(
            bridge
                .validate_args(&schema, &serde_json::json!("just a string"))
                .is_err()
        );
        assert!(
            bridge
                .validate_args(&schema, &serde_json::json!(42))
                .is_err()
        );
        assert!(
            bridge
                .validate_args(&schema, &serde_json::json!([1, 2]))
                .is_err()
        );
    }

    #[test]
    fn test_validate_required_fields_present() {
        let bridge = McpBridge::new();
        let schema = serde_json::json!({
            "type": "object",
            "required": ["a", "b"],
            "properties": {
                "a": {"type": "string"},
                "b": {"type": "number"}
            }
        });
        let args = serde_json::json!({"a": "hello", "b": 42});
        assert!(bridge.validate_args(&schema, &args).is_ok());
    }

    #[test]
    fn test_validate_required_field_missing() {
        let bridge = McpBridge::new();
        let schema = serde_json::json!({
            "type": "object",
            "required": ["a", "b"],
            "properties": {
                "a": {"type": "string"},
                "b": {"type": "number"}
            }
        });
        let args = serde_json::json!({"a": "hello"});
        let err = bridge.validate_args(&schema, &args).unwrap_err();
        assert!(err.contains("b"), "should mention missing field 'b': {err}");
    }

    #[test]
    fn test_validate_type_string() {
        let bridge = McpBridge::new();
        let schema = serde_json::json!({
            "type": "object",
            "properties": {"name": {"type": "string"}}
        });
        assert!(
            bridge
                .validate_args(&schema, &serde_json::json!({"name": "Zeus"}))
                .is_ok()
        );
        let err = bridge
            .validate_args(&schema, &serde_json::json!({"name": 123}))
            .unwrap_err();
        assert!(
            err.contains("string"),
            "should mention expected type: {err}"
        );
    }

    #[test]
    fn test_validate_type_number() {
        let bridge = McpBridge::new();
        let schema = serde_json::json!({
            "type": "object",
            "properties": {"count": {"type": "number"}}
        });
        assert!(
            bridge
                .validate_args(&schema, &serde_json::json!({"count": std::f64::consts::PI}))
                .is_ok()
        );
        assert!(
            bridge
                .validate_args(&schema, &serde_json::json!({"count": 42}))
                .is_ok()
        );
        assert!(
            bridge
                .validate_args(&schema, &serde_json::json!({"count": "not a number"}))
                .is_err()
        );
    }

    #[test]
    fn test_validate_type_integer() {
        let bridge = McpBridge::new();
        let schema = serde_json::json!({
            "type": "object",
            "properties": {"n": {"type": "integer"}}
        });
        assert!(
            bridge
                .validate_args(&schema, &serde_json::json!({"n": 42}))
                .is_ok()
        );
        // Floats with fractional parts are stored as f64 in serde_json,
        // which is_i64()/is_u64() returns false for.
        assert!(
            bridge
                .validate_args(&schema, &serde_json::json!({"n": std::f64::consts::PI}))
                .is_err()
        );
    }

    #[test]
    fn test_validate_type_boolean() {
        let bridge = McpBridge::new();
        let schema = serde_json::json!({
            "type": "object",
            "properties": {"flag": {"type": "boolean"}}
        });
        assert!(
            bridge
                .validate_args(&schema, &serde_json::json!({"flag": true}))
                .is_ok()
        );
        assert!(
            bridge
                .validate_args(&schema, &serde_json::json!({"flag": "yes"}))
                .is_err()
        );
    }

    #[test]
    fn test_validate_type_array() {
        let bridge = McpBridge::new();
        let schema = serde_json::json!({
            "type": "object",
            "properties": {"items": {"type": "array"}}
        });
        assert!(
            bridge
                .validate_args(&schema, &serde_json::json!({"items": [1, 2, 3]}))
                .is_ok()
        );
        assert!(
            bridge
                .validate_args(&schema, &serde_json::json!({"items": "not array"}))
                .is_err()
        );
    }

    #[test]
    fn test_validate_type_object() {
        let bridge = McpBridge::new();
        let schema = serde_json::json!({
            "type": "object",
            "properties": {"meta": {"type": "object"}}
        });
        assert!(
            bridge
                .validate_args(&schema, &serde_json::json!({"meta": {"k": "v"}}))
                .is_ok()
        );
        assert!(
            bridge
                .validate_args(&schema, &serde_json::json!({"meta": 42}))
                .is_err()
        );
    }

    #[test]
    fn test_validate_additional_properties_false() {
        let bridge = McpBridge::new();
        let schema = serde_json::json!({
            "type": "object",
            "properties": {"a": {"type": "string"}},
            "additionalProperties": false
        });
        assert!(
            bridge
                .validate_args(&schema, &serde_json::json!({"a": "ok"}))
                .is_ok()
        );
        let err = bridge
            .validate_args(&schema, &serde_json::json!({"a": "ok", "b": "extra"}))
            .unwrap_err();
        assert!(err.contains("Unknown property"), "{err}");
        assert!(err.contains("b"), "{err}");
    }

    #[test]
    fn test_validate_additional_properties_allowed() {
        let bridge = McpBridge::new();
        let schema = serde_json::json!({
            "type": "object",
            "properties": {"a": {"type": "string"}},
            "additionalProperties": true
        });
        // Extra properties should be accepted
        assert!(
            bridge
                .validate_args(&schema, &serde_json::json!({"a": "ok", "extra": 99}))
                .is_ok()
        );
    }

    #[test]
    fn test_validate_no_properties_section() {
        let bridge = McpBridge::new();
        // Schema with no properties defined — any object should pass
        let schema = serde_json::json!({"type": "object"});
        assert!(
            bridge
                .validate_args(&schema, &serde_json::json!({"anything": "goes"}))
                .is_ok()
        );
    }

    #[test]
    fn test_validate_unknown_type_keyword_accepted() {
        let bridge = McpBridge::new();
        let schema = serde_json::json!({
            "type": "object",
            "properties": {"x": {"type": "custom_type"}}
        });
        // Unknown type keyword should not reject the value
        assert!(
            bridge
                .validate_args(&schema, &serde_json::json!({"x": "anything"}))
                .is_ok()
        );
    }

    #[test]
    fn test_value_matches_type_null() {
        assert!(value_matches_type(&Value::Null, "null"));
        assert!(!value_matches_type(&serde_json::json!(""), "null"));
    }

    #[test]
    fn test_json_type_name_coverage() {
        assert_eq!(json_type_name(&Value::Null), "null");
        assert_eq!(json_type_name(&serde_json::json!(true)), "boolean");
        assert_eq!(json_type_name(&serde_json::json!(1)), "number");
        assert_eq!(json_type_name(&serde_json::json!("s")), "string");
        assert_eq!(json_type_name(&serde_json::json!([1])), "array");
        assert_eq!(json_type_name(&serde_json::json!({"a": 1})), "object");
    }
}
