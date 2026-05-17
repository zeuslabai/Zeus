//! Integration tests for Agent construction, subsystem wiring, and tool execution.

use tempfile::tempdir;
use zeus_agent::{Agent, AgentEvent, ToolRegistry};
use zeus_core::{Config, Provider};
use zeus_llm::LlmClient;
use zeus_memory::Workspace;
use zeus_session::Session;

// ============================================================================
// Agent construction
// ============================================================================

#[test]
fn agent_new_creates_minimal_agent() {
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().join("workspace");
    let sessions_dir = dir.path().join("sessions");
    std::fs::create_dir_all(&workspace_dir).unwrap();
    std::fs::create_dir_all(&sessions_dir).unwrap();

    let mut config = Config::default();
    config.workspace = workspace_dir;
    config.sessions = sessions_dir.clone();
    config.model = "ollama/llama3".to_string();

    let llm = LlmClient::new(Provider::Ollama, "llama3".to_string()).unwrap();
    let workspace = Workspace::from_config(&config);
    let session = Session::new(&sessions_dir);

    let agent = Agent::new(config, llm, workspace, session, None);

    // Agent should be constructable with no subsystems
    assert_eq!(agent.running_subagents(), 0);
    assert!(agent.subagent_ids().is_empty());
}

#[test]
fn agent_session_accessible() {
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().join("workspace");
    let sessions_dir = dir.path().join("sessions");
    std::fs::create_dir_all(&workspace_dir).unwrap();
    std::fs::create_dir_all(&sessions_dir).unwrap();

    let mut config = Config::default();
    config.workspace = workspace_dir;
    config.sessions = sessions_dir.clone();
    config.model = "ollama/llama3".to_string();

    let llm = LlmClient::new(Provider::Ollama, "llama3".to_string()).unwrap();
    let workspace = Workspace::from_config(&config);
    let session = Session::new(&sessions_dir);

    let agent = Agent::new(config, llm, workspace, session, None);

    // Session and workspace should be accessible
    let _session = agent.session();
    let _workspace = agent.workspace();
}

#[test]
fn agent_with_events_channel() {
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().join("workspace");
    let sessions_dir = dir.path().join("sessions");
    std::fs::create_dir_all(&workspace_dir).unwrap();
    std::fs::create_dir_all(&sessions_dir).unwrap();

    let mut config = Config::default();
    config.workspace = workspace_dir;
    config.sessions = sessions_dir.clone();
    config.model = "ollama/llama3".to_string();

    let llm = LlmClient::new(Provider::Ollama, "llama3".to_string()).unwrap();
    let workspace = Workspace::from_config(&config);
    let session = Session::new(&sessions_dir);

    let (tx, _rx) = tokio::sync::mpsc::channel::<AgentEvent>(32);
    let agent = Agent::new(config, llm, workspace, session, None).with_events(tx);

    assert_eq!(agent.running_subagents(), 0);
}

#[tokio::test]
async fn agent_with_subsystems_no_config() {
    // with_subsystems should succeed even with no subsystem configs
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().join("workspace");
    let sessions_dir = dir.path().join("sessions");
    std::fs::create_dir_all(&workspace_dir).unwrap();
    std::fs::create_dir_all(&sessions_dir).unwrap();

    let mut config = Config::default();
    config.workspace = workspace_dir;
    config.sessions = sessions_dir.clone();
    config.model = "ollama/llama3".to_string();
    // All subsystem configs are None by default

    let llm = LlmClient::new(Provider::Ollama, "llama3".to_string()).unwrap();
    let workspace = Workspace::from_config(&config);
    let session = Session::new(&sessions_dir);

    let agent = Agent::with_subsystems(config, llm, workspace, session)
        .await
        .unwrap();

    assert_eq!(agent.running_subagents(), 0);
}

// ============================================================================
// Agent event types
// ============================================================================

#[test]
fn agent_event_variants_constructable() {
    let events = vec![
        AgentEvent::Started,
        AgentEvent::TextChunk("hello world".to_string()),
        AgentEvent::ToolCall {
            name: "read_file".to_string(),
            args: serde_json::json!({"path": "/tmp/test"}),
        },
        AgentEvent::ToolResult {
            name: "read_file".to_string(),
            success: true,
            output: "file contents".to_string(),
        },
        AgentEvent::Finished { iterations: 3 },
        AgentEvent::Error("something went wrong".to_string()),
    ];
    assert_eq!(events.len(), 6);
}

// ============================================================================
// Tool execution through agent's tool registry
// ============================================================================

#[tokio::test]
async fn agent_tool_registry_read_file() {
    let dir = tempdir().unwrap();
    let test_file = dir.path().join("hello.txt");
    std::fs::write(&test_file, "Hello, Zeus!").unwrap();

    let registry = ToolRegistry::new();
    let result = registry
        .execute(
            "read_file",
            serde_json::json!({"path": test_file.to_str().unwrap()}),
        )
        .await
        .unwrap();

    assert_eq!(result, "Hello, Zeus!");
}

#[tokio::test]
async fn agent_tool_registry_write_file() {
    let dir = tempdir().unwrap();
    let test_file = dir.path().join("output.txt");

    let registry = ToolRegistry::new();
    let result = registry
        .execute(
            "write_file",
            serde_json::json!({
                "path": test_file.to_str().unwrap(),
                "content": "Written by Zeus"
            }),
        )
        .await
        .unwrap();

    assert!(result.contains("Wrote"));
    let content = std::fs::read_to_string(&test_file).unwrap();
    assert_eq!(content, "Written by Zeus");
}

#[tokio::test]
async fn agent_tool_registry_list_dir() {
    let dir = tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "a").unwrap();
    std::fs::write(dir.path().join("b.txt"), "b").unwrap();

    let registry = ToolRegistry::new();
    let result = registry
        .execute(
            "list_dir",
            serde_json::json!({"path": dir.path().to_str().unwrap()}),
        )
        .await
        .unwrap();

    assert!(result.contains("a.txt"));
    assert!(result.contains("b.txt"));
}

#[tokio::test]
async fn agent_tool_registry_edit_file() {
    let dir = tempdir().unwrap();
    let test_file = dir.path().join("edit_me.txt");
    std::fs::write(&test_file, "Hello World").unwrap();

    let registry = ToolRegistry::new();
    let result = registry
        .execute(
            "edit_file",
            serde_json::json!({
                "path": test_file.to_str().unwrap(),
                "search": "World",
                "replace": "Zeus"
            }),
        )
        .await
        .unwrap();

    assert!(result.contains("Replaced"));
    let content = std::fs::read_to_string(&test_file).unwrap();
    assert_eq!(content, "Hello Zeus");
}

#[tokio::test]
async fn agent_tool_registry_shell() {
    let registry = ToolRegistry::new();
    let result = registry
        .execute("shell", serde_json::json!({"command": "echo hello_zeus"}))
        .await
        .unwrap();

    assert!(result.contains("hello_zeus"));
}

#[tokio::test]
async fn agent_tool_registry_unknown_tool_errors() {
    let registry = ToolRegistry::new();
    let result = registry
        .execute("nonexistent_tool", serde_json::json!({}))
        .await;

    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("Unknown tool"));
}

// ============================================================================
// Subsystem wiring (mnemosyne integration)
// ============================================================================

#[tokio::test]
async fn agent_with_mnemosyne_config() {
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().join("workspace");
    let sessions_dir = dir.path().join("sessions");
    let db_path = dir.path().join("mnemosyne.db");
    std::fs::create_dir_all(&workspace_dir).unwrap();
    std::fs::create_dir_all(&sessions_dir).unwrap();

    let mut config = Config::default();
    config.workspace = workspace_dir;
    config.sessions = sessions_dir.clone();
    config.model = "ollama/llama3".to_string();
    config.mnemosyne = Some(zeus_core::MnemosyneConfig {
        db_path,
        enable_fts: true,
        ..Default::default()
    });

    let llm = LlmClient::new(Provider::Ollama, "llama3".to_string()).unwrap();
    let workspace = Workspace::from_config(&config);
    let session = Session::new(&sessions_dir);

    // Should succeed — Mnemosyne initializes with SQLite
    let agent = Agent::with_subsystems(config, llm, workspace, session)
        .await
        .unwrap();

    assert_eq!(agent.running_subagents(), 0);
}

// ============================================================================
// Tool policy
// ============================================================================

#[test]
fn agent_set_tool_policy() {
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().join("workspace");
    let sessions_dir = dir.path().join("sessions");
    std::fs::create_dir_all(&workspace_dir).unwrap();
    std::fs::create_dir_all(&sessions_dir).unwrap();

    let mut config = Config::default();
    config.workspace = workspace_dir;
    config.sessions = sessions_dir.clone();
    config.model = "ollama/llama3".to_string();

    let llm = LlmClient::new(Provider::Ollama, "llama3".to_string()).unwrap();
    let workspace = Workspace::from_config(&config);
    let session = Session::new(&sessions_dir);

    let mut agent = Agent::new(config, llm, workspace, session, None);

    // Set a tool policy (empty allow/deny = allow all)
    agent.set_tool_policy(zeus_core::AgentToolPolicy::default());
}

// ============================================================================
// Goals context injection
// ============================================================================

#[test]
fn agent_set_goals_context() {
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().join("workspace");
    let sessions_dir = dir.path().join("sessions");
    std::fs::create_dir_all(&workspace_dir).unwrap();
    std::fs::create_dir_all(&sessions_dir).unwrap();

    let mut config = Config::default();
    config.workspace = workspace_dir;
    config.sessions = sessions_dir.clone();
    config.model = "ollama/llama3".to_string();

    let llm = LlmClient::new(Provider::Ollama, "llama3".to_string()).unwrap();
    let workspace = Workspace::from_config(&config);
    let session = Session::new(&sessions_dir);

    let mut agent = Agent::new(config, llm, workspace, session, None);

    // Set goals context (used in system prompt)
    agent.set_goals_context(Some("Goal 1: Complete integration tests".to_string()));
    agent.set_goals_context(None); // Clear it
}

// ============================================================================
// Memory persistence verification (c606c617)
// ============================================================================

#[tokio::test]
async fn memory_md_content_appears_in_workspace_reload() {
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().join("workspace");
    let memory_dir = workspace_dir.join("memory");
    std::fs::create_dir_all(&memory_dir).unwrap();

    // Write a known fact into MEMORY.md
    std::fs::write(
        memory_dir.join("MEMORY.md"),
        "# Long-term Memory\n\n- [test] The answer is 42.\n",
    )
    .unwrap();

    let mut config = zeus_core::Config::default();
    config.workspace = workspace_dir.clone();
    config.sessions = dir.path().join("sessions");
    std::fs::create_dir_all(&config.sessions).unwrap();

    let workspace = Workspace::from_config(&config);

    // get_memory() should return the content we wrote
    let content = workspace.get_memory().await.unwrap();
    assert!(
        content.contains("The answer is 42"),
        "MEMORY.md content not returned by get_memory(): got {:?}",
        content
    );
}

#[tokio::test]
async fn memory_md_empty_when_file_missing() {
    let dir = tempdir().unwrap();
    let workspace_dir = dir.path().join("workspace");
    std::fs::create_dir_all(&workspace_dir).unwrap();

    let mut config = zeus_core::Config::default();
    config.workspace = workspace_dir.clone();
    config.sessions = dir.path().join("sessions");
    std::fs::create_dir_all(&config.sessions).unwrap();

    let workspace = Workspace::from_config(&config);

    // Should return empty/error gracefully — no panic
    let content = workspace.get_memory().await.unwrap_or_default();
    assert!(
        content.is_empty(),
        "Expected empty string when MEMORY.md absent, got: {:?}",
        content
    );
}
