//! Outbound Webhook Manager
//!
//! Sends HTTP POST notifications to configured webhook URLs when events occur.
//! Supports event type filtering, retry with exponential backoff, and JSON persistence.

use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::url_validator;

/// Event types that can trigger outbound webhooks
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum WebhookEventType {
    ToolCall,
    Message,
    Error,
    TaskComplete,
}

impl std::fmt::Display for WebhookEventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ToolCall => write!(f, "tool_call"),
            Self::Message => write!(f, "message"),
            Self::Error => write!(f, "error"),
            Self::TaskComplete => write!(f, "task_complete"),
        }
    }
}

/// A registered outbound webhook
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboundWebhook {
    pub id: String,
    pub url: String,
    pub events: Vec<WebhookEventType>,
    pub secret: Option<String>,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub last_triggered_at: Option<DateTime<Utc>>,
    pub failure_count: u32,
}

/// Payload sent to webhook endpoints
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookPayload {
    pub event: String,
    pub timestamp: String,
    pub data: Value,
}

/// Result of a webhook delivery attempt
#[derive(Debug, Clone, Serialize)]
pub struct DeliveryResult {
    pub webhook_id: String,
    pub success: bool,
    pub status_code: Option<u16>,
    pub attempts: u32,
    pub error: Option<String>,
}

/// Manages outbound webhook registrations and delivery
pub struct WebhookManager {
    store_path: PathBuf,
    webhooks: RwLock<Vec<OutboundWebhook>>,
    client: Client,
}

impl WebhookManager {
    /// Create a new WebhookManager persisting to the given directory.
    pub fn new(workspace_dir: &Path) -> Self {
        let store_path = workspace_dir.join("webhooks_outbound.json");
        let webhooks = if store_path.exists() {
            match std::fs::read_to_string(&store_path) {
                Ok(data) => serde_json::from_str(&data).unwrap_or_default(),
                Err(_) => Vec::new(),
            }
        } else {
            Vec::new()
        };

        Self {
            store_path,
            webhooks: RwLock::new(webhooks),
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
        }
    }

    /// Save webhooks to disk.
    async fn persist(&self) -> Result<(), String> {
        let hooks = self.webhooks.read().await;
        let data =
            serde_json::to_string_pretty(&*hooks).map_err(|e| format!("serialize error: {}", e))?;
        if let Some(parent) = self.store_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("mkdir error: {}", e))?;
        }
        tokio::fs::write(&self.store_path, data)
            .await
            .map_err(|e| format!("write error: {}", e))?;
        Ok(())
    }

    /// List all registered webhooks.
    pub async fn list(&self) -> Vec<OutboundWebhook> {
        self.webhooks.read().await.clone()
    }

    /// Get a webhook by ID.
    pub async fn get(&self, id: &str) -> Option<OutboundWebhook> {
        self.webhooks
            .read()
            .await
            .iter()
            .find(|w| w.id == id)
            .cloned()
    }

    /// Register a new outbound webhook.
    pub async fn register(
        &self,
        url: String,
        events: Vec<WebhookEventType>,
        secret: Option<String>,
    ) -> Result<OutboundWebhook, String> {
        if url.is_empty() {
            return Err("url is required".to_string());
        }

        // SSRF protection: validate webhook URL before registration
        url_validator::validate_url(&url)
            .map_err(|e| format!("Invalid webhook URL (SSRF protection): {}", e))?;

        if events.is_empty() {
            return Err("at least one event type is required".to_string());
        }

        let hook = OutboundWebhook {
            id: uuid::Uuid::new_v4().to_string(),
            url,
            events,
            secret,
            enabled: true,
            created_at: Utc::now(),
            last_triggered_at: None,
            failure_count: 0,
        };

        {
            let mut hooks = self.webhooks.write().await;
            hooks.push(hook.clone());
        }
        self.persist().await?;

        info!("Registered outbound webhook {} -> {}", hook.id, hook.url);
        Ok(hook)
    }

    /// Delete a webhook by ID.
    pub async fn delete(&self, id: &str) -> Result<(), String> {
        {
            let mut hooks = self.webhooks.write().await;
            let before = hooks.len();
            hooks.retain(|w| w.id != id);
            if hooks.len() == before {
                return Err(format!("webhook '{}' not found", id));
            }
        }
        self.persist().await?;
        info!("Deleted outbound webhook {}", id);
        Ok(())
    }

    /// Fire an event to all matching webhooks.
    /// Returns delivery results for each webhook attempted.
    pub async fn fire_event(
        &self,
        event_type: WebhookEventType,
        data: Value,
    ) -> Vec<DeliveryResult> {
        let matching: Vec<OutboundWebhook> = {
            let hooks = self.webhooks.read().await;
            hooks
                .iter()
                .filter(|w| w.enabled && w.events.contains(&event_type))
                .cloned()
                .collect()
        };

        if matching.is_empty() {
            return Vec::new();
        }

        let payload = WebhookPayload {
            event: event_type.to_string(),
            timestamp: Utc::now().to_rfc3339(),
            data,
        };

        let mut results = Vec::new();
        for hook in &matching {
            let result = self.deliver(&payload, hook).await;
            // Update last_triggered_at and failure_count
            {
                let mut hooks = self.webhooks.write().await;
                if let Some(h) = hooks.iter_mut().find(|h| h.id == hook.id) {
                    h.last_triggered_at = Some(Utc::now());
                    if result.success {
                        h.failure_count = 0;
                    } else {
                        h.failure_count += 1;
                    }
                }
            }
            results.push(result);
        }

        // Best-effort persist (don't fail the fire)
        let _ = self.persist().await;
        results
    }

    /// Deliver a payload to a single webhook with retry (3 attempts, exponential backoff).
    async fn deliver(&self, payload: &WebhookPayload, hook: &OutboundWebhook) -> DeliveryResult {
        let max_attempts: u32 = 3;
        let mut last_error = None;
        let mut last_status = None;

        for attempt in 0..max_attempts {
            if attempt > 0 {
                let delay = std::time::Duration::from_millis(500 * 2u64.pow(attempt - 1));
                debug!(
                    "Webhook {} retry attempt {} after {:?}",
                    hook.id,
                    attempt + 1,
                    delay
                );
                tokio::time::sleep(delay).await;
            }

            let mut req = self.client.post(&hook.url).json(payload);
            if let Some(ref secret) = hook.secret {
                req = req.header("X-Webhook-Secret", secret.as_str());
            }
            req = req.header("X-Zeus-Event", payload.event.as_str());

            match req.send().await {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    last_status = Some(status);
                    if resp.status().is_success() {
                        debug!("Webhook {} delivered successfully ({})", hook.id, status);
                        return DeliveryResult {
                            webhook_id: hook.id.clone(),
                            success: true,
                            status_code: Some(status),
                            attempts: attempt + 1,
                            error: None,
                        };
                    }
                    last_error = Some(format!("HTTP {}", status));
                    warn!(
                        "Webhook {} attempt {} failed: HTTP {}",
                        hook.id,
                        attempt + 1,
                        status
                    );
                }
                Err(e) => {
                    last_error = Some(e.to_string());
                    warn!("Webhook {} attempt {} failed: {}", hook.id, attempt + 1, e);
                }
            }
        }

        DeliveryResult {
            webhook_id: hook.id.clone(),
            success: false,
            status_code: last_status,
            attempts: max_attempts,
            error: last_error,
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_register_and_list() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mgr = WebhookManager::new(tmp.path());

        let hook = mgr
            .register(
                "https://example.com/hook".to_string(),
                vec![WebhookEventType::Message, WebhookEventType::Error],
                None,
            )
            .await
            .unwrap();

        assert!(!hook.id.is_empty());
        assert_eq!(hook.url, "https://example.com/hook");
        assert_eq!(hook.events.len(), 2);
        assert!(hook.enabled);

        let hooks = mgr.list().await;
        assert_eq!(hooks.len(), 1);
        assert_eq!(hooks[0].id, hook.id);
    }

    #[tokio::test]
    async fn test_register_validates_url() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mgr = WebhookManager::new(tmp.path());

        let result = mgr
            .register(String::new(), vec![WebhookEventType::Message], None)
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("url is required"));
    }

    #[tokio::test]
    async fn test_register_validates_events() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mgr = WebhookManager::new(tmp.path());

        let result = mgr
            .register("https://example.com/hook".to_string(), vec![], None)
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("event type"));
    }

    #[tokio::test]
    async fn test_delete() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mgr = WebhookManager::new(tmp.path());

        let hook = mgr
            .register(
                "https://example.com/hook".to_string(),
                vec![WebhookEventType::ToolCall],
                None,
            )
            .await
            .unwrap();

        mgr.delete(&hook.id).await.unwrap();
        assert!(mgr.list().await.is_empty());
    }

    #[tokio::test]
    async fn test_delete_not_found() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mgr = WebhookManager::new(tmp.path());

        let result = mgr.delete("nonexistent").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[tokio::test]
    async fn test_persistence() {
        let tmp = tempfile::TempDir::new().unwrap();

        // Register a webhook
        {
            let mgr = WebhookManager::new(tmp.path());
            mgr.register(
                "https://example.com/hook".to_string(),
                vec![WebhookEventType::Error],
                Some("my-secret".to_string()),
            )
            .await
            .unwrap();
        }

        // Load from disk
        let mgr2 = WebhookManager::new(tmp.path());
        let hooks = mgr2.list().await;
        assert_eq!(hooks.len(), 1);
        assert_eq!(hooks[0].url, "https://example.com/hook");
        assert_eq!(hooks[0].secret.as_deref(), Some("my-secret"));
    }

    #[tokio::test]
    async fn test_fire_event_no_matching() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mgr = WebhookManager::new(tmp.path());

        mgr.register(
            "https://example.com/hook".to_string(),
            vec![WebhookEventType::Message],
            None,
        )
        .await
        .unwrap();

        // Fire a different event type — no match
        let results = mgr
            .fire_event(WebhookEventType::Error, json!({"msg": "test"}))
            .await;
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_fire_event_unreachable_url() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mgr = WebhookManager::new(tmp.path());

        // SSRF protection blocks localhost URLs, so use a valid but unreachable public URL
        mgr.register(
            "http://192.0.2.1:19999/nonexistent".to_string(), // RFC 5737 TEST-NET-1 (unreachable)
            vec![WebhookEventType::ToolCall],
            None,
        )
        .await
        .unwrap();

        let results = mgr
            .fire_event(WebhookEventType::ToolCall, json!({"tool": "shell"}))
            .await;
        assert_eq!(results.len(), 1);
        assert!(!results[0].success);
        assert_eq!(results[0].attempts, 3);
        assert!(results[0].error.is_some());
    }

    #[tokio::test]
    async fn test_event_type_serialization() {
        let types = vec![
            WebhookEventType::ToolCall,
            WebhookEventType::Message,
            WebhookEventType::Error,
            WebhookEventType::TaskComplete,
        ];
        let json = serde_json::to_string(&types).unwrap();
        assert!(json.contains("tool_call"));
        assert!(json.contains("message"));
        assert!(json.contains("error"));
        assert!(json.contains("task_complete"));

        let parsed: Vec<WebhookEventType> = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, types);
    }

    #[tokio::test]
    async fn test_get_webhook() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mgr = WebhookManager::new(tmp.path());

        let hook = mgr
            .register(
                "https://example.com/hook".to_string(),
                vec![WebhookEventType::Message],
                None,
            )
            .await
            .unwrap();

        let fetched = mgr.get(&hook.id).await;
        assert!(fetched.is_some());
        assert_eq!(fetched.unwrap().url, "https://example.com/hook");

        assert!(mgr.get("nonexistent").await.is_none());
    }
}
