//! Channel-key SHA-256 auth + permission tiers.
use sha2::{Digest, Sha256};
use crate::protocol::PermissionTier;

/// Verify a client auth token.
/// Token = hex(SHA-256(channel_key + ":" + user_id + ":" + nonce))
pub fn verify_token(channel_key: &str, user_id: &str, nonce: &str, token: &str) -> bool {
    let expected = compute_token(channel_key, user_id, nonce);
    // Constant-time compare
    expected.len() == token.len() && expected.bytes().zip(token.bytes()).all(|(a, b)| a == b)
}

pub fn compute_token(channel_key: &str, user_id: &str, nonce: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(channel_key);
    hasher.update(b":");
    hasher.update(user_id);
    hasher.update(b":");
    hasher.update(nonce);
    hex::encode(hasher.finalize())
}

/// Determine permission tier from config.
pub fn resolve_tier(user_id: &str, admin_ids: &[String]) -> PermissionTier {
    if admin_ids.iter().any(|id| id == user_id) {
        PermissionTier::Admin
    } else {
        PermissionTier::Member
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_roundtrip() {
        let key = "supersecret";
        let user = "zeus107";
        let nonce = "abc123";
        let token = compute_token(key, user, nonce);
        assert!(verify_token(key, user, nonce, &token));
        assert!(!verify_token(key, user, "wrong_nonce", &token));
    }
}
