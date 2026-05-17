//! Conversation branching support
//!
//! Allows creating branches from existing sessions at specific message indices,
//! enabling alternative conversation paths ("what-if" exploration).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::fs;
use zeus_core::{Error, Result};

use crate::Session;

// ============================================================================
// Types
// ============================================================================

/// A point where a conversation was branched
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchPoint {
    /// ID of the parent session that was branched from
    pub parent_session_id: String,
    /// ID of the newly created branch session
    pub branch_session_id: String,
    /// Message index where the branch starts (messages before this index are copied)
    pub branch_at_index: usize,
    /// When this branch was created
    pub created: DateTime<Utc>,
    /// Optional human-readable label for this branch
    pub label: Option<String>,
}

// ============================================================================
// BranchManager
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct BranchStoreData {
    branches: Vec<BranchPoint>,
}

/// Manages session branches with JSON file persistence
pub struct BranchManager {
    sessions_dir: PathBuf,
    store_path: PathBuf,
}

impl BranchManager {
    /// Create a new BranchManager for the given sessions directory
    pub fn new(sessions_dir: impl AsRef<Path>) -> Self {
        let sessions_dir = sessions_dir.as_ref().to_path_buf();
        let store_path = sessions_dir.join("branches.json");
        Self {
            sessions_dir,
            store_path,
        }
    }

    /// Load branch data from disk
    async fn load(&self) -> BranchStoreData {
        if !self.store_path.exists() {
            return BranchStoreData::default();
        }
        match fs::read_to_string(&self.store_path).await {
            Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
            Err(_) => BranchStoreData::default(),
        }
    }

    /// Persist branch data to disk
    async fn save(&self, data: &BranchStoreData) -> Result<()> {
        if let Some(parent) = self.store_path.parent() {
            fs::create_dir_all(parent).await?;
        }
        let json = serde_json::to_string_pretty(data)?;
        fs::write(&self.store_path, json).await?;
        Ok(())
    }

    /// Create a branch from a parent session at a specific message index.
    ///
    /// Copies messages `[0..at_index]` from the parent session into a new session.
    /// Returns the `BranchPoint` describing the branch.
    pub async fn create_branch(
        &self,
        parent_id: &str,
        at_index: usize,
        label: Option<String>,
    ) -> Result<BranchPoint> {
        // Load the parent session
        let parent = Session::load(&self.sessions_dir, parent_id).await?;

        // Validate the index
        if at_index > parent.messages.len() {
            return Err(Error::Session(format!(
                "Branch index {} exceeds message count {} in session {}",
                at_index,
                parent.messages.len(),
                parent_id,
            )));
        }

        // Create a new session with the copied messages
        let mut branch = Session::new(&self.sessions_dir);
        branch.init().await?;

        for msg in parent.messages.iter().take(at_index) {
            branch.add(msg.clone()).await?;
        }

        // Record the branch point
        let branch_point = BranchPoint {
            parent_session_id: parent_id.to_string(),
            branch_session_id: branch.id.clone(),
            branch_at_index: at_index,
            created: Utc::now(),
            label,
        };

        let mut data = self.load().await;
        data.branches.push(branch_point.clone());
        self.save(&data).await?;

        Ok(branch_point)
    }

    /// List all branches of a given session (where the session is the parent)
    pub async fn list_branches(&self, session_id: &str) -> Vec<BranchPoint> {
        let data = self.load().await;
        data.branches
            .into_iter()
            .filter(|b| b.parent_session_id == session_id)
            .collect()
    }

    /// Get a specific branch by its branch session ID
    pub async fn get_branch(&self, branch_id: &str) -> Option<BranchPoint> {
        let data = self.load().await;
        data.branches
            .into_iter()
            .find(|b| b.branch_session_id == branch_id)
    }

    /// Delete a branch record (does NOT delete the branch session file)
    pub async fn delete_branch(&self, branch_id: &str) -> Result<()> {
        let mut data = self.load().await;
        let before = data.branches.len();
        data.branches.retain(|b| b.branch_session_id != branch_id);

        if data.branches.len() == before {
            return Err(Error::Session(format!("Branch not found: {}", branch_id)));
        }

        self.save(&data).await
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use zeus_core::Message;

    /// Helper: create a session with N user/assistant message pairs
    async fn create_test_session(dir: &Path, pairs: usize) -> Session {
        let mut session = Session::new(dir);
        session
            .init()
            .await
            .expect("async operation should succeed");
        for i in 0..pairs {
            session
                .add(Message::user(format!("User message {}", i)))
                .await
                .expect("async operation should succeed");
            session
                .add(Message::assistant(format!("Assistant reply {}", i)))
                .await
                .expect("async operation should succeed");
        }
        session
    }

    #[tokio::test]
    async fn test_create_branch_from_session() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let session = create_test_session(tmp.path(), 3).await;
        assert_eq!(session.len(), 6);

        let mgr = BranchManager::new(tmp.path());
        let bp = mgr
            .create_branch(&session.id, 4, Some("try different approach".to_string()))
            .await
            .expect("should serialize");

        assert_eq!(bp.parent_session_id, session.id);
        assert_eq!(bp.branch_at_index, 4);
        assert_eq!(bp.label, Some("try different approach".to_string()));

        // Load the branch session and verify it has the first 4 messages
        let branch_session = Session::load(tmp.path(), &bp.branch_session_id)
            .await
            .expect("async operation should succeed");
        assert_eq!(branch_session.len(), 4);
        assert_eq!(branch_session.messages[0].content, "User message 0");
        assert_eq!(branch_session.messages[3].content, "Assistant reply 1");
    }

    #[tokio::test]
    async fn test_branch_preserves_messages_up_to_index() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let session = create_test_session(tmp.path(), 5).await;
        assert_eq!(session.len(), 10);

        let mgr = BranchManager::new(tmp.path());
        let bp = mgr
            .create_branch(&session.id, 7, None)
            .await
            .expect("async operation should succeed");

        let branch = Session::load(tmp.path(), &bp.branch_session_id)
            .await
            .expect("async operation should succeed");
        assert_eq!(branch.len(), 7);

        // Verify exact messages
        for i in 0..7 {
            assert_eq!(branch.messages[i].content, session.messages[i].content);
        }
    }

    #[tokio::test]
    async fn test_list_branches() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let session = create_test_session(tmp.path(), 3).await;

        let mgr = BranchManager::new(tmp.path());
        mgr.create_branch(&session.id, 2, Some("branch-a".to_string()))
            .await
            .expect("should serialize");
        mgr.create_branch(&session.id, 4, Some("branch-b".to_string()))
            .await
            .expect("should serialize");

        let branches = mgr.list_branches(&session.id).await;
        assert_eq!(branches.len(), 2);
        assert_eq!(branches[0].label, Some("branch-a".to_string()));
        assert_eq!(branches[1].label, Some("branch-b".to_string()));

        // Other session has no branches
        let other = create_test_session(tmp.path(), 1).await;
        let other_branches = mgr.list_branches(&other.id).await;
        assert!(other_branches.is_empty());
    }

    #[tokio::test]
    async fn test_branch_at_index_zero() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let session = create_test_session(tmp.path(), 2).await;

        let mgr = BranchManager::new(tmp.path());
        let bp = mgr
            .create_branch(&session.id, 0, None)
            .await
            .expect("async operation should succeed");

        // Branch at 0 means no messages copied
        let branch = Session::load(tmp.path(), &bp.branch_session_id)
            .await
            .expect("async operation should succeed");
        assert_eq!(branch.len(), 0);
        assert_eq!(bp.branch_at_index, 0);
    }

    #[tokio::test]
    async fn test_branch_at_last_message() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let session = create_test_session(tmp.path(), 3).await;
        let total = session.len();

        let mgr = BranchManager::new(tmp.path());
        let bp = mgr
            .create_branch(&session.id, total, None)
            .await
            .expect("async operation should succeed");

        // Branch at total means all messages copied
        let branch = Session::load(tmp.path(), &bp.branch_session_id)
            .await
            .expect("async operation should succeed");
        assert_eq!(branch.len(), total);
    }

    #[tokio::test]
    async fn test_branch_of_a_branch() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");

        // Create original session with 4 messages
        let session = create_test_session(tmp.path(), 2).await;
        assert_eq!(session.len(), 4);

        let mgr = BranchManager::new(tmp.path());

        // Branch at index 3 (first 3 messages)
        let bp1 = mgr
            .create_branch(&session.id, 3, Some("first-branch".to_string()))
            .await
            .expect("should serialize");

        // Add a message to the branch session
        let mut branch1 = Session::load(tmp.path(), &bp1.branch_session_id)
            .await
            .expect("async operation should succeed");
        branch1
            .add(Message::user("New direction"))
            .await
            .expect("async operation should succeed");
        branch1
            .add(Message::assistant("Okay, exploring new path"))
            .await
            .expect("async operation should succeed");
        assert_eq!(branch1.len(), 5);

        // Now branch the branch at index 4
        let bp2 = mgr
            .create_branch(&bp1.branch_session_id, 4, Some("nested-branch".to_string()))
            .await
            .expect("should serialize");

        let branch2 = Session::load(tmp.path(), &bp2.branch_session_id)
            .await
            .expect("async operation should succeed");
        assert_eq!(branch2.len(), 4);
        assert_eq!(bp2.parent_session_id, bp1.branch_session_id);
    }

    #[tokio::test]
    async fn test_delete_branch() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let session = create_test_session(tmp.path(), 2).await;

        let mgr = BranchManager::new(tmp.path());
        let bp = mgr
            .create_branch(&session.id, 2, Some("to-delete".to_string()))
            .await
            .expect("should serialize");

        // Branch exists
        assert!(mgr.get_branch(&bp.branch_session_id).await.is_some());

        // Delete it
        mgr.delete_branch(&bp.branch_session_id)
            .await
            .expect("async operation should succeed");

        // Branch record is gone
        assert!(mgr.get_branch(&bp.branch_session_id).await.is_none());
        assert!(mgr.list_branches(&session.id).await.is_empty());

        // Deleting again should error
        assert!(mgr.delete_branch(&bp.branch_session_id).await.is_err());
    }

    #[tokio::test]
    async fn test_branch_with_label() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let session = create_test_session(tmp.path(), 1).await;

        let mgr = BranchManager::new(tmp.path());

        // With label
        let bp1 = mgr
            .create_branch(&session.id, 1, Some("experiment-a".to_string()))
            .await
            .expect("should serialize");
        assert_eq!(bp1.label, Some("experiment-a".to_string()));

        // Without label
        let bp2 = mgr
            .create_branch(&session.id, 1, None)
            .await
            .expect("async operation should succeed");
        assert_eq!(bp2.label, None);

        // Verify both are listed
        let branches = mgr.list_branches(&session.id).await;
        assert_eq!(branches.len(), 2);
    }

    #[tokio::test]
    async fn test_branch_index_out_of_range() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let session = create_test_session(tmp.path(), 1).await;
        assert_eq!(session.len(), 2);

        let mgr = BranchManager::new(tmp.path());

        // Index 3 exceeds message count of 2
        let result = mgr.create_branch(&session.id, 3, None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_branch_persistence() {
        let tmp = TempDir::new().expect("TempDir::new should succeed");
        let session = create_test_session(tmp.path(), 2).await;

        // Create branch with one manager instance
        let mgr = BranchManager::new(tmp.path());
        let bp = mgr
            .create_branch(&session.id, 3, Some("persistent".to_string()))
            .await
            .expect("should serialize");

        // Load with a new manager instance (simulates restart)
        let mgr2 = BranchManager::new(tmp.path());
        let loaded = mgr2.get_branch(&bp.branch_session_id).await;
        assert!(loaded.is_some());

        let loaded = loaded.expect("operation should succeed");
        assert_eq!(loaded.parent_session_id, session.id);
        assert_eq!(loaded.branch_at_index, 3);
        assert_eq!(loaded.label, Some("persistent".to_string()));
    }
}
