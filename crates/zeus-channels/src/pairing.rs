//! DM Pairing / Channel Auth System
//!
//! Provides per-user channel pairing with 6-digit verification codes.
//! PairingManager tracks user-to-channel mappings with status lifecycle:
//! Pending → Verified (or Expired after TTL).

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use rand::Rng;
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use zeus_core::Result;

use crate::ChannelSource;

/// How long a pairing code stays valid before expiring (15 minutes)
const CODE_TTL: Duration = Duration::from_secs(15 * 60);

/// Maximum failed verification attempts before lockout
const MAX_VERIFY_ATTEMPTS: u32 = 5;

/// Lockout duration after exceeding max attempts (30 minutes)
const LOCKOUT_DURATION: Duration = Duration::from_secs(30 * 60);

/// Alphanumeric charset for pairing codes (no ambiguous chars: 0/O, 1/I/l)
const CODE_CHARSET: &[u8] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ";

/// Length of generated pairing codes (31^8 ≈ 852 billion combinations)
const CODE_LENGTH: usize = 8;

/// Status of a pairing request
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairingStatus {
    /// Code generated, waiting for user to verify
    Pending,
    /// Code verified, channel pairing active
    Verified,
    /// Code expired (TTL exceeded without verification)
    Expired,
}

/// Tracks failed verification attempts per IP/source for brute-force protection
struct VerifyAttemptTracker {
    attempts: u32,
    first_attempt: Instant,
}

/// A pairing request tracking a user-to-channel binding
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairingRequest {
    /// Pairing code (8-char alphanumeric, high entropy)
    pub code: String,
    /// Channel ID this pairing is for
    pub channel_id: String,
    /// User identifier requesting the pairing
    pub user_id: String,
    /// Channel type (e.g. "telegram", "discord")
    pub channel_type: String,
    /// Current pairing status
    pub status: PairingStatus,
    /// When the request was created
    pub created_at: DateTime<Utc>,
    /// When the request was verified (if applicable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verified_at: Option<DateTime<Utc>>,
    /// Monotonic instant for TTL checks (not serialized)
    #[serde(skip)]
    pub created_instant: Option<Instant>,
}

/// A verified user-to-channel pairing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifiedPairing {
    pub user_id: String,
    pub channel_id: String,
    pub channel_type: String,
    pub verified_at: DateTime<Utc>,
}

/// Thread-safe pairing manager using DashMap for concurrent access
#[derive(Clone)]
pub struct PairingManager {
    /// Pending pairing requests keyed by code
    pending: Arc<DashMap<String, PairingRequest>>,
    /// Verified pairings keyed by "user_id:channel_id"
    verified: Arc<DashMap<String, VerifiedPairing>>,
    /// Legacy approved contacts (for backward compat with old pairing flow)
    approved: Arc<DashMap<String, bool>>,
    /// Failed verification attempt tracker keyed by source identifier
    verify_attempts: Arc<DashMap<String, VerifyAttemptTracker>>,
    /// Persistence path for verified pairings
    persistence_path: PathBuf,
}

/// Format a ChannelSource into the key used in the approved set.
fn source_key(source: &ChannelSource) -> String {
    format!("{}:{}", source.channel_type, source.user_id)
}

fn pairing_key(user_id: &str, channel_id: &str) -> String {
    format!("{user_id}:{channel_id}")
}

impl PairingManager {
    /// Create a new PairingManager
    pub fn new(persistence_path: PathBuf) -> Self {
        Self {
            pending: Arc::new(DashMap::new()),
            verified: Arc::new(DashMap::new()),
            approved: Arc::new(DashMap::new()),
            verify_attempts: Arc::new(DashMap::new()),
            persistence_path,
        }
    }

    /// Load verified pairings from disk
    pub fn load(persistence_path: PathBuf) -> Result<Self> {
        let mgr = Self::new(persistence_path.clone());

        if persistence_path.exists() {
            let data = std::fs::read_to_string(&persistence_path)?;
            let pairings: Vec<VerifiedPairing> = serde_json::from_str(&data)
                .map_err(|e| zeus_core::Error::Channel(format!("Failed to parse pairings: {e}")))?;
            for p in pairings {
                let key = pairing_key(&p.user_id, &p.channel_id);
                // Also mark as approved for legacy compat
                mgr.approved
                    .insert(format!("{}:{}", p.channel_type, p.user_id), true);
                mgr.verified.insert(key, p);
            }
        }

        Ok(mgr)
    }

    /// Persist verified pairings to disk
    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.persistence_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let pairings: Vec<VerifiedPairing> = self
            .verified
            .iter()
            .map(|entry| entry.value().clone())
            .collect();
        let json = serde_json::to_string_pretty(&pairings)
            .map_err(|e| zeus_core::Error::Channel(format!("Failed to serialize pairings: {e}")))?;
        std::fs::write(&self.persistence_path, json)?;
        Ok(())
    }

    /// Generate a high-entropy pairing code for a user + channel combination.
    /// Uses OsRng (kernel CSPRNG) and 8-char alphanumeric codes (~40 bits entropy).
    pub fn generate_code(&self, channel_id: &str, user_id: &str, channel_type: &str) -> String {
        let mut rng = OsRng;
        let code: String = (0..CODE_LENGTH)
            .map(|_| {
                let idx = rng.gen_range(0..CODE_CHARSET.len());
                CODE_CHARSET[idx] as char
            })
            .collect();

        let request = PairingRequest {
            code: code.clone(),
            channel_id: channel_id.to_string(),
            user_id: user_id.to_string(),
            channel_type: channel_type.to_string(),
            status: PairingStatus::Pending,
            created_at: Utc::now(),
            verified_at: None,
            created_instant: Some(Instant::now()),
        };

        self.pending.insert(code.clone(), request);
        code
    }

    /// Check and record a verification attempt, enforcing rate limits.
    /// Returns Err if the source is locked out from too many failed attempts.
    fn check_rate_limit(&self, source: &str) -> Result<()> {
        if let Some(tracker) = self.verify_attempts.get(source)
            && tracker.attempts >= MAX_VERIFY_ATTEMPTS
        {
            if tracker.first_attempt.elapsed() < LOCKOUT_DURATION {
                return Err(zeus_core::Error::Channel(
                    "Too many failed verification attempts — try again later".to_string(),
                ));
            }
            // Lockout expired, reset
            drop(tracker);
            self.verify_attempts.remove(source);
        }
        Ok(())
    }

    /// Record a failed verification attempt for a source.
    fn record_failed_attempt(&self, source: &str) {
        self.verify_attempts
            .entry(source.to_string())
            .and_modify(|t| t.attempts += 1)
            .or_insert(VerifyAttemptTracker {
                attempts: 1,
                first_attempt: Instant::now(),
            });
    }

    /// Verify a pairing code, transitioning from Pending → Verified.
    /// Enforces global rate limiting to prevent brute-force attacks.
    pub fn verify_code(&self, code: &str) -> Result<VerifiedPairing> {
        // Global rate limit — all failed attempts count toward lockout
        let rate_key = "global_verify";
        self.check_rate_limit(rate_key)?;

        let request = self.pending.remove(code).map(|(_, v)| v).ok_or_else(|| {
            self.record_failed_attempt(rate_key);
            zeus_core::Error::Channel(format!("No pending pairing with code {code}"))
        })?;

        // Check expiry
        if let Some(created) = request.created_instant
            && created.elapsed() > CODE_TTL
        {
            return Err(zeus_core::Error::Channel(
                "Pairing code has expired".to_string(),
            ));
        }

        let pairing = VerifiedPairing {
            user_id: request.user_id.clone(),
            channel_id: request.channel_id.clone(),
            channel_type: request.channel_type.clone(),
            verified_at: Utc::now(),
        };

        let key = pairing_key(&pairing.user_id, &pairing.channel_id);
        self.verified.insert(key, pairing.clone());

        // Legacy compat
        self.approved.insert(
            format!("{}:{}", request.channel_type, request.user_id),
            true,
        );

        Ok(pairing)
    }

    /// Check the status of a pairing code
    pub fn check_status(&self, code: &str) -> PairingStatus {
        if let Some(entry) = self.pending.get(code) {
            if let Some(created) = entry.created_instant
                && created.elapsed() > CODE_TTL
            {
                return PairingStatus::Expired;
            }
            PairingStatus::Pending
        } else {
            // Could be verified or never existed — check verified map
            PairingStatus::Expired
        }
    }

    /// Check if a user is paired (verified) with a specific channel
    pub fn is_paired(&self, user_id: &str, channel_id: &str) -> bool {
        self.verified
            .contains_key(&pairing_key(user_id, channel_id))
    }

    /// Check whether a channel source has been approved (legacy compat)
    pub fn is_approved(&self, source: &ChannelSource) -> bool {
        self.approved.contains_key(&source_key(source))
    }

    /// List all verified pairings for a specific channel
    pub fn pairings_for_channel(&self, channel_id: &str) -> Vec<VerifiedPairing> {
        self.verified
            .iter()
            .filter(|entry| entry.value().channel_id == channel_id)
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// List all pending requests
    pub fn pending_requests(&self) -> Vec<PairingRequest> {
        self.pending
            .iter()
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Remove a verified pairing
    pub fn unpair(&self, user_id: &str, channel_id: &str) -> Result<()> {
        let key = pairing_key(user_id, channel_id);
        self.verified
            .remove(&key)
            .ok_or_else(|| zeus_core::Error::Channel(format!("No verified pairing for {key}")))?;
        Ok(())
    }

    /// Cleanup expired pending codes
    pub fn cleanup_expired(&self) -> usize {
        let before = self.pending.len();
        self.pending.retain(|_, req| {
            req.created_instant
                .map(|t| t.elapsed() <= CODE_TTL)
                .unwrap_or(true)
        });
        before - self.pending.len()
    }

    /// Return the full set of approved contact keys (legacy compat)
    pub fn approved_contacts(&self) -> HashSet<String> {
        self.approved.iter().map(|e| e.key().clone()).collect()
    }

    // Legacy compat: generate_code with ChannelSource
    pub fn generate_code_legacy(&self, source: &ChannelSource) -> String {
        self.generate_code("default", &source.user_id, &source.channel_type)
    }

    // Legacy compat: approve by code
    pub fn approve(&self, code: &str) -> Result<String> {
        let pairing = self.verify_code(code)?;
        Ok(format!(
            "Approved {}:{}",
            pairing.channel_type, pairing.user_id
        ))
    }

    // Legacy compat: reject by code
    pub fn reject(&self, code: &str) -> Result<String> {
        let request = self.pending.remove(code).map(|(_, v)| v).ok_or_else(|| {
            zeus_core::Error::Channel(format!("No pending request with code {code}"))
        })?;
        Ok(format!(
            "Rejected {}:{}",
            request.channel_type, request.user_id
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn dummy_path() -> PathBuf {
        PathBuf::from("/tmp/zeus_pairing_test_dummy.json")
    }

    #[test]
    fn test_generate_code_format() {
        let mgr = PairingManager::new(dummy_path());
        let code = mgr.generate_code("chan-1", "user123", "telegram");

        assert_eq!(code.len(), CODE_LENGTH);
        assert!(code.chars().all(|c| CODE_CHARSET.contains(&(c as u8))));
    }

    #[test]
    fn test_generate_code_uniqueness() {
        let mgr = PairingManager::new(dummy_path());
        let codes: Vec<String> = (0..100)
            .map(|i| mgr.generate_code("chan-1", &format!("user{i}"), "telegram"))
            .collect();
        let unique: HashSet<String> = codes.iter().cloned().collect();
        assert_eq!(unique.len(), 100, "100 codes should all be unique");
    }

    #[test]
    fn test_brute_force_lockout() {
        let mgr = PairingManager::new(dummy_path());
        // Exhaust attempts with invalid codes
        for _ in 0..MAX_VERIFY_ATTEMPTS {
            let _ = mgr.verify_code("XXXXXXXX");
        }
        // Next attempt should be locked out
        let result = mgr.verify_code("YYYYYYYY");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Too many failed"));
    }

    #[test]
    fn test_verify_code_success() {
        let mgr = PairingManager::new(dummy_path());
        let code = mgr.generate_code("chan-1", "alice", "telegram");

        let pairing = mgr.verify_code(&code).expect("should verify");
        assert_eq!(pairing.user_id, "alice");
        assert_eq!(pairing.channel_id, "chan-1");
        assert_eq!(pairing.channel_type, "telegram");
        assert!(mgr.is_paired("alice", "chan-1"));
    }

    #[test]
    fn test_verify_invalid_code_fails() {
        let mgr = PairingManager::new(dummy_path());
        assert!(mgr.verify_code("ZZZZZZZZ").is_err());
    }

    #[test]
    fn test_verify_consumes_code() {
        let mgr = PairingManager::new(dummy_path());
        let code = mgr.generate_code("chan-1", "bob", "discord");

        mgr.verify_code(&code).expect("first verify succeeds");
        assert!(mgr.verify_code(&code).is_err(), "second verify should fail");
    }

    #[test]
    fn test_pairing_status_lifecycle() {
        let mgr = PairingManager::new(dummy_path());
        let code = mgr.generate_code("chan-1", "carol", "slack");

        assert_eq!(mgr.check_status(&code), PairingStatus::Pending);

        mgr.verify_code(&code).expect("verify");

        // After verification, code is consumed — status returns Expired (not found)
        assert_eq!(mgr.check_status(&code), PairingStatus::Expired);
    }

    #[test]
    fn test_pairings_for_channel() {
        let mgr = PairingManager::new(dummy_path());

        let c1 = mgr.generate_code("chan-A", "user1", "telegram");
        let c2 = mgr.generate_code("chan-A", "user2", "telegram");
        let c3 = mgr.generate_code("chan-B", "user3", "discord");

        mgr.verify_code(&c1).unwrap();
        mgr.verify_code(&c2).unwrap();
        mgr.verify_code(&c3).unwrap();

        let chan_a = mgr.pairings_for_channel("chan-A");
        assert_eq!(chan_a.len(), 2);

        let chan_b = mgr.pairings_for_channel("chan-B");
        assert_eq!(chan_b.len(), 1);
    }

    #[test]
    fn test_unpair() {
        let mgr = PairingManager::new(dummy_path());
        let code = mgr.generate_code("chan-1", "dave", "telegram");
        mgr.verify_code(&code).unwrap();

        assert!(mgr.is_paired("dave", "chan-1"));
        mgr.unpair("dave", "chan-1").unwrap();
        assert!(!mgr.is_paired("dave", "chan-1"));
    }

    #[test]
    fn test_cleanup_expired() {
        let mgr = PairingManager::new(dummy_path());

        // Insert a request with an already-elapsed instant
        let req = PairingRequest {
            code: "ZZTEST99".to_string(),
            channel_id: "chan-1".to_string(),
            user_id: "expired_user".to_string(),
            channel_type: "telegram".to_string(),
            status: PairingStatus::Pending,
            created_at: Utc::now(),
            verified_at: None,
            created_instant: Some(Instant::now() - Duration::from_secs(20 * 60)),
        };
        mgr.pending.insert("ZZTEST99".to_string(), req);

        // Also add a fresh one
        mgr.generate_code("chan-2", "fresh_user", "telegram");

        assert_eq!(mgr.pending.len(), 2);
        let cleaned = mgr.cleanup_expired();
        assert_eq!(cleaned, 1);
        assert_eq!(mgr.pending.len(), 1);
    }

    #[test]
    fn test_legacy_compat_approve_reject() {
        let mgr = PairingManager::new(dummy_path());

        let source_a = ChannelSource::new("telegram", "alice");
        let source_b = ChannelSource::new("discord", "bob");

        let code_a = mgr.generate_code_legacy(&source_a);
        let code_b = mgr.generate_code_legacy(&source_b);

        let msg = mgr.approve(&code_a).expect("approve should succeed");
        assert!(msg.contains("telegram:alice"));

        let msg = mgr.reject(&code_b).expect("reject should succeed");
        assert!(msg.contains("discord:bob"));

        assert!(mgr.is_approved(&source_a));
        assert!(!mgr.is_approved(&source_b));
    }

    #[test]
    fn test_persistence_save_load() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("pairings.json");

        {
            let mgr = PairingManager::new(path.clone());
            let code = mgr.generate_code("chan-1", "persist_user", "email");
            mgr.verify_code(&code).unwrap();
            mgr.save().unwrap();
        }

        {
            let mgr = PairingManager::load(path).unwrap();
            assert!(mgr.is_paired("persist_user", "chan-1"));
            let source = ChannelSource::new("email", "persist_user");
            assert!(mgr.is_approved(&source));
        }
    }
}
