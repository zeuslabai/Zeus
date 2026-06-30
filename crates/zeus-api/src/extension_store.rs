//! Extension persistence — CRUD for installed extensions
//!
//! Persists extension configs to `extensions.json` in the workspace directory.
//! On load, runtime-only fields (status, logs) are reset since processes do not
//! survive a restart.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::fs;
use zeus_extensions::{Extension, ExtensionSource, ExtensionStatus};

// ============================================================================
// Request / Response types
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallExtensionRequest {
    pub name: String,
    pub source: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub extension_type: Option<String>,
    #[serde(default)]
    pub config: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateExtensionRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub enabled: Option<bool>,
    pub config: Option<HashMap<String, String>>,
    pub version: Option<String>,
}

// ============================================================================
// Persisted extension record
// ============================================================================

/// Stored representation of an extension. Wraps [`Extension`] from
/// `zeus-extensions` with API-layer metadata (description, extension_type,
/// enabled flag, arbitrary config).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionRecord {
    /// Core extension data (id, name, version, source, permissions, etc.)
    #[serde(flatten)]
    pub inner: Extension,
    /// Human-readable description.
    #[serde(default)]
    pub description: String,
    /// Extension category: "skill", "mcp", "plugin", "deno", etc.
    #[serde(default = "default_extension_type")]
    pub extension_type: String,
    /// Whether the extension should be active (user-facing on/off toggle).
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// When the extension was installed through the API.
    pub installed_at: DateTime<Utc>,
    /// Arbitrary key-value configuration.
    #[serde(default)]
    pub config: HashMap<String, String>,
}

fn default_true() -> bool {
    true
}

fn default_extension_type() -> String {
    "plugin".to_string()
}

// ============================================================================
// ExtensionStore — JSON file persistence
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ExtensionStoreData {
    extensions: Vec<ExtensionRecord>,
}

pub struct ExtensionStore {
    path: PathBuf,
}

impl ExtensionStore {
    pub fn new(workspace_dir: impl AsRef<Path>) -> Self {
        Self {
            path: workspace_dir.as_ref().join("extensions.json"),
        }
    }

    async fn load(&self) -> ExtensionStoreData {
        if !self.path.exists() {
            return ExtensionStoreData::default();
        }
        match fs::read_to_string(&self.path).await {
            Ok(content) => {
                let mut data: ExtensionStoreData =
                    serde_json::from_str(&content).unwrap_or_default();
                // Reset runtime fields on load — processes don't survive restart
                for ext in &mut data.extensions {
                    ext.inner.status = ExtensionStatus::Stopped;
                    ext.inner.logs.clear();
                }
                data
            }
            Err(_) => ExtensionStoreData::default(),
        }
    }

    async fn save(&self, data: &ExtensionStoreData) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_string_pretty(data).map_err(|e| e.to_string())?;
        fs::write(&self.path, json).await.map_err(|e| e.to_string())
    }

    /// List all installed extensions.
    pub async fn list(&self) -> Vec<ExtensionRecord> {
        self.load().await.extensions
    }

    /// Get an extension by ID.
    pub async fn get(&self, id: &str) -> Option<ExtensionRecord> {
        self.load()
            .await
            .extensions
            .into_iter()
            .find(|e| e.inner.id == id)
    }

    /// Install (create) a new extension and persist to disk.
    pub async fn install(&self, req: InstallExtensionRequest) -> Result<ExtensionRecord, String> {
        let mut data = self.load().await;

        // Determine source variant
        let source = if req.source.starts_with("http://") || req.source.starts_with("https://") {
            ExtensionSource::Url(req.source.clone())
        } else if req.source.starts_with("openclaw:") {
            ExtensionSource::OpenClaw(req.source.trim_start_matches("openclaw:").to_string())
        } else {
            ExtensionSource::Local(req.source.clone())
        };

        let version = req.version.clone().unwrap_or_else(|| "0.1.0".to_string());
        let ext = Extension::new(&req.name, source).with_version(version);

        let record = ExtensionRecord {
            inner: ext,
            description: req.description.unwrap_or_default(),
            extension_type: req.extension_type.unwrap_or_else(|| "plugin".to_string()),
            enabled: true,
            installed_at: Utc::now(),
            config: req.config,
        };

        data.extensions.push(record.clone());
        self.save(&data).await?;
        Ok(record)
    }

    /// Update an extension by ID. Only the provided fields are changed.
    pub async fn update(
        &self,
        id: &str,
        req: UpdateExtensionRequest,
    ) -> Result<ExtensionRecord, String> {
        let mut data = self.load().await;

        let record = data
            .extensions
            .iter_mut()
            .find(|e| e.inner.id == id)
            .ok_or_else(|| format!("Extension not found: {}", id))?;

        if let Some(name) = req.name {
            record.inner.name = name;
        }
        if let Some(description) = req.description {
            record.description = description;
        }
        if let Some(enabled) = req.enabled {
            record.enabled = enabled;
        }
        if let Some(config) = req.config {
            record.config = config;
        }
        if let Some(version) = req.version {
            record.inner.version = version;
        }

        let updated = record.clone();
        self.save(&data).await?;
        Ok(updated)
    }

    /// Delete an extension by ID.
    pub async fn delete(&self, id: &str) -> Result<(), String> {
        let mut data = self.load().await;
        let before = data.extensions.len();
        data.extensions.retain(|e| e.inner.id != id);
        if data.extensions.len() == before {
            return Err(format!("Extension not found: {}", id));
        }
        self.save(&data).await
    }

    /// Set an extension's enabled flag to true and persist.
    pub async fn start(&self, id: &str) -> Result<ExtensionRecord, String> {
        let mut data = self.load().await;

        let record = data
            .extensions
            .iter_mut()
            .find(|e| e.inner.id == id)
            .ok_or_else(|| format!("Extension not found: {}", id))?;

        record.enabled = true;
        record.inner.status = ExtensionStatus::Running;

        let updated = record.clone();
        self.save(&data).await?;
        Ok(updated)
    }

    /// Set an extension's enabled flag to false and persist.
    pub async fn stop(&self, id: &str) -> Result<ExtensionRecord, String> {
        let mut data = self.load().await;

        let record = data
            .extensions
            .iter_mut()
            .find(|e| e.inner.id == id)
            .ok_or_else(|| format!("Extension not found: {}", id))?;

        record.enabled = false;
        record.inner.status = ExtensionStatus::Stopped;

        let updated = record.clone();
        self.save(&data).await?;
        Ok(updated)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_extension_store_crud() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = ExtensionStore::new(tmp.path());

        // List empty
        assert!(store.list().await.is_empty());

        // Install
        let ext = store
            .install(InstallExtensionRequest {
                name: "my-ext".to_string(),
                source: "https://github.com/example/ext".to_string(),
                description: Some("A test extension".to_string()),
                version: Some("1.0.0".to_string()),
                extension_type: Some("skill".to_string()),
                config: HashMap::from([("key".to_string(), "value".to_string())]),
            })
            .await
            .unwrap();
        assert_eq!(ext.inner.name, "my-ext");
        assert_eq!(ext.description, "A test extension");
        assert_eq!(ext.inner.version, "1.0.0");
        assert_eq!(ext.extension_type, "skill");
        assert!(ext.enabled);
        assert_eq!(ext.config["key"], "value");

        // List
        let all = store.list().await;
        assert_eq!(all.len(), 1);

        // Get
        let fetched = store.get(&ext.inner.id).await.unwrap();
        assert_eq!(fetched.inner.name, "my-ext");

        // Update
        let updated = store
            .update(
                &ext.inner.id,
                UpdateExtensionRequest {
                    name: Some("renamed-ext".to_string()),
                    description: None,
                    enabled: Some(false),
                    config: None,
                    version: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(updated.inner.name, "renamed-ext");
        assert!(!updated.enabled);

        // Delete
        store.delete(&ext.inner.id).await.unwrap();
        assert!(store.list().await.is_empty());
    }

    #[tokio::test]
    async fn test_extension_store_get_not_found() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = ExtensionStore::new(tmp.path());
        assert!(store.get("nonexistent").await.is_none());
    }

    #[tokio::test]
    async fn test_extension_store_delete_not_found() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = ExtensionStore::new(tmp.path());
        assert!(store.delete("nonexistent").await.is_err());
    }

    #[tokio::test]
    async fn test_extension_store_persistence() {
        let tmp = tempfile::TempDir::new().unwrap();

        // Install with one store instance
        let store = ExtensionStore::new(tmp.path());
        let ext = store
            .install(InstallExtensionRequest {
                name: "persist-test".to_string(),
                source: "/path/to/ext.ts".to_string(),
                description: None,
                version: None,
                extension_type: None,
                config: HashMap::new(),
            })
            .await
            .unwrap();

        // Read with a new store instance (simulates restart)
        let store2 = ExtensionStore::new(tmp.path());
        let all = store2.list().await;
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].inner.name, "persist-test");
        // Status should be reset to Stopped on reload
        assert_eq!(all[0].inner.status, ExtensionStatus::Stopped);
    }

    #[tokio::test]
    async fn test_extension_store_start_stop() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = ExtensionStore::new(tmp.path());

        let ext = store
            .install(InstallExtensionRequest {
                name: "toggle-ext".to_string(),
                source: "https://example.com/ext".to_string(),
                description: None,
                version: None,
                extension_type: None,
                config: HashMap::new(),
            })
            .await
            .unwrap();

        // Start
        let started = store.start(&ext.inner.id).await.unwrap();
        assert!(started.enabled);
        assert_eq!(started.inner.status, ExtensionStatus::Running);

        // Stop
        let stopped = store.stop(&ext.inner.id).await.unwrap();
        assert!(!stopped.enabled);
        assert_eq!(stopped.inner.status, ExtensionStatus::Stopped);
    }

    #[tokio::test]
    async fn test_extension_store_start_not_found() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = ExtensionStore::new(tmp.path());
        assert!(store.start("nonexistent").await.is_err());
    }

    #[tokio::test]
    async fn test_extension_store_stop_not_found() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = ExtensionStore::new(tmp.path());
        assert!(store.stop("nonexistent").await.is_err());
    }

    #[tokio::test]
    async fn test_extension_source_classification() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = ExtensionStore::new(tmp.path());

        // URL source
        let ext_url = store
            .install(InstallExtensionRequest {
                name: "url-ext".to_string(),
                source: "https://github.com/foo/bar".to_string(),
                description: None,
                version: None,
                extension_type: None,
                config: HashMap::new(),
            })
            .await
            .unwrap();
        assert!(matches!(ext_url.inner.source, ExtensionSource::Url(_)));

        // OpenClaw source
        let ext_oc = store
            .install(InstallExtensionRequest {
                name: "openclaw-ext".to_string(),
                source: "openclaw:my-package".to_string(),
                description: None,
                version: None,
                extension_type: None,
                config: HashMap::new(),
            })
            .await
            .unwrap();
        assert!(matches!(ext_oc.inner.source, ExtensionSource::OpenClaw(_)));

        // Local source (default)
        let ext_local = store
            .install(InstallExtensionRequest {
                name: "local-ext".to_string(),
                source: "/usr/local/lib/ext.ts".to_string(),
                description: None,
                version: None,
                extension_type: None,
                config: HashMap::new(),
            })
            .await
            .unwrap();
        assert!(matches!(ext_local.inner.source, ExtensionSource::Local(_)));
    }

    #[tokio::test]
    async fn test_extension_update_not_found() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = ExtensionStore::new(tmp.path());
        let result = store
            .update(
                "nonexistent",
                UpdateExtensionRequest {
                    name: Some("test".to_string()),
                    description: None,
                    enabled: None,
                    config: None,
                    version: None,
                },
            )
            .await;
        assert!(result.is_err());
    }
}
