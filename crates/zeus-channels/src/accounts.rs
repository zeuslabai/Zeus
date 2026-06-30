//! Multi-account support for channel adapters.
//!
//! Allows multiple bot tokens / credentials per channel type,
//! persisted to `accounts.json` in the workspace directory.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;

/// Unique identifier for a channel account.
pub type AccountId = String;

/// Error type for account operations.
#[derive(Debug, thiserror::Error)]
pub enum AccountError {
    #[error("Account not found: {0}")]
    NotFound(String),

    #[error("Account already exists: {0}")]
    AlreadyExists(String),

    #[error("Invalid credentials: {0}")]
    InvalidCredentials(String),

    #[error("Persistence error: {0}")]
    PersistenceError(String),
}

impl From<AccountError> for zeus_core::Error {
    fn from(e: AccountError) -> Self {
        zeus_core::Error::Channel(e.to_string())
    }
}

/// A single account (set of credentials) for a channel adapter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelAccount {
    /// Unique account identifier (UUID).
    pub id: AccountId,
    /// Channel type this account belongs to (e.g. "telegram", "discord").
    pub channel_type: String,
    /// Human-friendly label (e.g. "Work Bot", "Personal Bot").
    pub label: String,
    /// Credentials map (e.g. bot_token, api_key, api_hash).
    pub credentials: HashMap<String, String>,
    /// Whether this account is enabled.
    pub enabled: bool,
    /// When this account was created.
    pub created_at: DateTime<Utc>,
    /// Arbitrary extra metadata.
    pub metadata: HashMap<String, String>,
}

impl ChannelAccount {
    /// Create a new account with the given channel type and label.
    ///
    /// Generates a new UUID and sets `created_at` to now.
    pub fn new(channel_type: &str, label: &str) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            channel_type: channel_type.to_string(),
            label: label.to_string(),
            credentials: HashMap::new(),
            enabled: true,
            created_at: Utc::now(),
            metadata: HashMap::new(),
        }
    }

    /// Builder: insert a credential key-value pair.
    pub fn with_credential(mut self, key: &str, value: &str) -> Self {
        self.credentials.insert(key.to_string(), value.to_string());
        self
    }

    /// Builder: insert a metadata key-value pair.
    pub fn with_metadata(mut self, key: &str, value: &str) -> Self {
        self.metadata.insert(key.to_string(), value.to_string());
        self
    }
}

/// Persistent store for channel accounts.
///
/// Thread-safe via `Arc<Mutex<...>>` — safe to share across async tasks.
pub struct AccountStore {
    accounts: Arc<Mutex<HashMap<AccountId, ChannelAccount>>>,
    file_path: PathBuf,
}

impl AccountStore {
    /// Create (or load) an account store backed by `<workspace_dir>/accounts.json`.
    pub fn new(workspace_dir: &Path) -> Self {
        let file_path = workspace_dir.join("accounts.json");
        let accounts = Self::load(&file_path).unwrap_or_default();
        Self {
            accounts: Arc::new(Mutex::new(accounts)),
            file_path,
        }
    }

    /// Add a new account. Returns `AccountError::AlreadyExists` if the id is taken.
    pub async fn add_account(&self, account: ChannelAccount) -> Result<(), AccountError> {
        let mut map = self.accounts.lock().await;
        if map.contains_key(&account.id) {
            return Err(AccountError::AlreadyExists(account.id.clone()));
        }
        map.insert(account.id.clone(), account);
        drop(map);
        self.save().await
    }

    /// Get an account by id.
    pub async fn get_account(&self, id: &str) -> Option<ChannelAccount> {
        self.accounts.lock().await.get(id).cloned()
    }

    /// List all accounts (arbitrary order).
    pub async fn list_accounts(&self) -> Vec<ChannelAccount> {
        self.accounts.lock().await.values().cloned().collect()
    }

    /// List accounts matching a specific channel type.
    pub async fn list_accounts_for_type(&self, channel_type: &str) -> Vec<ChannelAccount> {
        self.accounts
            .lock()
            .await
            .values()
            .filter(|a| a.channel_type == channel_type)
            .cloned()
            .collect()
    }

    /// Update an existing account. Returns `AccountError::NotFound` if id is missing.
    pub async fn update_account(&self, account: ChannelAccount) -> Result<(), AccountError> {
        let mut map = self.accounts.lock().await;
        if !map.contains_key(&account.id) {
            return Err(AccountError::NotFound(account.id.clone()));
        }
        map.insert(account.id.clone(), account);
        drop(map);
        self.save().await
    }

    /// Delete an account by id. Returns `AccountError::NotFound` if id is missing.
    pub async fn delete_account(&self, id: &str) -> Result<(), AccountError> {
        let mut map = self.accounts.lock().await;
        if map.remove(id).is_none() {
            return Err(AccountError::NotFound(id.to_string()));
        }
        drop(map);
        self.save().await
    }

    /// Persist the current state to the JSON file.
    pub async fn save(&self) -> Result<(), AccountError> {
        let map = self.accounts.lock().await;
        let json = serde_json::to_string_pretty(&*map).map_err(|e| {
            AccountError::PersistenceError(format!("Failed to serialize accounts: {e}"))
        })?;
        drop(map);

        // Ensure parent directory exists (non-blocking)
        if let Some(parent) = self.file_path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                AccountError::PersistenceError(format!("Failed to create directory: {e}"))
            })?;
        }

        tokio::fs::write(&self.file_path, json).await.map_err(|e| {
            AccountError::PersistenceError(format!("Failed to write accounts file: {e}"))
        })
    }

    /// Load accounts from a JSON file. Returns an empty map on any failure.
    pub fn load(path: &Path) -> Result<HashMap<AccountId, ChannelAccount>, AccountError> {
        if !path.exists() {
            return Ok(HashMap::new());
        }
        let data = std::fs::read_to_string(path).map_err(|e| {
            AccountError::PersistenceError(format!("Failed to read accounts file: {e}"))
        })?;
        let map: HashMap<AccountId, ChannelAccount> = serde_json::from_str(&data).map_err(|e| {
            AccountError::PersistenceError(format!("Failed to parse accounts file: {e}"))
        })?;
        Ok(map)
    }

    /// Return the number of stored accounts.
    pub async fn account_count(&self) -> usize {
        self.accounts.lock().await.len()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Helper: create a store in a temporary directory.
    fn temp_store() -> (AccountStore, TempDir) {
        let dir = TempDir::new().expect("failed to create temp dir");
        let store = AccountStore::new(dir.path());
        (store, dir)
    }

    /// Helper: build a simple test account for a given channel type.
    fn make_account(channel_type: &str, label: &str) -> ChannelAccount {
        ChannelAccount::new(channel_type, label).with_credential("bot_token", "tok_test_123")
    }

    // ---- CRUD basics ----

    #[tokio::test]
    async fn test_add_and_get_account() {
        let (store, _dir) = temp_store();
        let acct = make_account("telegram", "Work Bot");
        let id = acct.id.clone();

        store
            .add_account(acct)
            .await
            .expect("async operation should succeed");

        let fetched = store.get_account(&id).await.expect("account should exist");
        assert_eq!(fetched.label, "Work Bot");
        assert_eq!(fetched.channel_type, "telegram");
        assert_eq!(
            fetched
                .credentials
                .get("bot_token")
                .expect("key should exist"),
            "tok_test_123"
        );
    }

    #[tokio::test]
    async fn test_list_accounts() {
        let (store, _dir) = temp_store();
        store
            .add_account(make_account("telegram", "Bot A"))
            .await
            .expect("async operation should succeed");
        store
            .add_account(make_account("discord", "Bot B"))
            .await
            .expect("async operation should succeed");

        let all = store.list_accounts().await;
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn test_update_account() {
        let (store, _dir) = temp_store();
        let mut acct = make_account("slack", "Slack Bot");
        let id = acct.id.clone();
        store
            .add_account(acct.clone())
            .await
            .expect("async operation should succeed");

        acct.label = "Updated Slack Bot".to_string();
        acct.credentials
            .insert("bot_token".to_string(), "new_tok".to_string());
        store
            .update_account(acct)
            .await
            .expect("async operation should succeed");

        let fetched = store
            .get_account(&id)
            .await
            .expect("async operation should succeed");
        assert_eq!(fetched.label, "Updated Slack Bot");
        assert_eq!(
            fetched
                .credentials
                .get("bot_token")
                .expect("key should exist"),
            "new_tok"
        );
    }

    #[tokio::test]
    async fn test_delete_account() {
        let (store, _dir) = temp_store();
        let acct = make_account("email", "Email Bot");
        let id = acct.id.clone();
        store
            .add_account(acct)
            .await
            .expect("async operation should succeed");

        store
            .delete_account(&id)
            .await
            .expect("async operation should succeed");
        assert!(store.get_account(&id).await.is_none());
    }

    // ---- Filtering ----

    #[tokio::test]
    async fn test_list_accounts_for_type() {
        let (store, _dir) = temp_store();
        store
            .add_account(make_account("telegram", "TG 1"))
            .await
            .expect("async operation should succeed");
        store
            .add_account(make_account("telegram", "TG 2"))
            .await
            .expect("async operation should succeed");
        store
            .add_account(make_account("discord", "DC 1"))
            .await
            .expect("async operation should succeed");

        let tg = store.list_accounts_for_type("telegram").await;
        assert_eq!(tg.len(), 2);
        assert!(tg.iter().all(|a| a.channel_type == "telegram"));

        let dc = store.list_accounts_for_type("discord").await;
        assert_eq!(dc.len(), 1);

        let empty = store.list_accounts_for_type("matrix").await;
        assert!(empty.is_empty());
    }

    // ---- Persistence roundtrip ----

    #[tokio::test]
    async fn test_save_and_load_roundtrip() {
        let dir = TempDir::new().expect("TempDir::new should succeed");

        // Create store, add accounts, persist
        {
            let store = AccountStore::new(dir.path());
            store
                .add_account(
                    make_account("telegram", "Persist Bot").with_metadata("region", "us-east"),
                )
                .await
                .expect("async operation should succeed");
            store
                .add_account(make_account("discord", "DC Persist"))
                .await
                .expect("async operation should succeed");
        }

        // Reload from same directory
        let store2 = AccountStore::new(dir.path());
        let all = store2.list_accounts().await;
        assert_eq!(all.len(), 2);

        let tg = store2.list_accounts_for_type("telegram").await;
        assert_eq!(tg.len(), 1);
        assert_eq!(tg[0].label, "Persist Bot");
        assert_eq!(
            tg[0].metadata.get("region").expect("key should exist"),
            "us-east"
        );
    }

    // ---- Serialization / deserialization ----

    #[test]
    fn test_channel_account_serde() {
        let acct = make_account("telegram", "Serde Bot")
            .with_credential("api_hash", "abc123")
            .with_metadata("env", "production");

        let json = serde_json::to_string(&acct).expect("should serialize to JSON");
        let deser: ChannelAccount = serde_json::from_str(&json).expect("should parse successfully");

        assert_eq!(deser.id, acct.id);
        assert_eq!(deser.channel_type, "telegram");
        assert_eq!(deser.label, "Serde Bot");
        assert_eq!(
            deser.credentials.get("api_hash").expect("key should exist"),
            "abc123"
        );
        assert_eq!(
            deser.metadata.get("env").expect("key should exist"),
            "production"
        );
        assert!(deser.enabled);
    }

    // ---- Error display ----

    #[test]
    fn test_account_error_display() {
        let e = AccountError::NotFound("abc".to_string());
        assert_eq!(e.to_string(), "Account not found: abc");

        let e = AccountError::AlreadyExists("xyz".to_string());
        assert_eq!(e.to_string(), "Account already exists: xyz");

        let e = AccountError::InvalidCredentials("missing token".to_string());
        assert_eq!(e.to_string(), "Invalid credentials: missing token");

        let e = AccountError::PersistenceError("disk full".to_string());
        assert_eq!(e.to_string(), "Persistence error: disk full");
    }

    // ---- Duplicate detection ----

    #[tokio::test]
    async fn test_duplicate_account_rejected() {
        let (store, _dir) = temp_store();
        let acct = make_account("telegram", "Dup Bot");
        let id = acct.id.clone();

        store
            .add_account(acct.clone())
            .await
            .expect("async operation should succeed");

        // Second add with same id should fail
        let mut dup = acct;
        dup.label = "Dup Bot 2".to_string();
        let err = store.add_account(dup).await.unwrap_err();
        match err {
            AccountError::AlreadyExists(ref eid) => assert_eq!(eid, &id),
            other => panic!("expected AlreadyExists, got: {other}"),
        }
    }

    // ---- account_count ----

    #[tokio::test]
    async fn test_account_count() {
        let (store, _dir) = temp_store();
        assert_eq!(store.account_count().await, 0);

        store
            .add_account(make_account("telegram", "A"))
            .await
            .expect("async operation should succeed");
        assert_eq!(store.account_count().await, 1);

        store
            .add_account(make_account("discord", "B"))
            .await
            .expect("async operation should succeed");
        assert_eq!(store.account_count().await, 2);
    }

    // ---- Empty store behaviour ----

    #[tokio::test]
    async fn test_empty_store() {
        let (store, _dir) = temp_store();

        assert_eq!(store.account_count().await, 0);
        assert!(store.list_accounts().await.is_empty());
        assert!(store.get_account("nonexistent").await.is_none());
        assert!(store.list_accounts_for_type("telegram").await.is_empty());
    }

    // ---- Update / delete non-existent ----

    #[tokio::test]
    async fn test_update_nonexistent_account() {
        let (store, _dir) = temp_store();
        let acct = make_account("telegram", "Ghost");
        let err = store.update_account(acct).await.unwrap_err();
        assert!(matches!(err, AccountError::NotFound(_)));
    }

    #[tokio::test]
    async fn test_delete_nonexistent_account() {
        let (store, _dir) = temp_store();
        let err = store.delete_account("no-such-id").await.unwrap_err();
        assert!(matches!(err, AccountError::NotFound(_)));
    }

    // ---- Multiple accounts per channel type ----

    #[tokio::test]
    async fn test_multiple_accounts_per_type() {
        let (store, _dir) = temp_store();

        let a1 =
            ChannelAccount::new("telegram", "Work TG").with_credential("bot_token", "tok_work");
        let a2 = ChannelAccount::new("telegram", "Personal TG")
            .with_credential("bot_token", "tok_personal");
        let a3 = ChannelAccount::new("telegram", "CI TG").with_credential("bot_token", "tok_ci");

        store
            .add_account(a1)
            .await
            .expect("async operation should succeed");
        store
            .add_account(a2)
            .await
            .expect("async operation should succeed");
        store
            .add_account(a3)
            .await
            .expect("async operation should succeed");

        let tg = store.list_accounts_for_type("telegram").await;
        assert_eq!(tg.len(), 3);

        // All should have distinct ids
        let mut ids: Vec<&str> = tg.iter().map(|a| a.id.as_str()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 3);

        // Verify labels
        let mut labels: Vec<&str> = tg.iter().map(|a| a.label.as_str()).collect();
        labels.sort();
        assert_eq!(labels, vec!["CI TG", "Personal TG", "Work TG"]);
    }

    // ---- AccountError -> zeus_core::Error conversion ----

    #[test]
    fn test_account_error_converts_to_core_error() {
        let err: zeus_core::Error = AccountError::NotFound("x".to_string()).into();
        let msg = err.to_string();
        assert!(msg.contains("Account not found: x"), "got: {msg}");
    }

    // ---- ChannelAccount builder helpers ----

    #[test]
    fn test_channel_account_builders() {
        let acct = ChannelAccount::new("discord", "My Bot")
            .with_credential("token", "abc")
            .with_credential("client_id", "123")
            .with_metadata("env", "staging")
            .with_metadata("version", "2");

        assert_eq!(acct.credentials.len(), 2);
        assert_eq!(acct.metadata.len(), 2);
        assert_eq!(acct.credentials["token"], "abc");
        assert_eq!(acct.metadata["version"], "2");
        assert!(acct.enabled);
        assert!(!acct.id.is_empty());
    }
}
