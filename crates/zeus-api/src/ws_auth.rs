//! WebSocket v3 Ed25519 challenge-response authentication.
//!
//! When `ws_auth.enabled = true` in config, the server runs a handshake
//! immediately after WebSocket upgrade:
//!
//! ```text
//! Server → Client: {"type":"auth_challenge","nonce":"<b64url-32B>","server_timestamp":<u64>,"version":"3"}
//! Client → Server: {"type":"auth_response","nonce":"<same>","timestamp":<u64>,"signature":"<hex-64B>"}
//!   signed payload = nonce_bytes(32) ++ timestamp_be(8) = 40 bytes
//! Server → Client: {"type":"auth_ok","version":"3","public_key_hex":"<hex>"} or {"type":"auth_failed","reason":"..."}
//! ```

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use ring::rand::SecureRandom;
use ring::signature::{self, Ed25519KeyPair, KeyPair};

/// Ed25519 key pair stored as raw bytes (Clone + Send + Sync safe).
#[derive(Clone)]
pub struct WsKeyPair {
    /// PKCS8 v2 document bytes (used to reconstruct ring's Ed25519KeyPair)
    #[cfg_attr(not(test), allow(dead_code))]
    pkcs8_bytes: Vec<u8>,
    /// Raw 32-byte public key
    public_key_bytes: Vec<u8>,
}

/// Server-side challenge state sent to the client.
pub struct ChallengeState {
    pub nonce_bytes: Vec<u8>,
    pub nonce_b64: String,
    pub issued_at: u64,
}

/// Authentication errors.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("timestamp outside tolerance window")]
    TimestampWindow,
    #[error("invalid signature")]
    InvalidSignature,
    #[error("bad encoding: {0}")]
    BadEncoding(String),
    #[error("nonce mismatch")]
    NonceMismatch,
}

impl WsKeyPair {
    /// Build from raw PKCS8 bytes.
    fn from_pkcs8(pkcs8: Vec<u8>) -> anyhow::Result<Self> {
        let kp = Ed25519KeyPair::from_pkcs8(&pkcs8)
            .map_err(|e| anyhow::anyhow!("invalid PKCS8 key: {e}"))?;
        let pub_bytes = kp.public_key().as_ref().to_vec();
        Ok(Self {
            pkcs8_bytes: pkcs8,
            public_key_bytes: pub_bytes,
        })
    }

    /// Reconstruct ring's Ed25519KeyPair (needed for signing in tests).
    #[cfg(test)]
    fn ring_keypair(&self) -> Ed25519KeyPair {
        Ed25519KeyPair::from_pkcs8(&self.pkcs8_bytes).expect("stored pkcs8 must be valid")
    }
}

/// Load an existing PKCS8 key from disk, or generate a new one and persist it.
pub fn load_or_generate(key_path: &Path) -> anyhow::Result<WsKeyPair> {
    if key_path.exists() {
        let bytes = std::fs::read(key_path)?;
        return WsKeyPair::from_pkcs8(bytes);
    }

    // Generate new key pair
    let rng = ring::rand::SystemRandom::new();
    let pkcs8 =
        Ed25519KeyPair::generate_pkcs8(&rng).map_err(|e| anyhow::anyhow!("keygen failed: {e}"))?;
    let pkcs8_bytes = pkcs8.as_ref().to_vec();

    // Ensure parent directory exists
    if let Some(parent) = key_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Write key file
    std::fs::write(key_path, &pkcs8_bytes)?;

    // chmod 600 on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(key_path, std::fs::Permissions::from_mode(0o600))?;
    }

    WsKeyPair::from_pkcs8(pkcs8_bytes)
}

/// Create a fresh 32-byte random challenge nonce.
pub fn new_challenge() -> anyhow::Result<ChallengeState> {
    let rng = ring::rand::SystemRandom::new();
    let mut nonce = vec![0u8; 32];
    rng.fill(&mut nonce)
        .map_err(|e| anyhow::anyhow!("RNG failed: {e}"))?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    use base64::Engine;
    let nonce_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&nonce);

    Ok(ChallengeState {
        nonce_bytes: nonce,
        nonce_b64,
        issued_at: now,
    })
}

/// Verify a client's auth response against a previously issued challenge.
///
/// `signature_hex` is the 64-byte Ed25519 signature encoded as 128-char hex.
/// The signed payload is `nonce_bytes(32) ++ client_timestamp_be(8)` = 40 bytes.
pub fn verify_response(
    keypair: &WsKeyPair,
    challenge: &ChallengeState,
    client_nonce_b64: &str,
    client_timestamp: u64,
    signature_hex: &str,
    tolerance_secs: u64,
) -> Result<(), AuthError> {
    // 1. Verify nonce matches
    if client_nonce_b64 != challenge.nonce_b64 {
        return Err(AuthError::NonceMismatch);
    }

    // 2. Check timestamp window: [now - tolerance, now + 5s]
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    if client_timestamp + tolerance_secs < now || client_timestamp > now + 5 {
        return Err(AuthError::TimestampWindow);
    }

    // 3. Decode hex signature
    let sig_bytes =
        hex::decode(signature_hex).map_err(|e| AuthError::BadEncoding(e.to_string()))?;

    // 4. Reconstruct signed payload: nonce(32) ++ timestamp_be(8)
    let mut payload = Vec::with_capacity(40);
    payload.extend_from_slice(&challenge.nonce_bytes);
    payload.extend_from_slice(&client_timestamp.to_be_bytes());

    // 5. Verify signature using the public key
    let peer_public_key =
        signature::UnparsedPublicKey::new(&signature::ED25519, &keypair.public_key_bytes);
    peer_public_key
        .verify(&payload, &sig_bytes)
        .map_err(|_| AuthError::InvalidSignature)?;

    Ok(())
}

/// Return the hex-encoded 32-byte public key.
pub fn public_key_hex(keypair: &WsKeyPair) -> String {
    hex::encode(&keypair.public_key_bytes)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_keypair() -> WsKeyPair {
        let rng = ring::rand::SystemRandom::new();
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
        WsKeyPair::from_pkcs8(pkcs8.as_ref().to_vec()).unwrap()
    }

    #[test]
    fn test_load_or_generate_creates_new_key() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.key");
        assert!(!path.exists());

        let kp = load_or_generate(&path).unwrap();
        assert!(path.exists());
        assert_eq!(kp.public_key_bytes.len(), 32);
    }

    #[test]
    fn test_load_or_generate_idempotent() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.key");

        let kp1 = load_or_generate(&path).unwrap();
        let kp2 = load_or_generate(&path).unwrap();
        assert_eq!(kp1.public_key_bytes, kp2.public_key_bytes);
        assert_eq!(kp1.pkcs8_bytes, kp2.pkcs8_bytes);
    }

    #[test]
    fn test_nonce_uniqueness() {
        let c1 = new_challenge().unwrap();
        let c2 = new_challenge().unwrap();
        assert_ne!(c1.nonce_bytes, c2.nonce_bytes);
        assert_ne!(c1.nonce_b64, c2.nonce_b64);
    }

    #[test]
    fn test_valid_signature_roundtrip() {
        let kp = test_keypair();
        let challenge = new_challenge().unwrap();

        // Client signs: nonce(32) ++ timestamp_be(8)
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let mut payload = Vec::with_capacity(40);
        payload.extend_from_slice(&challenge.nonce_bytes);
        payload.extend_from_slice(&now.to_be_bytes());

        let ring_kp = kp.ring_keypair();
        let sig = ring_kp.sign(&payload);
        let sig_hex = hex::encode(sig.as_ref());

        let result = verify_response(&kp, &challenge, &challenge.nonce_b64, now, &sig_hex, 30);
        assert!(result.is_ok());
    }

    #[test]
    fn test_wrong_signature_rejected() {
        let kp = test_keypair();
        let challenge = new_challenge().unwrap();

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Use a bogus 64-byte signature
        let bad_sig = hex::encode([0xABu8; 64]);

        let result = verify_response(&kp, &challenge, &challenge.nonce_b64, now, &bad_sig, 30);
        assert!(matches!(result, Err(AuthError::InvalidSignature)));
    }

    #[test]
    fn test_expired_timestamp_rejected() {
        let kp = test_keypair();
        let challenge = new_challenge().unwrap();

        // Timestamp 60 seconds in the past with 30s tolerance -> should fail
        let old = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            - 60;
        let mut payload = Vec::with_capacity(40);
        payload.extend_from_slice(&challenge.nonce_bytes);
        payload.extend_from_slice(&old.to_be_bytes());

        let ring_kp = kp.ring_keypair();
        let sig = ring_kp.sign(&payload);
        let sig_hex = hex::encode(sig.as_ref());

        let result = verify_response(&kp, &challenge, &challenge.nonce_b64, old, &sig_hex, 30);
        assert!(matches!(result, Err(AuthError::TimestampWindow)));
    }

    #[test]
    fn test_future_timestamp_rejected() {
        let kp = test_keypair();
        let challenge = new_challenge().unwrap();

        // Timestamp 30 seconds in the future (max allowed is +5s)
        let future = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 30;
        let mut payload = Vec::with_capacity(40);
        payload.extend_from_slice(&challenge.nonce_bytes);
        payload.extend_from_slice(&future.to_be_bytes());

        let ring_kp = kp.ring_keypair();
        let sig = ring_kp.sign(&payload);
        let sig_hex = hex::encode(sig.as_ref());

        let result = verify_response(&kp, &challenge, &challenge.nonce_b64, future, &sig_hex, 30);
        assert!(matches!(result, Err(AuthError::TimestampWindow)));
    }

    #[test]
    fn test_bad_hex_encoding() {
        let kp = test_keypair();
        let challenge = new_challenge().unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let result = verify_response(
            &kp,
            &challenge,
            &challenge.nonce_b64,
            now,
            "not-valid-hex!",
            30,
        );
        assert!(matches!(result, Err(AuthError::BadEncoding(_))));
    }

    #[test]
    fn test_nonce_mismatch_rejected() {
        let kp = test_keypair();
        let challenge = new_challenge().unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let mut payload = Vec::with_capacity(40);
        payload.extend_from_slice(&challenge.nonce_bytes);
        payload.extend_from_slice(&now.to_be_bytes());

        let ring_kp = kp.ring_keypair();
        let sig = ring_kp.sign(&payload);
        let sig_hex = hex::encode(sig.as_ref());

        let result = verify_response(&kp, &challenge, "wrong-nonce", now, &sig_hex, 30);
        assert!(matches!(result, Err(AuthError::NonceMismatch)));
    }

    #[test]
    fn test_pubkey_hex_length() {
        let kp = test_keypair();
        let hex_str = public_key_hex(&kp);
        assert_eq!(hex_str.len(), 64); // 32 bytes = 64 hex chars
    }

    #[test]
    fn test_pubkey_hex_consistency() {
        let kp = test_keypair();
        let h1 = public_key_hex(&kp);
        let h2 = public_key_hex(&kp);
        assert_eq!(h1, h2);
    }

    #[cfg(unix)]
    #[test]
    fn test_key_file_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("perm.key");
        let _kp = load_or_generate(&path).unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
