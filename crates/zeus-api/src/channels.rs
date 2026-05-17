//! Channel management — CRUD for messaging channel configurations
//!
//! Persists channel configs to `channels.json` in the workspace directory.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::fs;
use uuid::Uuid;

// ============================================================================
// Types
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChannelType {
    Telegram,
    Discord,
    Slack,
    Webhook,
}

impl std::fmt::Display for ChannelType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Telegram => write!(f, "telegram"),
            Self::Discord => write!(f, "discord"),
            Self::Slack => write!(f, "slack"),
            Self::Webhook => write!(f, "webhook"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Channel {
    pub id: String,
    pub channel_type: ChannelType,
    pub name: String,
    /// Arbitrary key-value config (bot_token, chat_id, webhook_url, etc.)
    pub config: HashMap<String, String>,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub last_message_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateChannelRequest {
    pub channel_type: ChannelType,
    pub name: String,
    #[serde(default)]
    pub config: HashMap<String, String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateChannelRequest {
    pub name: Option<String>,
    pub config: Option<HashMap<String, String>>,
    pub enabled: Option<bool>,
}

// ============================================================================
// ChannelStore — JSON file persistence
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ChannelStoreData {
    channels: Vec<Channel>,
}

pub struct ChannelStore {
    path: PathBuf,
}

impl ChannelStore {
    pub fn new(workspace_dir: impl AsRef<Path>) -> Self {
        Self {
            path: workspace_dir.as_ref().join("channels.json"),
        }
    }

    async fn load(&self) -> ChannelStoreData {
        if !self.path.exists() {
            return ChannelStoreData::default();
        }
        match fs::read_to_string(&self.path).await {
            Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
            Err(_) => ChannelStoreData::default(),
        }
    }

    async fn save(&self, data: &ChannelStoreData) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_string_pretty(data).map_err(|e| e.to_string())?;
        fs::write(&self.path, json).await.map_err(|e| e.to_string())
    }

    pub async fn list(&self) -> Vec<Channel> {
        self.load().await.channels
    }

    /// Add a channel entry (used to seed from config.toml on boot)
    pub async fn add(&self, channel: Channel) -> Result<(), String> {
        let mut data = self.load().await;
        // Don't duplicate — skip if ID already exists
        if data.channels.iter().any(|c| c.id == channel.id) {
            return Ok(());
        }
        data.channels.push(channel);
        self.save(&data).await
    }

    pub async fn get(&self, id: &str) -> Option<Channel> {
        self.load().await.channels.into_iter().find(|c| c.id == id)
    }

    pub async fn create(&self, req: CreateChannelRequest) -> Result<Channel, String> {
        let mut data = self.load().await;

        let channel = Channel {
            id: Uuid::new_v4().to_string(),
            channel_type: req.channel_type,
            name: req.name,
            config: req.config,
            enabled: req.enabled,
            created_at: Utc::now(),
            last_message_at: None,
        };

        data.channels.push(channel.clone());
        self.save(&data).await?;
        Ok(channel)
    }

    pub async fn update(&self, id: &str, req: UpdateChannelRequest) -> Result<Channel, String> {
        let mut data = self.load().await;

        let channel = data
            .channels
            .iter_mut()
            .find(|c| c.id == id)
            .ok_or_else(|| format!("Channel not found: {}", id))?;

        if let Some(name) = req.name {
            channel.name = name;
        }
        if let Some(config) = req.config {
            channel.config = config;
        }
        if let Some(enabled) = req.enabled {
            channel.enabled = enabled;
        }

        let updated = channel.clone();
        self.save(&data).await?;
        Ok(updated)
    }

    pub async fn delete(&self, id: &str) -> Result<(), String> {
        let mut data = self.load().await;
        let before = data.channels.len();
        data.channels.retain(|c| c.id != id);
        if data.channels.len() == before {
            return Err(format!("Channel not found: {}", id));
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

    #[tokio::test]
    async fn test_channel_store_crud() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = ChannelStore::new(tmp.path());

        // List empty
        assert!(store.list().await.is_empty());

        // Create
        let ch = store
            .create(CreateChannelRequest {
                channel_type: ChannelType::Telegram,
                name: "My Telegram".to_string(),
                config: HashMap::from([("bot_token".to_string(), "abc123".to_string())]),
                enabled: true,
            })
            .await
            .unwrap();
        assert_eq!(ch.channel_type, ChannelType::Telegram);
        assert_eq!(ch.name, "My Telegram");
        assert!(ch.enabled);

        // List
        let all = store.list().await;
        assert_eq!(all.len(), 1);

        // Get
        let fetched = store.get(&ch.id).await.unwrap();
        assert_eq!(fetched.name, "My Telegram");
        assert_eq!(fetched.config["bot_token"], "abc123");

        // Update
        let updated = store
            .update(
                &ch.id,
                UpdateChannelRequest {
                    name: Some("Renamed".to_string()),
                    config: None,
                    enabled: Some(false),
                },
            )
            .await
            .unwrap();
        assert_eq!(updated.name, "Renamed");
        assert!(!updated.enabled);

        // Delete
        store.delete(&ch.id).await.unwrap();
        assert!(store.list().await.is_empty());
    }

    #[tokio::test]
    async fn test_channel_store_get_not_found() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = ChannelStore::new(tmp.path());
        assert!(store.get("nonexistent").await.is_none());
    }

    #[tokio::test]
    async fn test_channel_store_delete_not_found() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = ChannelStore::new(tmp.path());
        assert!(store.delete("nonexistent").await.is_err());
    }

    #[tokio::test]
    async fn test_channel_store_persistence() {
        let tmp = tempfile::TempDir::new().unwrap();

        // Create with one store instance
        let store = ChannelStore::new(tmp.path());
        store
            .create(CreateChannelRequest {
                channel_type: ChannelType::Discord,
                name: "Discord Bot".to_string(),
                config: HashMap::new(),
                enabled: true,
            })
            .await
            .unwrap();

        // Read with a new store instance (simulates restart)
        let store2 = ChannelStore::new(tmp.path());
        let all = store2.list().await;
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].name, "Discord Bot");
    }

    #[tokio::test]
    async fn test_channel_type_serialization() {
        let ch = Channel {
            id: "test".to_string(),
            channel_type: ChannelType::Webhook,
            name: "Hook".to_string(),
            config: HashMap::new(),
            enabled: true,
            created_at: Utc::now(),
            last_message_at: None,
        };
        let json = serde_json::to_string(&ch).unwrap();
        assert!(json.contains("\"webhook\""));

        let parsed: Channel = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.channel_type, ChannelType::Webhook);
    }
}
