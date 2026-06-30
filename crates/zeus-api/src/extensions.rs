//! Extension management — CRUD for installed extensions
//!
//! In-memory DashMap for fast concurrent access + JSON file persistence.

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs;
use uuid::Uuid;

// ============================================================================
// Types
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExtensionStatus {
    Installed,
    Running,
    Stopped,
    Error,
}

impl std::fmt::Display for ExtensionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Installed => write!(f, "installed"),
            Self::Running => write!(f, "running"),
            Self::Stopped => write!(f, "stopped"),
            Self::Error => write!(f, "error"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionPermissions {
    pub allow_read: bool,
    pub allow_write: bool,
    pub allow_net: bool,
    pub allow_env: bool,
    pub allow_run: bool,
}

impl Default for ExtensionPermissions {
    fn default() -> Self {
        Self {
            allow_read: true,
            allow_write: false,
            allow_net: false,
            allow_env: false,
            allow_run: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionInfo {
    pub id: String,
    pub name: String,
    pub source: String,
    pub version: Option<String>,
    pub description: Option<String>,
    pub status: ExtensionStatus,
    pub enabled: bool,
    pub permissions: ExtensionPermissions,
    pub installed_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ============================================================================
// ExtensionStore — DashMap + JSON file persistence
// ============================================================================

pub struct ExtensionStore {
    extensions: Arc<DashMap<String, ExtensionInfo>>,
    persist_path: PathBuf,
}

#[derive(Serialize, Deserialize, Default)]
struct PersistData {
    extensions: Vec<ExtensionInfo>,
}

impl ExtensionStore {
    /// Create a new extension store. Loads persisted data from disk synchronously.
    pub fn new(workspace_dir: impl AsRef<Path>) -> Self {
        let dir = workspace_dir.as_ref().join("extensions");
        let persist_path = dir.join("extensions.json");
        let extensions = Arc::new(DashMap::new());

        // Load from disk (sync — runs once at startup)
        if persist_path.exists()
            && let Ok(content) = std::fs::read_to_string(&persist_path)
            && let Ok(data) = serde_json::from_str::<PersistData>(&content)
        {
            for ext in data.extensions {
                extensions.insert(ext.id.clone(), ext);
            }
        }

        Self {
            extensions,
            persist_path,
        }
    }

    /// Persist current state to disk.
    async fn save(&self) -> Result<(), String> {
        if let Some(parent) = self.persist_path.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("Failed to create extensions dir: {}", e))?;
        }
        let exts: Vec<ExtensionInfo> = self
            .extensions
            .iter()
            .map(|entry| entry.value().clone())
            .collect();
        let data = PersistData { extensions: exts };
        let json = serde_json::to_string_pretty(&data).map_err(|e| e.to_string())?;
        fs::write(&self.persist_path, json)
            .await
            .map_err(|e| format!("Failed to write extensions.json: {}", e))
    }

    /// List all installed extensions.
    pub fn list(&self) -> Vec<ExtensionInfo> {
        self.extensions
            .iter()
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Get a single extension by ID.
    pub fn get(&self, id: &str) -> Option<ExtensionInfo> {
        self.extensions.get(id).map(|entry| entry.value().clone())
    }

    /// Install a new extension. Returns the created ExtensionInfo.
    pub async fn install(
        &self,
        name: String,
        source: String,
        version: Option<String>,
        description: Option<String>,
        permissions: Option<ExtensionPermissions>,
    ) -> Result<ExtensionInfo, String> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let ext = ExtensionInfo {
            id: id.clone(),
            name,
            source,
            version,
            description,
            status: ExtensionStatus::Installed,
            enabled: true,
            permissions: permissions.unwrap_or_default(),
            installed_at: now,
            updated_at: now,
        };
        self.extensions.insert(id, ext.clone());
        self.save().await?;
        Ok(ext)
    }

    /// Update an existing extension. Returns the updated ExtensionInfo.
    pub async fn update(
        &self,
        id: &str,
        enabled: Option<bool>,
        permissions: Option<ExtensionPermissions>,
    ) -> Result<ExtensionInfo, String> {
        let mut entry = self
            .extensions
            .get_mut(id)
            .ok_or_else(|| format!("Extension not found: {}", id))?;
        if let Some(enabled) = enabled {
            entry.enabled = enabled;
        }
        if let Some(perms) = permissions {
            entry.permissions = perms;
        }
        entry.updated_at = Utc::now();
        let updated = entry.clone();
        drop(entry);
        self.save().await?;
        Ok(updated)
    }

    /// Delete an extension by ID.
    pub async fn delete(&self, id: &str) -> Result<(), String> {
        self.extensions
            .remove(id)
            .ok_or_else(|| format!("Extension not found: {}", id))?;
        self.save().await
    }

    /// Set the runtime status of an extension (start/stop).
    pub async fn set_status(
        &self,
        id: &str,
        status: ExtensionStatus,
    ) -> Result<ExtensionInfo, String> {
        let mut entry = self
            .extensions
            .get_mut(id)
            .ok_or_else(|| format!("Extension not found: {}", id))?;
        entry.status = status;
        entry.updated_at = Utc::now();
        let updated = entry.clone();
        drop(entry);
        self.save().await?;
        Ok(updated)
    }

    /// Total number of installed extensions.
    pub fn count(&self) -> usize {
        self.extensions.len()
    }
}

// ============================================================================
// Runtime bridge — convert ExtensionInfo (metadata) → zeus_extensions::Extension
// ============================================================================

/// Convert an [`ExtensionInfo`] (API metadata layer) to a [`zeus_extensions::Extension`]
/// (runtime layer) so it can be registered with `zeus_extensions::ExtensionRegistry`.
///
/// The same ID is preserved, so callers can use the same ID from both `ExtensionStore`
/// and `ExtensionRegistry` without tracking a separate mapping.
///
/// Source convention:
/// - `http://` / `https://` prefix → `ExtensionSource::Url`
/// - `openclaw:<name>` prefix      → `ExtensionSource::OpenClaw`
/// - anything else                 → `ExtensionSource::Local` (filesystem path)
pub fn info_to_registry_extension(info: &ExtensionInfo) -> zeus_extensions::Extension {
    let source = if info.source.starts_with("http://") || info.source.starts_with("https://") {
        zeus_extensions::ExtensionSource::Url(info.source.clone())
    } else if let Some(name) = info.source.strip_prefix("openclaw:") {
        zeus_extensions::ExtensionSource::OpenClaw(name.to_string())
    } else {
        zeus_extensions::ExtensionSource::Local(info.source.clone())
    };

    let perms = zeus_extensions::ExtensionPermissions {
        allow_net: if info.permissions.allow_net {
            vec!["*".to_string()]
        } else {
            vec![]
        },
        allow_read: if info.permissions.allow_read {
            vec!["/".to_string()]
        } else {
            vec![]
        },
        allow_write: if info.permissions.allow_write {
            vec!["/tmp".to_string()]
        } else {
            vec![]
        },
        allow_env: if info.permissions.allow_env {
            vec!["*".to_string()]
        } else {
            vec![]
        },
    };

    let mut ext = zeus_extensions::Extension::new(info.name.clone(), source)
        .with_permissions(perms);

    if let Some(ref v) = info.version {
        ext = ext.with_version(v);
    }

    // Preserve the same UUID so ExtensionRegistry::start(id) works with ExtensionStore IDs.
    ext.id = info.id.clone();
    ext
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_extension_store_empty() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = ExtensionStore::new(tmp.path());
        assert_eq!(store.count(), 0);
        assert!(store.list().is_empty());
    }

    #[tokio::test]
    async fn test_install_and_list() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = ExtensionStore::new(tmp.path());

        let ext = store
            .install(
                "test-ext".to_string(),
                "https://example.com/ext.tar.gz".to_string(),
                Some("1.0.0".to_string()),
                Some("A test extension".to_string()),
                None,
            )
            .await
            .unwrap();

        assert_eq!(ext.name, "test-ext");
        assert_eq!(ext.status, ExtensionStatus::Installed);
        assert!(ext.enabled);
        assert_eq!(store.count(), 1);

        let list = store.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, ext.id);
    }

    #[tokio::test]
    async fn test_get_extension() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = ExtensionStore::new(tmp.path());

        let ext = store
            .install("my-ext".to_string(), "local".to_string(), None, None, None)
            .await
            .unwrap();

        let found = store.get(&ext.id).unwrap();
        assert_eq!(found.name, "my-ext");

        assert!(store.get("nonexistent").is_none());
    }

    #[tokio::test]
    async fn test_update_extension() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = ExtensionStore::new(tmp.path());

        let ext = store
            .install("upd-ext".to_string(), "src".to_string(), None, None, None)
            .await
            .unwrap();

        let updated = store.update(&ext.id, Some(false), None).await.unwrap();
        assert!(!updated.enabled);
        assert_eq!(updated.name, "upd-ext");
    }

    #[tokio::test]
    async fn test_update_not_found() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = ExtensionStore::new(tmp.path());
        assert!(store.update("nope", Some(true), None).await.is_err());
    }

    #[tokio::test]
    async fn test_delete_extension() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = ExtensionStore::new(tmp.path());

        let ext = store
            .install("del-ext".to_string(), "src".to_string(), None, None, None)
            .await
            .unwrap();

        store.delete(&ext.id).await.unwrap();
        assert_eq!(store.count(), 0);
        assert!(store.get(&ext.id).is_none());
    }

    #[tokio::test]
    async fn test_delete_not_found() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = ExtensionStore::new(tmp.path());
        assert!(store.delete("nope").await.is_err());
    }

    #[tokio::test]
    async fn test_set_status() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = ExtensionStore::new(tmp.path());

        let ext = store
            .install("stat-ext".to_string(), "src".to_string(), None, None, None)
            .await
            .unwrap();

        let updated = store
            .set_status(&ext.id, ExtensionStatus::Running)
            .await
            .unwrap();
        assert_eq!(updated.status, ExtensionStatus::Running);

        let stopped = store
            .set_status(&ext.id, ExtensionStatus::Stopped)
            .await
            .unwrap();
        assert_eq!(stopped.status, ExtensionStatus::Stopped);
    }

    #[tokio::test]
    async fn test_persistence_across_loads() {
        let tmp = tempfile::TempDir::new().unwrap();

        // Install in first store
        {
            let store = ExtensionStore::new(tmp.path());
            store
                .install(
                    "persist-ext".to_string(),
                    "https://ext.example.com".to_string(),
                    Some("2.0.0".to_string()),
                    None,
                    None,
                )
                .await
                .unwrap();
            assert_eq!(store.count(), 1);
        }

        // Load in fresh store — should have the extension
        {
            let store2 = ExtensionStore::new(tmp.path());
            assert_eq!(store2.count(), 1);
            let exts = store2.list();
            assert_eq!(exts[0].name, "persist-ext");
            assert_eq!(exts[0].version.as_deref(), Some("2.0.0"));
        }
    }

    #[tokio::test]
    async fn test_permissions_default() {
        let perms = ExtensionPermissions::default();
        assert!(perms.allow_read);
        assert!(!perms.allow_write);
        assert!(!perms.allow_net);
        assert!(!perms.allow_env);
        assert!(!perms.allow_run);
    }

    #[tokio::test]
    async fn test_extension_status_display() {
        assert_eq!(ExtensionStatus::Installed.to_string(), "installed");
        assert_eq!(ExtensionStatus::Running.to_string(), "running");
        assert_eq!(ExtensionStatus::Stopped.to_string(), "stopped");
        assert_eq!(ExtensionStatus::Error.to_string(), "error");
    }
}
