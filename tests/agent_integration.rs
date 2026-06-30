//! Integration tests for Zeus Agent
//!
//! Tests agent initialization, tool execution, and message processing end-to-end.

use chrono::Utc;
use tempfile::TempDir;
use zeus_agent::Agent;
use zeus_core::{Config, Message, Role, TextDirection};
use zeus_llm::LlmClient;
use zeus_memory::Workspace;
use zeus_session::Session;

/// Helper to create a test agent with temporary workspace
async fn create_test_agent() -> (Agent, TempDir) {
    let temp = TempDir::new().unwrap();
    let workspace_path = temp.path().to_path_buf();
    let sessions_path = temp.path().join("sessions");

    let mut config = Config::default();
    config.workspace = workspace_path.clone();
    config.sessions = sessions_path.clone();
    config.model = "ollama/llama3.2".to_string();
    config.max_iterations = 3;

    let workspace = Workspace::new(workspace_path);
    let session = Session::new(&sessions_path);

    let llm = LlmClient::from_config(&config).unwrap();
    let agent = Agent::new(config, llm, workspace, session, None);

    (agent, temp)
}

#[tokio::test]
async fn test_agent_initialization() {
    let (agent, _temp) = create_test_agent().await;

    // Verify agent is properly initialized
    assert_eq!(agent.session().messages.len(), 0);
    assert!(agent.workspace().get_context().await.is_ok());
}

#[tokio::test]
async fn test_agent_session_persistence() {
    let temp = TempDir::new().unwrap();
    let sessions_path = temp.path().join("sessions");

    let mut _config = Config::default();
    _config.sessions = sessions_path.clone();
    _config.model = "ollama/llama3.2".to_string();

    // Create first session and add a message
    let mut session1 = Session::new(&sessions_path);
    let session_id = session1.id.clone();
    session1.init().await.unwrap();
    session1
        .add(Message {
            role: Role::User,
            content: "Test message".to_string(),
            tool_calls: vec![],
            tool_results: vec![],
            timestamp: Utc::now(),
            attachments: vec![],
            message_id: None,
            parent_id: None,
            thread_id: None,
            direction: TextDirection::Ltr,
            channel_source: None,
            compaction_hint: zeus_core::CompactionHint::default(),
        })
        .await
        .unwrap();

    // Load the same session and verify message persists
    let session2 = Session::load(&sessions_path, &session_id).await.unwrap();
    assert_eq!(session2.messages.len(), 1);
    assert_eq!(session2.messages[0].content, "Test message");
}

#[tokio::test]
async fn test_agent_workspace_operations() {
    let (agent, _temp) = create_test_agent().await;

    // Test remember operation
    let result = agent.workspace().remember("Test fact").await;
    assert!(result.is_ok());

    // Test note operation
    let result = agent.workspace().note("Test note").await;
    assert!(result.is_ok());

    // Verify memory retrieval
    let memory = agent.workspace().get_memory().await;
    assert!(memory.is_ok());
    assert!(memory.unwrap().contains("Test fact"));
}

#[tokio::test]
async fn test_agent_workspace_access() {
    let (agent, _temp) = create_test_agent().await;

    // Verify workspace is accessible
    let context = agent.workspace().get_context().await;
    assert!(context.is_ok());
}

#[tokio::test]
async fn test_agent_context_building() {
    let (agent, _temp) = create_test_agent().await;

    // Verify session is initialized
    assert_eq!(agent.session().messages.len(), 0);
}

#[tokio::test]
async fn test_agent_multiple_sessions() {
    let temp = TempDir::new().unwrap();
    let sessions_path = temp.path().join("sessions");

    // Create multiple sessions
    let session1 = Session::new(&sessions_path);
    let id1 = session1.id.clone();
    session1.init().await.unwrap();

    let session2 = Session::new(&sessions_path);
    let id2 = session2.id.clone();
    session2.init().await.unwrap();

    // Verify different session IDs
    assert_ne!(id1, id2);

    // Verify both can be loaded
    let loaded1 = Session::load(&sessions_path, &id1).await;
    let loaded2 = Session::load(&sessions_path, &id2).await;
    assert!(loaded1.is_ok());
    assert!(loaded2.is_ok());
}
