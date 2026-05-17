//! PKCE (Proof Key for Code Exchange) generation.
//!
//! Generates a random verifier and its SHA256 challenge for OAuth 2.0 PKCE flow.

use base64::Engine;
use rand::Rng;
use sha2::{Digest, Sha256};

/// A PKCE verifier + challenge pair.
#[derive(Debug, Clone)]
pub struct PkceChallenge {
    /// The random verifier string (sent during token exchange).
    pub verifier: String,
    /// The SHA256 challenge derived from the verifier (sent during authorization).
    pub challenge: String,
}

impl PkceChallenge {
    /// Generate a new PKCE challenge pair.
    ///
    /// The verifier is 43 random alphanumeric characters (per RFC 7636).
    /// The challenge is `base64url(sha256(verifier))`.
    pub fn generate() -> Self {
        let mut rng = rand::thread_rng();
        let verifier: String = (0..43)
            .map(|_| {
                const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";
                CHARSET[rng.gen_range(0..CHARSET.len())] as char
            })
            .collect();

        let mut hasher = Sha256::new();
        hasher.update(verifier.as_bytes());
        let digest = hasher.finalize();
        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest);

        Self { verifier, challenge }
    }
}

/// Generate a random state parameter for CSRF protection.
pub fn generate_state() -> String {
    let mut rng = rand::thread_rng();
    let bytes: Vec<u8> = (0..32).map(|_| rng.r#gen()).collect();
    hex::encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_verifier_is_43_chars() {
        let pkce = PkceChallenge::generate();
        assert_eq!(pkce.verifier.len(), 43);
    }

    #[test]
    fn pkce_challenge_is_base64url() {
        let pkce = PkceChallenge::generate();
        assert!(!pkce.challenge.is_empty());
        assert!(!pkce.challenge.contains('+'));
        assert!(!pkce.challenge.contains('/'));
        assert!(!pkce.challenge.contains('='));
    }

    #[test]
    fn state_is_64_hex_chars() {
        let state = generate_state();
        assert_eq!(state.len(), 64);
    }
}
