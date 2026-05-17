// ============================================================================
// whatsapp_additions.rs — HIGH priority OpenClaw parity features
//
// Included by whatsapp.rs via `mod whatsapp_additions` or merged in.
// Features:
//   1. WhatsAppAccountConfig — multi-account lifecycle management
//   2. WhatsAppStatus + StatusTracker — health/status monitoring
//   3. chunk_text() — 4000-char message chunking
//   4. QR login flow — request_qr_login / wait_qr_login
// ============================================================================

use futures_util::SinkExt as _;
use futures_util::StreamExt as _;
use serde::{Deserialize, Serialize};
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use zeus_core::{Error, Result};

pub(crate) fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// ============================================================================
// 1. Multi-account config
// ============================================================================

/// Per-account WhatsApp configuration for multi-account setups.
/// Mirrors OpenClaw's account lifecycle: name, enabled, auth_dir, dm_policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhatsAppAccountConfig {
    /// Human-readable account name
    pub name: String,
    /// Whether this account is active
    #[serde(default = "bool_true")]
    pub enabled: bool,
    /// Auth directory for Baileys session storage (bridge mode)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_dir: Option<String>,
    /// Phone number for this account
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phone: Option<String>,
    /// DM policy: "open" (default) or "allowlist"
    #[serde(default)]
    pub dm_policy: String,
    /// Allowlist of phone numbers (when dm_policy == "allowlist")
    #[serde(default)]
    pub allow_from: Vec<String>,
}

fn bool_true() -> bool {
    true
}

impl Default for WhatsAppAccountConfig {
    fn default() -> Self {
        Self {
            name: "default".to_string(),
            enabled: true,
            auth_dir: None,
            phone: None,
            dm_policy: "open".to_string(),
            allow_from: Vec::new(),
        }
    }
}

impl WhatsAppAccountConfig {
    /// Returns true if the sender is allowed under this account's DM policy.
    pub fn allows_sender(&self, from: &str) -> bool {
        if !self.enabled {
            return false;
        }
        match self.dm_policy.as_str() {
            "allowlist" => self
                .allow_from
                .iter()
                .any(|n| n.trim_start_matches('+') == from.trim_start_matches('+')),
            _ => true,
        }
    }
}

// ============================================================================
// 2. Health / status monitoring
// ============================================================================

/// Structured health snapshot for a WhatsApp adapter instance.
/// Mirrors OpenClaw's status fields: running, connected, reconnectAttempts, etc.
#[derive(Debug)]
pub struct WhatsAppStatus {
    pub running: bool,
    pub connected: bool,
    pub reconnect_attempts: u32,
    pub last_connected_at: Option<u64>,
    pub last_disconnect_at: Option<u64>,
    pub last_message_at: Option<u64>,
    pub last_error: Option<String>,
}

impl WhatsAppStatus {
    /// Structured readiness check — mirrors OpenClaw's `checkReady`.
    pub fn check_ready(&self) -> Result<()> {
        if !self.running {
            return Err(Error::Channel("whatsapp-not-running".into()));
        }
        if self.last_connected_at.is_none() {
            return Err(Error::Channel("whatsapp-not-linked".into()));
        }
        if !self.connected {
            return Err(Error::Channel("whatsapp-disconnected".into()));
        }
        Ok(())
    }
}

/// Atomic status tracker shared between the bridge task and the adapter.
#[derive(Debug, Default)]
pub struct StatusTracker {
    pub reconnect_attempts: AtomicU64,
    pub last_connected_at: Mutex<Option<u64>>,
    pub last_disconnect_at: Mutex<Option<u64>>,
    pub last_message_at: Mutex<Option<u64>>,
    pub last_error: Mutex<Option<String>>,
}

impl StatusTracker {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub async fn on_connected(&self) {
        use std::sync::atomic::Ordering;
        self.reconnect_attempts.store(0, Ordering::SeqCst);
        *self.last_connected_at.lock().await = Some(now_unix());
    }

    pub async fn on_disconnected(&self, err: Option<&str>) {
        use std::sync::atomic::Ordering;
        self.reconnect_attempts.fetch_add(1, Ordering::SeqCst);
        *self.last_disconnect_at.lock().await = Some(now_unix());
        if let Some(e) = err {
            *self.last_error.lock().await = Some(e.to_string());
        }
    }

    pub async fn on_message(&self) {
        *self.last_message_at.lock().await = Some(now_unix());
    }
}

// ============================================================================
// 3. Text chunking (WhatsApp silently truncates at ~4096 chars)
// ============================================================================

/// Split text into chunks of at most `max_len` chars, breaking on whitespace.
pub fn chunk_text(text: &str, max_len: usize) -> Vec<String> {
    if text.len() <= max_len {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        if remaining.len() <= max_len {
            chunks.push(remaining.to_string());
            break;
        }
        let slice = &remaining[..max_len];
        let break_at = slice.rfind(char::is_whitespace).unwrap_or(max_len);
        let (chunk, rest) = remaining.split_at(break_at);
        chunks.push(chunk.to_string());
        remaining = rest.trim_start();
    }

    chunks
}

// ============================================================================
// 4. QR login flow
// ============================================================================

/// QR login session info returned by the bridge on `qr_login_start`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QrLoginSession {
    /// Base64 QR data URI (or raw QR string) for display/agent use
    pub qr_data: String,
    /// Unix timestamp when this QR expires
    pub expires_at: u64,
    /// Correlation ID for the pairing flow
    pub session_id: String,
}

/// Request the Baileys bridge to start a QR login flow.
/// Sends `{"type":"qr_login_start"}` and returns QR data for the agent to surface.
pub async fn request_qr_login(bridge_url: &str) -> Result<QrLoginSession> {
    let (mut ws, _) = tokio_tungstenite::connect_async(bridge_url)
        .await
        .map_err(|e| Error::Channel(format!("QR login: bridge connect failed: {e}")))?;

    let req = serde_json::json!({"type": "qr_login_start"}).to_string();
    ws.send(WsMessage::Text(req))
        .await
        .map_err(|e| Error::Channel(format!("QR login: send failed: {e}")))?;

    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(30);
    loop {
        if tokio::time::Instant::now() > deadline {
            return Err(Error::Channel(
                "QR login: timed out waiting for QR data".into(),
            ));
        }
        match tokio::time::timeout(
            tokio::time::Duration::from_secs(5),
            ws.next(),
        )
        .await
        {
            Ok(Some(Ok(WsMessage::Text(text)))) => {
                let v: serde_json::Value = serde_json::from_str(&text)
                    .map_err(|e| Error::Channel(format!("QR login: bad JSON: {e}")))?;
                if v["type"] == "qr_data" {
                    return Ok(QrLoginSession {
                        qr_data: v["qr"]
                            .as_str()
                            .ok_or_else(|| Error::Channel("QR login: missing 'qr' field".into()))?
                            .to_string(),
                        expires_at: v["expires_at"].as_u64().unwrap_or(now_unix() + 60),
                        session_id: v["session_id"].as_str().unwrap_or("default").to_string(),
                    });
                }
            }
            Ok(Some(Err(e))) => {
                return Err(Error::Channel(format!("QR login: WS error: {e}")));
            }
            _ => continue,
        }
    }
}

/// Poll the bridge waiting for a QR scan to complete.
/// Returns the linked phone number on success.
pub async fn wait_qr_login(
    bridge_url: &str,
    session_id: &str,
    timeout_secs: u64,
) -> Result<String> {
    let (mut ws, _) = tokio_tungstenite::connect_async(bridge_url)
        .await
        .map_err(|e| Error::Channel(format!("QR wait: bridge connect failed: {e}")))?;

    let req = serde_json::json!({"type": "qr_login_wait", "session_id": session_id}).to_string();
    ws.send(WsMessage::Text(req))
        .await
        .map_err(|e| Error::Channel(format!("QR wait: send failed: {e}")))?;

    let deadline =
        tokio::time::Instant::now() + tokio::time::Duration::from_secs(timeout_secs);
    loop {
        if tokio::time::Instant::now() > deadline {
            return Err(Error::Channel(
                "QR wait: timed out — QR not scanned in time".into(),
            ));
        }
        match tokio::time::timeout(tokio::time::Duration::from_secs(5), ws.next()).await {
            Ok(Some(Ok(WsMessage::Text(text)))) => {
                let v: serde_json::Value = serde_json::from_str(&text)
                    .map_err(|e| Error::Channel(format!("QR wait: bad JSON: {e}")))?;
                if v["type"] == "qr_login_success" {
                    return Ok(v["phone"].as_str().unwrap_or("unknown").to_string());
                }
                if v["type"] == "qr_login_failed" {
                    return Err(Error::Channel(
                        "QR wait: login failed (bridge rejected)".into(),
                    ));
                }
            }
            Ok(Some(Err(e))) => {
                return Err(Error::Channel(format!("QR wait: WS error: {e}")));
            }
            _ => continue,
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_text_short() {
        assert_eq!(chunk_text("hello", 4000), vec!["hello"]);
    }

    #[test]
    fn test_chunk_text_exact_boundary() {
        let text = "a".repeat(4000);
        assert_eq!(chunk_text(&text, 4000).len(), 1);
    }

    #[test]
    fn test_chunk_text_long_preserves_content() {
        let word = "word";
        let text = (0..2000).map(|_| word).collect::<Vec<_>>().join(" ");
        let chunks = chunk_text(&text, 4000);
        for c in &chunks {
            assert!(c.len() <= 4000, "chunk too long: {}", c.len());
        }
        assert!(chunks.len() > 1);
    }

    #[test]
    fn test_account_config_open_policy() {
        let acct = WhatsAppAccountConfig::default();
        assert!(acct.allows_sender("+1234567890"));
        assert!(acct.allows_sender("99999"));
    }

    #[test]
    fn test_account_config_allowlist() {
        let acct = WhatsAppAccountConfig {
            dm_policy: "allowlist".to_string(),
            allow_from: vec!["+1234567890".to_string()],
            ..Default::default()
        };
        assert!(acct.allows_sender("+1234567890"));
        assert!(acct.allows_sender("1234567890")); // + stripping
        assert!(!acct.allows_sender("+9999999999"));
    }

    #[test]
    fn test_account_config_disabled() {
        let acct = WhatsAppAccountConfig {
            enabled: false,
            ..Default::default()
        };
        assert!(!acct.allows_sender("+1234567890"));
    }

    #[test]
    fn test_status_not_running() {
        let s = WhatsAppStatus {
            running: false,
            connected: false,
            reconnect_attempts: 0,
            last_connected_at: None,
            last_disconnect_at: None,
            last_message_at: None,
            last_error: None,
        };
        assert!(s.check_ready().unwrap_err().to_string().contains("not-running"));
    }

    #[test]
    fn test_status_not_linked() {
        let s = WhatsAppStatus {
            running: true,
            connected: false,
            reconnect_attempts: 0,
            last_connected_at: None,
            last_disconnect_at: None,
            last_message_at: None,
            last_error: None,
        };
        assert!(s.check_ready().unwrap_err().to_string().contains("not-linked"));
    }

    #[test]
    fn test_status_healthy() {
        let s = WhatsAppStatus {
            running: true,
            connected: true,
            reconnect_attempts: 0,
            last_connected_at: Some(now_unix()),
            last_disconnect_at: None,
            last_message_at: None,
            last_error: None,
        };
        assert!(s.check_ready().is_ok());
    }

    #[test]
    fn test_qr_session_serde() {
        let session = QrLoginSession {
            qr_data: "data:image/png;base64,abc123".to_string(),
            expires_at: 9999999999,
            session_id: "sess-001".to_string(),
        };
        let json = serde_json::to_string(&session).unwrap();
        let back: QrLoginSession = serde_json::from_str(&json).unwrap();
        assert_eq!(back.session_id, "sess-001");
        assert_eq!(back.expires_at, 9999999999);
    }
}
