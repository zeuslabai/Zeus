//! Auth middleware with key rotation support
//!
//! Provides `KeyRotation` which accepts both the current key and the
//! previous key during a 24-hour grace period after rotation. This
//! ensures zero-downtime key rotation for all connected clients.

use crate::api_key::constant_time_eq;
use crate::SharedState;
use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Json, Response},
};
use ring::digest::{SHA256, digest};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// Duration (in seconds) that the previous key remains valid after rotation
const GRACE_PERIOD_SECS: u64 = 24 * 60 * 60; // 24 hours

/// Key rotation state — thread-safe, shareable across request handlers
#[derive(Clone)]
pub struct KeyRotation {
    inner: Arc<RwLock<KeyRotationInner>>,
}

struct KeyRotationInner {
    current_key: String,
    /// Previous key + when it was rotated out (for grace period)
    previous: Option<(String, u64)>,
    /// History of rotation events
    history: Vec<RotationEvent>,
}

/// Record of a key rotation event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RotationEvent {
    pub timestamp: u64,
    /// SHA256 hash of the old key (for audit, never store raw keys)
    pub old_key_hash: String,
    /// SHA256 hash of the new key
    pub new_key_hash: String,
    pub reason: Option<String>,
}

/// Response from the rotate-key endpoint
#[derive(Debug, Serialize, Deserialize)]
pub struct RotateKeyResponse {
    pub success: bool,
    pub message: String,
    pub grace_period_hours: u64,
    pub new_key_hash: String,
}

/// Request body for the rotate-key endpoint
#[derive(Debug, Deserialize)]
pub struct RotateKeyRequest {
    pub new_key: String,
    pub reason: Option<String>,
}

/// Response for rotation status
#[derive(Debug, Serialize)]
pub struct RotationStatusResponse {
    pub current_key_hash: String,
    pub previous_key_active: bool,
    pub grace_period_remaining_secs: Option<u64>,
    pub total_rotations: usize,
    pub last_rotation: Option<RotationEvent>,
}

impl KeyRotation {
    /// Create a new KeyRotation with the initial key
    pub fn new(initial_key: String) -> Self {
        Self {
            inner: Arc::new(RwLock::new(KeyRotationInner {
                current_key: initial_key,
                previous: None,
                history: Vec::new(),
            })),
        }
    }

    /// Rotate to a new key. The old key remains valid for 24 hours.
    pub fn rotate(&self, new_key: String, reason: Option<String>) -> Result<(), String> {
        let mut inner = self
            .inner
            .write()
            .map_err(|e| format!("Lock error: {}", e))?;

        if new_key == inner.current_key {
            return Err("New key must be different from current key".to_string());
        }

        if new_key.len() < 16 {
            return Err("New key must be at least 16 characters".to_string());
        }

        let now = now_epoch();
        let old_key = std::mem::replace(&mut inner.current_key, new_key.clone());

        // Record rotation event
        let event = RotationEvent {
            timestamp: now,
            old_key_hash: hash_key(&old_key),
            new_key_hash: hash_key(&new_key),
            reason,
        };
        inner.history.push(event);

        // Set old key as previous with grace period
        inner.previous = Some((old_key, now));

        Ok(())
    }

    /// Check if a given token matches the current or grace-period previous key
    pub fn verify(&self, token: &str) -> bool {
        let inner = self.inner.read().unwrap_or_else(|p| p.into_inner());

        // Check current key
        if constant_time_eq(token, &inner.current_key) {
            return true;
        }

        // Check previous key within grace period
        if let Some((ref prev_key, rotated_at)) = inner.previous {
            let now = now_epoch();
            if now - rotated_at <= GRACE_PERIOD_SECS && constant_time_eq(token, prev_key) {
                return true;
            }
        }

        false
    }

    /// Get rotation status
    pub fn status(&self) -> RotationStatusResponse {
        let inner = self.inner.read().unwrap_or_else(|p| p.into_inner());
        let now = now_epoch();

        let (previous_active, grace_remaining) = match &inner.previous {
            Some((_, rotated_at)) => {
                let elapsed = now.saturating_sub(*rotated_at);
                if elapsed <= GRACE_PERIOD_SECS {
                    (true, Some(GRACE_PERIOD_SECS - elapsed))
                } else {
                    (false, None)
                }
            }
            None => (false, None),
        };

        RotationStatusResponse {
            current_key_hash: hash_key(&inner.current_key),
            previous_key_active: previous_active,
            grace_period_remaining_secs: grace_remaining,
            total_rotations: inner.history.len(),
            last_rotation: inner.history.last().cloned(),
        }
    }

    /// Get rotation history
    pub fn history(&self) -> Vec<RotationEvent> {
        self.inner.read().unwrap_or_else(|p| p.into_inner()).history.clone()
    }
}

/// Auth middleware that supports key rotation with grace period
pub async fn auth_with_rotation(
    key_rotation: KeyRotation,
    req: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    let path = req.uri().path();

    // Public endpoints (no auth required)
    if path == "/" || path == "/health" {
        return Ok(next.run(req).await);
    }
    if path.starts_with("/v1/auth/") || path.starts_with("/v1/onboarding/") {
        return Ok(next.run(req).await);
    }

    // Extract Bearer token
    let auth_header = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());

    match auth_header {
        Some(value) if value.starts_with("Bearer ") => {
            let token = &value[7..];
            if key_rotation.verify(token) {
                Ok(next.run(req).await)
            } else {
                Err(StatusCode::UNAUTHORIZED)
            }
        }
        _ => Err(StatusCode::UNAUTHORIZED),
    }
}

/// Handler for POST /v1/security/rotate-key
pub async fn handle_rotate_key(
    State(state): State<SharedState>,
    Json(req): Json<RotateKeyRequest>,
) -> Response {
    let kr = {
        let s = state.read().await;
        match &s.key_rotation {
            Some(kr) => kr.clone(),
            None => {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(RotateKeyResponse {
                        success: false,
                        message: "Key rotation not configured".to_string(),
                        grace_period_hours: 0,
                        new_key_hash: String::new(),
                    }),
                )
                    .into_response();
            }
        }
    };
    match kr.rotate(req.new_key.clone(), req.reason) {
        Ok(()) => {
            let resp = RotateKeyResponse {
                success: true,
                message: "Key rotated successfully. Previous key valid for 24 hours.".to_string(),
                grace_period_hours: 24,
                new_key_hash: hash_key(&req.new_key),
            };
            (StatusCode::OK, Json(resp)).into_response()
        }
        Err(e) => {
            let resp = RotateKeyResponse {
                success: false,
                message: e,
                grace_period_hours: 0,
                new_key_hash: String::new(),
            };
            (StatusCode::BAD_REQUEST, Json(resp)).into_response()
        }
    }
}

/// Handler for GET /v1/security/rotation-status
pub async fn handle_rotation_status(State(state): State<SharedState>) -> Response {
    let kr = {
        let s = state.read().await;
        match &s.key_rotation {
            Some(kr) => kr.clone(),
            None => {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(serde_json::json!({
                        "error": "Key rotation not configured"
                    })),
                )
                    .into_response();
            }
        }
    };
    Json(kr.status()).into_response()
}



/// Hash a key for safe logging/audit (never store raw keys)
fn hash_key(key: &str) -> String {
    let d = digest(&SHA256, key.as_bytes());
    hex::encode(&d.as_ref()[..8]) // First 8 bytes = 16 hex chars
}

fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_rotation_basic() {
        let kr = KeyRotation::new("initial-key-1234567890".to_string());
        assert!(kr.verify("initial-key-1234567890"));
        assert!(!kr.verify("wrong-key"));
    }

    #[test]
    fn test_rotate_accepts_old_key() {
        let kr = KeyRotation::new("old-key-1234567890abcdef".to_string());
        kr.rotate(
            "new-key-1234567890abcdef".to_string(),
            Some("test".to_string()),
        )
        .unwrap();

        // Both keys should work during grace period
        assert!(kr.verify("new-key-1234567890abcdef"));
        assert!(kr.verify("old-key-1234567890abcdef"));
        assert!(!kr.verify("wrong-key"));
    }

    #[test]
    fn test_rotate_rejects_same_key() {
        let kr = KeyRotation::new("same-key-1234567890abcd".to_string());
        let result = kr.rotate("same-key-1234567890abcd".to_string(), None);
        assert!(result.is_err());
    }

    #[test]
    fn test_rotate_rejects_short_key() {
        let kr = KeyRotation::new("initial-key-1234567890".to_string());
        let result = kr.rotate("short".to_string(), None);
        assert!(result.is_err());
    }

    #[test]
    fn test_rotation_status() {
        let kr = KeyRotation::new("initial-key-1234567890".to_string());
        let status = kr.status();
        assert_eq!(status.total_rotations, 0);
        assert!(!status.previous_key_active);

        kr.rotate("rotated-key-1234567890a".to_string(), None)
            .unwrap();
        let status = kr.status();
        assert_eq!(status.total_rotations, 1);
        assert!(status.previous_key_active);
        assert!(status.grace_period_remaining_secs.is_some());
    }

    #[test]
    fn test_rotation_history() {
        let kr = KeyRotation::new("key-one-1234567890abcde".to_string());
        kr.rotate(
            "key-two-1234567890abcde".to_string(),
            Some("scheduled".to_string()),
        )
        .unwrap();
        kr.rotate(
            "key-three-1234567890abc".to_string(),
            Some("compromise".to_string()),
        )
        .unwrap();

        let history = kr.history();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].reason, Some("scheduled".to_string()));
        assert_eq!(history[1].reason, Some("compromise".to_string()));
    }

    #[test]
    fn test_constant_time_eq() {
        assert!(constant_time_eq("hello", "hello"));
        assert!(!constant_time_eq("hello", "world"));
        assert!(!constant_time_eq("hello", "hell"));
    }

    #[test]
    fn test_hash_key_deterministic() {
        let h1 = hash_key("test-key");
        let h2 = hash_key("test-key");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 16); // 8 bytes = 16 hex chars
    }

    #[test]
    fn test_double_rotate_previous_only_last() {
        let kr = KeyRotation::new("key-aaa-1234567890abcde".to_string());
        kr.rotate("key-bbb-1234567890abcde".to_string(), None)
            .unwrap();
        kr.rotate("key-ccc-1234567890abcde".to_string(), None)
            .unwrap();

        // Current (ccc) and previous (bbb) should work
        assert!(kr.verify("key-ccc-1234567890abcde"));
        assert!(kr.verify("key-bbb-1234567890abcde"));
        // Two rotations ago (aaa) should NOT work
        assert!(!kr.verify("key-aaa-1234567890abcde"));
    }
}
