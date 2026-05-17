//! Integration tests for Zeus Tools
//!
//! Tests tool execution, validation, and error handling.

use tempfile::TempDir;
use zeus_agent::ToolRegistry;

#[tokio::test]
async fn test_tool_registry_initialization() {
    let registry = ToolRegistry::with_defaults();
    let tools = registry.schemas();

    assert!(!tools.is_empty());
    assert!(tools.len() >= 5); // At least 5 core tools
}

#[tokio::test]
async fn test_tool_registry_has_core_tools() {
    let registry = ToolRegistry::with_defaults();
    let tool_names: Vec<String> = registry.schemas().iter().map(|t| t.name.clone()).collect();

    // Verify all 8 core tools are present
    assert!(tool_names.contains(&"read_file".to_string()));
    assert!(tool_names.contains(&"write_file".to_string()));
    assert!(tool_names.contains(&"edit_file".to_string()));
    assert!(tool_names.contains(&"list_dir".to_string()));
    assert!(tool_names.contains(&"shell".to_string()));
    assert!(tool_names.contains(&"web_fetch".to_string()));
    assert!(tool_names.contains(&"spawn".to_string()));
    assert!(tool_names.contains(&"message".to_string()));
}

#[tokio::test]
async fn test_list_dir_execution() {
    let temp = TempDir::new().unwrap();
    let test_dir = temp.path();

    // Create test files
    std::fs::write(test_dir.join("file1.txt"), "content1").unwrap();
    std::fs::write(test_dir.join("file2.txt"), "content2").unwrap();

    let registry = ToolRegistry::with_defaults();
    let args = serde_json::json!({
        "path": test_dir.to_string_lossy()
    });

    let result = registry.execute("list_dir", args).await;
    assert!(result.is_ok());

    let output = result.unwrap();
    assert!(output.contains("file1.txt"));
    assert!(output.contains("file2.txt"));
}

#[tokio::test]
async fn test_write_read_file_execution() {
    let temp = TempDir::new().unwrap();
    let test_file = temp.path().join("test.txt");
    let test_content = "Hello, Zeus!";

    let registry = ToolRegistry::with_defaults();

    // Write file
    let write_args = serde_json::json!({
        "path": test_file.to_string_lossy(),
        "content": test_content
    });
    let write_result = registry.execute("write_file", write_args).await;
    assert!(write_result.is_ok());

    // Read file
    let read_args = serde_json::json!({
        "path": test_file.to_string_lossy()
    });
    let read_result = registry.execute("read_file", read_args).await;
    assert!(read_result.is_ok());
    assert!(read_result.unwrap().contains(test_content));
}

#[tokio::test]
async fn test_tool_execution_with_invalid_args() {
    let registry = ToolRegistry::with_defaults();

    // Missing required path argument
    let args = serde_json::json!({});
    let result = registry.execute("read_file", args).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_tool_execution_with_nonexistent_file() {
    let registry = ToolRegistry::with_defaults();

    let args = serde_json::json!({
        "path": "/nonexistent/path/to/file.txt"
    });
    let result = registry.execute("read_file", args).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_shell_tool_execution() {
    let registry = ToolRegistry::with_defaults();

    let args = serde_json::json!({
        "command": "echo 'test output'"
    });
    let result = registry.execute("shell", args).await;
    assert!(result.is_ok());
    assert!(result.unwrap().contains("test output"));
}

#[tokio::test]
async fn test_edit_file_execution() {
    let temp = TempDir::new().unwrap();
    let test_file = temp.path().join("edit_test.txt");

    // Create initial file
    std::fs::write(&test_file, "Original content").unwrap();

    let registry = ToolRegistry::with_defaults();

    let args = serde_json::json!({
        "path": test_file.to_string_lossy(),
        "search": "Original",
        "replace": "Modified"
    });
    let result = registry.execute("edit_file", args).await;
    assert!(result.is_ok());

    // Verify edit
    let content = std::fs::read_to_string(&test_file).unwrap();
    assert!(content.contains("Modified content"));
}

#[tokio::test]
async fn test_tool_schema_validation() {
    let registry = ToolRegistry::with_defaults();
    let schemas = registry.schemas();

    for schema in schemas {
        // Each tool should have a name
        assert!(!schema.name.is_empty());

        // Each tool should have a description
        assert!(!schema.description.is_empty());

        // Parameters should be a valid JSON object
        assert!(schema.parameters.is_object() || schema.parameters.is_array());
    }
}

#[tokio::test]
async fn test_tool_registry_lookup() {
    let registry = ToolRegistry::with_defaults();
    let schemas = registry.schemas();

    // Should find existing tools
    let read_schema = schemas.iter().find(|t| t.name == "read_file");
    assert!(read_schema.is_some());

    let write_schema = schemas.iter().find(|t| t.name == "write_file");
    assert!(write_schema.is_some());
}
