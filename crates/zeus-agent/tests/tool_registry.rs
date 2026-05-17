//! Integration tests for the tool registry: schemas, execution, and error handling.

use zeus_agent::ToolRegistry;
use zeus_core::ToolSchema;

// ============================================================================
// Tool schema completeness
// ============================================================================

#[test]
fn core_schemas_returns_22_tools() {
    let registry = ToolRegistry::new();
    let schemas = registry.schemas();

    // 22 core tools — includes collect_spawns, send_file, trigger tools, and recent additions
    assert_eq!(schemas.len(), 22);
}

#[test]
fn all_schemas_have_required_fields() {
    let registry = ToolRegistry::new();
    for schema in registry.schemas() {
        assert!(!schema.name.is_empty(), "Tool name must not be empty");
        assert!(
            !schema.description.is_empty(),
            "Tool '{}' has empty description",
            schema.name
        );
        assert!(
            schema.parameters.is_object(),
            "Tool '{}' parameters must be an object",
            schema.name
        );
        assert_eq!(
            schema.parameters["type"], "object",
            "Tool '{}' parameters type must be 'object'",
            schema.name
        );
        assert!(
            schema.parameters["properties"].is_object(),
            "Tool '{}' must have properties",
            schema.name
        );
    }
}

#[test]
fn expected_tool_names_present() {
    let registry = ToolRegistry::new();
    let schemas = registry.schemas();
    let names: Vec<&str> = schemas.iter().map(|s| s.name.as_str()).collect();

    let expected = [
        "read_file",
        "write_file",
        "edit_file",
        "list_dir",
        "shell",
        "web_fetch",
        "spawn",
        "message",
        "link_understanding",
        "media_understanding",
        "auto_reply",
        "polls",
        "gmail_pubsub",
        "apply_patch",
        "send_file",
    ];

    for tool_name in &expected {
        assert!(
            names.contains(tool_name),
            "Missing expected tool: {}",
            tool_name
        );
    }
}

#[test]
fn tool_schemas_have_required_params() {
    let registry = ToolRegistry::new();
    let schemas = registry.schemas();

    // read_file requires "path"
    let read_file = schemas.iter().find(|s| s.name == "read_file").unwrap();
    let required = read_file.parameters["required"].as_array().unwrap();
    assert!(required.iter().any(|v| v == "path"));

    // write_file requires "path" and "content"
    let write_file = schemas.iter().find(|s| s.name == "write_file").unwrap();
    let required = write_file.parameters["required"].as_array().unwrap();
    assert!(required.iter().any(|v| v == "path"));
    assert!(required.iter().any(|v| v == "content"));

    // shell requires "command"
    let shell = schemas.iter().find(|s| s.name == "shell").unwrap();
    let required = shell.parameters["required"].as_array().unwrap();
    assert!(required.iter().any(|v| v == "command"));

    // message requires "channel" and "content"
    let message = schemas.iter().find(|s| s.name == "message").unwrap();
    let required = message.parameters["required"].as_array().unwrap();
    assert!(required.iter().any(|v| v == "channel"));
    assert!(required.iter().any(|v| v == "content"));
}

#[test]
fn tool_schemas_json_roundtrip() {
    let registry = ToolRegistry::new();
    for schema in registry.schemas() {
        let json = serde_json::to_string(&schema).unwrap();
        let parsed: ToolSchema = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, schema.name);
        assert_eq!(parsed.description, schema.description);
    }
}

// ============================================================================
// Tool execution — happy paths
// ============================================================================

#[tokio::test]
async fn execute_read_file_success() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.txt");
    std::fs::write(&path, "test content").unwrap();

    let registry = ToolRegistry::new();
    let result = registry
        .execute(
            "read_file",
            serde_json::json!({"path": path.to_str().unwrap()}),
        )
        .await
        .unwrap();

    assert_eq!(result, "test content");
}

#[tokio::test]
async fn execute_write_then_read() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("roundtrip.txt");

    let registry = ToolRegistry::new();

    // Write
    registry
        .execute(
            "write_file",
            serde_json::json!({
                "path": path.to_str().unwrap(),
                "content": "roundtrip data"
            }),
        )
        .await
        .unwrap();

    // Read back
    let content = registry
        .execute(
            "read_file",
            serde_json::json!({"path": path.to_str().unwrap()}),
        )
        .await
        .unwrap();

    assert_eq!(content, "roundtrip data");
}

#[tokio::test]
async fn execute_edit_file_replaces_text() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("edit.txt");
    std::fs::write(&path, "foo bar baz").unwrap();

    let registry = ToolRegistry::new();
    registry
        .execute(
            "edit_file",
            serde_json::json!({
                "path": path.to_str().unwrap(),
                "search": "bar",
                "replace": "qux"
            }),
        )
        .await
        .unwrap();

    let content = std::fs::read_to_string(&path).unwrap();
    assert_eq!(content, "foo qux baz");
}

#[tokio::test]
async fn execute_edit_file_replace_all() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("edit_all.txt");
    std::fs::write(&path, "aaa bbb aaa").unwrap();

    let registry = ToolRegistry::new();
    registry
        .execute(
            "edit_file",
            serde_json::json!({
                "path": path.to_str().unwrap(),
                "search": "aaa",
                "replace": "ccc",
                "all": true
            }),
        )
        .await
        .unwrap();

    let content = std::fs::read_to_string(&path).unwrap();
    assert_eq!(content, "ccc bbb ccc");
}

#[tokio::test]
async fn execute_list_dir_shows_entries() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("alpha.txt"), "").unwrap();
    std::fs::create_dir(dir.path().join("subdir")).unwrap();

    let registry = ToolRegistry::new();
    let result = registry
        .execute(
            "list_dir",
            serde_json::json!({"path": dir.path().to_str().unwrap()}),
        )
        .await
        .unwrap();

    assert!(result.contains("alpha.txt"));
    assert!(result.contains("subdir"));
}

#[tokio::test]
async fn execute_shell_captures_output() {
    let registry = ToolRegistry::new();
    let result = registry
        .execute(
            "shell",
            serde_json::json!({"command": "printf 'zeus_test_123'"}),
        )
        .await
        .unwrap();

    assert!(result.contains("zeus_test_123"));
}

#[tokio::test]
async fn execute_shell_with_cwd() {
    let dir = tempfile::tempdir().unwrap();

    let registry = ToolRegistry::new();
    let result = registry
        .execute(
            "shell",
            serde_json::json!({
                "command": "pwd",
                "cwd": dir.path().to_str().unwrap()
            }),
        )
        .await
        .unwrap();

    // The output should contain the temp dir path
    assert!(result.contains(dir.path().to_str().unwrap()));
}

// ============================================================================
// Tool execution — error handling
// ============================================================================

#[tokio::test]
async fn execute_unknown_tool_returns_error() {
    let registry = ToolRegistry::new();
    let result = registry
        .execute("does_not_exist", serde_json::json!({}))
        .await;

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Unknown tool"));
}

#[tokio::test]
async fn execute_read_file_missing_path_arg() {
    let registry = ToolRegistry::new();
    let result = registry.execute("read_file", serde_json::json!({})).await;

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("path"));
}

#[tokio::test]
async fn execute_read_file_nonexistent() {
    let registry = ToolRegistry::new();
    let result = registry
        .execute(
            "read_file",
            serde_json::json!({"path": "/tmp/zeus_nonexistent_file_12345.txt"}),
        )
        .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn execute_write_file_missing_content() {
    let registry = ToolRegistry::new();
    let result = registry
        .execute(
            "write_file",
            serde_json::json!({"path": "/tmp/zeus_test_write.txt"}),
        )
        .await;

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("content"));
}

#[tokio::test]
async fn execute_edit_file_missing_search() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("edit_err.txt");
    std::fs::write(&path, "hello").unwrap();

    let registry = ToolRegistry::new();
    let result = registry
        .execute(
            "edit_file",
            serde_json::json!({
                "path": path.to_str().unwrap(),
                "replace": "world"
            }),
        )
        .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn execute_shell_failing_command() {
    let registry = ToolRegistry::new();
    let result = registry
        .execute("shell", serde_json::json!({"command": "false"}))
        .await;

    // `false` exits with code 1 — the tool may still return Ok with exit code info
    // or may return an error. Either way, it shouldn't panic.
    let _ = result;
}

// ============================================================================
// Default / with_defaults equivalence
// ============================================================================

#[test]
fn default_and_with_defaults_equivalent() {
    let r1 = ToolRegistry::new();
    let r2 = ToolRegistry::with_defaults();
    let r3 = ToolRegistry::default();

    assert_eq!(r1.schemas().len(), r2.schemas().len());
    assert_eq!(r2.schemas().len(), r3.schemas().len());
}

// ============================================================================
// execute_tool helper function
// ============================================================================

#[tokio::test]
async fn execute_tool_helper_works() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("helper_test.txt");
    std::fs::write(&path, "helper content").unwrap();

    let result = zeus_agent::execute_tool(
        "read_file",
        serde_json::json!({"path": path.to_str().unwrap()}),
    )
    .await
    .unwrap();

    assert_eq!(result, "helper content");
}

// ============================================================================
// Large file truncation
// ============================================================================

#[tokio::test]
async fn execute_read_file_large_truncates() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("large.txt");
    let large_content = "x".repeat(200_000);
    std::fs::write(&path, &large_content).unwrap();

    let registry = ToolRegistry::new();
    let result = registry
        .execute(
            "read_file",
            serde_json::json!({"path": path.to_str().unwrap()}),
        )
        .await
        .unwrap();

    // Should be truncated to ~100KB + truncation notice
    assert!(result.len() < 200_000);
    assert!(result.contains("truncated"));
}

// ============================================================================
// Write file creates parent directories
// ============================================================================

#[tokio::test]
async fn execute_write_file_creates_parents() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a").join("b").join("c").join("deep.txt");

    let registry = ToolRegistry::new();
    registry
        .execute(
            "write_file",
            serde_json::json!({
                "path": path.to_str().unwrap(),
                "content": "deep content"
            }),
        )
        .await
        .unwrap();

    assert!(path.exists());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "deep content");
}
