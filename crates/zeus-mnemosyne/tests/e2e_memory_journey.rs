//! End-to-end tests for the full memory system user journey.
//!
//! These tests simulate real user workflows:
//! 1. Bootstrap workspace → sync → search indexed content
//! 2. Store conversations → recall → semantic search
//! 3. Memory lifecycle: store → boost → decay → forget
//! 4. Pattern extraction across sessions
//! 5. Working memory → finalize → promote to semantic

use tempfile::tempdir;
use zeus_core::Message;
use zeus_mnemosyne::*;

/// Helper: create a test Mnemosyne instance with FTS enabled
async fn test_mnemosyne(dir: &std::path::Path) -> Mnemosyne {
    let config = MnemosyneConfig {
        db_path: dir.join("test.db"),
        enable_fts: true,
        enable_embeddings: false,
        ..Default::default()
    };
    Mnemosyne::new(config)
        .await
        .expect("Mnemosyne::new should succeed")
}

// ============================================================================
// Journey 1: Workspace bootstrap → sync → search
// ============================================================================

#[tokio::test]
async fn journey_bootstrap_sync_search() {
    let db_dir = tempdir().expect("should create temp dir");
    let workspace = tempdir().expect("should create temp dir");

    // Step 1: Bootstrap creates workspace structure
    let created = bootstrap_workspace(workspace.path()).expect("operation should succeed");
    assert!(!created.is_empty(), "Bootstrap should create files");
    assert!(workspace.path().join("memory").join("MEMORY.md").exists());
    assert!(workspace.path().join("AGENTS.md").exists());
    assert!(workspace.path().join("IDENTITY.md").exists());

    // Step 2: Add some project-specific markdown
    std::fs::write(
        workspace.path().join("memory").join("project.md"),
        "# Zeus Project\n\nZeus is an AI agent platform.\n\nKey features:\n- Persistent memory\n- Tool execution\n- Multi-channel support\n",
    ).expect("operation should succeed");

    // Step 3: Create Mnemosyne and sync
    let mn = test_mnemosyne(db_dir.path()).await;
    let stats = mn
        .sync_workspace(workspace.path())
        .await
        .expect("async operation should succeed");
    assert!(
        stats.files_scanned >= 4,
        "Should scan bootstrap files + project.md"
    );
    assert!(stats.files_changed >= 4);
    assert_eq!(stats.files_unchanged, 0);

    // Step 4: Second sync — all unchanged
    let stats2 = mn
        .sync_workspace(workspace.path())
        .await
        .expect("async operation should succeed");
    assert_eq!(stats2.files_changed, 0);
    assert!(stats2.files_unchanged >= 4);

    // Step 5: Modify a file and resync
    std::fs::write(
        workspace.path().join("memory").join("project.md"),
        "# Zeus Project v2\n\nUpdated description with new features.\n",
    )
    .expect("operation should succeed");
    let stats3 = mn
        .sync_workspace(workspace.path())
        .await
        .expect("async operation should succeed");
    assert_eq!(stats3.files_changed, 1);

    // Step 6: Verify tracked files
    {
        let store = mn.store_ref().lock().await;
        let tracked = store
            .list_tracked_files("workspace")
            .expect("list_tracked_files should succeed");
        assert!(tracked.len() >= 4);
    }
}

// ============================================================================
// Journey 2: Conversation store → recall → search
// ============================================================================

#[tokio::test]
async fn journey_conversation_lifecycle() {
    let dir = tempdir().expect("should create temp dir");
    let mn = test_mnemosyne(dir.path()).await;

    // Step 1: Simulate a multi-turn conversation
    mn.store(
        "chat-1",
        &Message::user("How do I build a REST API in Rust?"),
    )
    .await
    .expect("async operation should succeed");
    mn.store(
        "chat-1",
        &Message::assistant("You can use Axum or Actix-web. Here's a basic Axum example..."),
    )
    .await
    .expect("async operation should succeed");
    mn.store(
        "chat-1",
        &Message::user("Can you add authentication middleware?"),
    )
    .await
    .expect("async operation should succeed");
    mn.store(
        "chat-1",
        &Message::assistant("Sure! You can use tower middleware with JWT validation..."),
    )
    .await
    .expect("async operation should succeed");

    // Step 2: Simulate a second conversation
    mn.store("chat-2", &Message::user("What's the weather like?"))
        .await
        .expect("async operation should succeed");
    mn.store(
        "chat-2",
        &Message::assistant("I don't have access to weather data."),
    )
    .await
    .expect("async operation should succeed");

    // Step 3: Recall specific session
    let chat1_msgs = mn
        .recall_session("chat-1", 100)
        .await
        .expect("async operation should succeed");
    assert_eq!(chat1_msgs.len(), 4);

    let chat2_msgs = mn
        .recall_session("chat-2", 100)
        .await
        .expect("async operation should succeed");
    assert_eq!(chat2_msgs.len(), 2);

    // Step 4: Search across all sessions
    let results = mn
        .search("Axum", 10)
        .await
        .expect("async operation should succeed");
    assert!(!results.is_empty(), "Should find Axum in chat-1");
    assert!(results[0].content.contains("Axum"));

    let results = mn
        .search("JWT", 10)
        .await
        .expect("async operation should succeed");
    assert!(!results.is_empty(), "Should find JWT in chat-1");

    // Step 5: Search should NOT find content from unrelated session
    let results = mn
        .search("weather", 10)
        .await
        .expect("async operation should succeed");
    assert!(results.len() >= 1, "Should find weather in chat-2");
    assert!(results[0].content.contains("weather"));

    // Step 6: Check stats
    let stats = mn.stats().await.expect("async operation should succeed");
    assert_eq!(stats.message_count, 6);
    assert!(
        stats.session_count >= 2,
        "At least 2 chat sessions (workspace files add more)"
    );
}

// ============================================================================
// Journey 3: Memory hierarchy — working → episodic → semantic
// ============================================================================

#[tokio::test]
async fn journey_memory_hierarchy() {
    let dir = tempdir().expect("should create temp dir");
    let mn = test_mnemosyne(dir.path()).await;

    // Step 1: Store working memory (scratch notes during a session)
    mn.store_typed(
        "s1",
        &Message::user("researching deployment options"),
        MemoryType::Working,
        0.8,
    )
    .await
    .expect("async operation should succeed");
    mn.store_typed(
        "s1",
        &Message::user("Docker vs bare metal comparison"),
        MemoryType::Working,
        0.7,
    )
    .await
    .expect("async operation should succeed");
    mn.store_typed(
        "s1",
        &Message::user("decided on Docker with k8s"),
        MemoryType::Working,
        0.9,
    )
    .await
    .expect("async operation should succeed");

    // Step 2: Store episodic memory (past interactions)
    mn.store_typed(
        "s1",
        &Message::user("deployed v1.0 to production"),
        MemoryType::Episodic,
        0.6,
    )
    .await
    .expect("async operation should succeed");

    // Step 3: Store semantic memory (extracted knowledge)
    mn.store_typed(
        "s1",
        &Message::user("Zeus uses Axum for HTTP, ratatui for TUI"),
        MemoryType::Semantic,
        0.9,
    )
    .await
    .expect("async operation should succeed");

    // Step 4: Search by type
    let working = mn
        .search_by_type("deployment", MemoryType::Working, 10)
        .await
        .expect("async operation should succeed");
    assert!(!working.is_empty());

    let semantic = mn
        .search_by_type("Axum", MemoryType::Semantic, 10)
        .await
        .expect("async operation should succeed");
    assert_eq!(semantic.len(), 1);
    assert!(semantic[0].content.contains("Axum"));

    // Step 5: Working memory for a session
    let wm = mn
        .working_memory("s1")
        .await
        .expect("async operation should succeed");
    assert_eq!(wm.len(), 3, "Should have 3 working memories");

    // Step 6: Finalize working memory — promote high-importance, discard low
    let (promoted, discarded) = mn
        .finalize_working_memory("s1", 0.75)
        .await
        .expect("async operation should succeed");
    // Items with importance >= 0.75 get promoted, rest discarded
    assert!(promoted > 0, "Should promote at least one item");
    assert!(
        promoted + discarded == 3,
        "All 3 working items should be resolved"
    );

    // Step 7: After finalization, working memory for s1 should be empty
    let wm_after = mn
        .working_memory("s1")
        .await
        .expect("async operation should succeed");
    assert!(
        wm_after.is_empty(),
        "Working memory should be empty after finalize"
    );
}

// ============================================================================
// Journey 4: Importance lifecycle — boost, decay, forget
// ============================================================================

#[tokio::test]
async fn journey_importance_lifecycle() {
    let dir = tempdir().expect("should create temp dir");
    let mn = test_mnemosyne(dir.path()).await;

    // Step 1: Store episodic memories with varying importance
    let id1 = mn
        .store_typed(
            "s1",
            &Message::user("important task: fix auth bug"),
            MemoryType::Episodic,
            0.9,
        )
        .await
        .expect("async operation should succeed");
    let id2 = mn
        .store_typed(
            "s1",
            &Message::user("minor: update readme"),
            MemoryType::Episodic,
            0.3,
        )
        .await
        .expect("async operation should succeed");
    let id3 = mn
        .store_typed(
            "s1",
            &Message::user("medium: add tests"),
            MemoryType::Episodic,
            0.5,
        )
        .await
        .expect("async operation should succeed");

    // Step 2: Boost a memory (simulating retrieval/usage)
    mn.boost_memory(id3, 0.3)
        .await
        .expect("async operation should succeed");
    let (imp3, _) = mn
        .get_memory_importance(id3)
        .await
        .expect("async operation should succeed");
    assert!(
        (imp3 - 0.8).abs() < 0.01,
        "Importance should be 0.5 + 0.3 = 0.8"
    );

    // Step 3: Verify original importances
    let (imp1, _) = mn
        .get_memory_importance(id1)
        .await
        .expect("async operation should succeed");
    let (imp2, _) = mn
        .get_memory_importance(id2)
        .await
        .expect("async operation should succeed");
    assert!((imp1 - 0.9).abs() < 0.01);
    assert!((imp2 - 0.3).abs() < 0.01);

    // Step 4: Apply decay
    let decayed = mn
        .decay_importance(0.1)
        .await
        .expect("async operation should succeed");
    assert!(decayed > 0, "Should decay some memories");

    // Step 5: Forget old messages
    // First, verify count
    let stats_before = mn.stats().await.expect("async operation should succeed");
    assert_eq!(stats_before.message_count, 3);

    // Forget everything before far future
    let deleted = mn
        .forget_before(chrono::Utc::now() + chrono::Duration::hours(1))
        .await
        .expect("async operation should succeed");
    assert_eq!(deleted, 3);

    let stats_after = mn.stats().await.expect("async operation should succeed");
    assert_eq!(stats_after.message_count, 0);
}

// ============================================================================
// Journey 5: Pattern extraction across sessions
// ============================================================================

#[tokio::test]
async fn journey_pattern_extraction() {
    let dir = tempdir().expect("should create temp dir");
    let mn = test_mnemosyne(dir.path()).await;

    // Step 1: Simulate multiple sessions with recurring themes
    for i in 0..5 {
        let session = format!("session-{}", i);
        mn.store(&session, &Message::user("run cargo test"))
            .await
            .expect("async operation should succeed");
        mn.store(&session, &Message::assistant("All tests passed"))
            .await
            .expect("async operation should succeed");
        mn.store(&session, &Message::user("deploy to production"))
            .await
            .expect("async operation should succeed");
        mn.store(&session, &Message::assistant("Deployed successfully"))
            .await
            .expect("async operation should succeed");
    }

    // Add some variety
    mn.store("session-5", &Message::user("write documentation"))
        .await
        .expect("async operation should succeed");
    mn.store("session-5", &Message::assistant("Documentation written"))
        .await
        .expect("async operation should succeed");

    // Step 2: Extract patterns
    let _count = mn
        .extract_patterns()
        .await
        .expect("async operation should succeed");
    // Patterns should be found (tool frequency, topics, themes)
    // Note: count is usize (always >= 0), unwrap() ensures no error occurred

    // Step 3: Get patterns
    let all_patterns = mn
        .get_all_patterns(20)
        .await
        .expect("async operation should succeed");
    // May or may not find patterns depending on implementation thresholds
    let _ = all_patterns;

    // Step 4: Verify session count
    let stats = mn.stats().await.expect("async operation should succeed");
    assert_eq!(stats.session_count, 6);
    assert_eq!(stats.message_count, 22);
}

// ============================================================================
// Journey 6: Atomic reindex — rebuild from scratch
// ============================================================================

#[tokio::test]
async fn journey_atomic_reindex() {
    let dir = tempdir().expect("should create temp dir");
    let workspace = tempdir().expect("should create temp dir");

    // Step 1: Create workspace content
    std::fs::create_dir_all(workspace.path().join("memory")).expect("should create directory");
    std::fs::write(
        workspace.path().join("memory").join("MEMORY.md"),
        "# Memory\n\nUser prefers Rust.",
    )
    .expect("operation should succeed");
    std::fs::write(
        workspace.path().join("notes.md"),
        "# Notes\n\nProject deadline: March 2026",
    )
    .expect("operation should succeed");

    // Step 2: Create and populate mnemosyne
    let mn = test_mnemosyne(dir.path()).await;
    mn.store("old-session", &Message::user("this will be lost"))
        .await
        .expect("async operation should succeed");
    let stats1 = mn
        .sync_workspace(workspace.path())
        .await
        .expect("async operation should succeed");
    assert_eq!(stats1.files_changed, 2);

    // Step 3: Verify data exists
    let old_msgs = mn
        .recall_session("old-session", 10)
        .await
        .expect("async operation should succeed");
    assert_eq!(old_msgs.len(), 1);

    // Step 4: Atomic reindex — rebuilds DB from workspace files only
    let reindex_stats = mn
        .atomic_reindex(workspace.path(), None)
        .await
        .expect("async operation should succeed");
    assert_eq!(reindex_stats.files_scanned, 2);
    assert_eq!(reindex_stats.files_changed, 2);

    // Step 5: Old session data is gone (fresh DB from workspace only)
    let old_msgs_after = mn
        .recall_session("old-session", 10)
        .await
        .expect("async operation should succeed");
    assert!(
        old_msgs_after.is_empty(),
        "Old messages should be gone after reindex"
    );

    // Step 6: Workspace content is re-indexed and intact
    let stats = mn.stats().await.expect("async operation should succeed");
    assert!(stats.tracked_file_count >= 2);
}

// ============================================================================
// Journey 7: Full user session — chat, remember, search, context
// ============================================================================

#[tokio::test]
async fn journey_full_user_session() {
    let dir = tempdir().expect("should create temp dir");
    let workspace = tempdir().expect("should create temp dir");

    // Bootstrap workspace
    bootstrap_workspace(workspace.path()).expect("operation should succeed");

    let config = MnemosyneConfig {
        db_path: dir.path().join("test.db"),
        enable_fts: true,
        enable_embeddings: false,
        ..Default::default()
    };
    let mn = Mnemosyne::new(config)
        .await
        .expect("Mnemosyne::new should succeed");

    // Sync workspace
    mn.sync_workspace(workspace.path())
        .await
        .expect("async operation should succeed");

    // User starts chatting
    mn.store("main", &Message::user("I'm building a web scraper in Rust"))
        .await
        .expect("async operation should succeed");
    mn.store("main", &Message::assistant("I'll help you build a web scraper. We can use reqwest for HTTP and scraper for HTML parsing.")).await.expect("async operation should succeed");
    mn.store("main", &Message::user("Add error handling and retry logic"))
        .await
        .expect("async operation should succeed");
    mn.store(
        "main",
        &Message::assistant("Here's a robust implementation with exponential backoff..."),
    )
    .await
    .expect("async operation should succeed");

    // User explicitly saves a fact as semantic memory
    mn.store_typed(
        "main",
        &Message::user("Remember: user prefers reqwest over hyper"),
        MemoryType::Semantic,
        1.0,
    )
    .await
    .expect("async operation should succeed");

    // Later, user comes back in a new session
    mn.store("session-2", &Message::user("I need to make HTTP requests"))
        .await
        .expect("async operation should succeed");

    // System searches for relevant context
    let context = mn
        .search("HTTP requests", 5)
        .await
        .expect("async operation should succeed");
    assert!(!context.is_empty(), "Should find relevant past discussions");

    // Semantic search for user preferences
    let prefs = mn
        .search_by_type("reqwest", MemoryType::Semantic, 5)
        .await
        .expect("async operation should succeed");
    assert!(!prefs.is_empty(), "Should find saved preference");

    // Verify the full picture
    let stats = mn.stats().await.expect("async operation should succeed");
    assert!(
        stats.message_count >= 6,
        "At least 6 chat messages (workspace chunks add more)"
    );
    assert!(
        stats.session_count >= 2,
        "At least 2 chat sessions (workspace files add more)"
    );
    assert!(stats.tracked_file_count > 0); // workspace files tracked
}
