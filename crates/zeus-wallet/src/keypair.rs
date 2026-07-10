//! Ed25519 keypair management — generation, signing, verification, persistence

use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit},
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use bs58;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::RngCore;
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};
use zeroize::Zeroize;

/// File header indicating AES-256-GCM encrypted key
const ENCRYPTED_HEADER: &[u8] = b"ZEUS-ENC-V1\n";
/// AES-GCM nonce size in bytes
const AES_NONCE_SIZE: usize = 12;

/// Errors specific to wallet operations
#[derive(Debug, thiserror::Error)]
pub enum WalletError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Key error: {0}")]
    Key(String),

    #[error("Signature error: {0}")]
    Signature(String),

    #[error("Serialization error: {0}")]
    Serialization(String),
}

impl From<WalletError> for zeus_core::Error {
    fn from(e: WalletError) -> Self {
        zeus_core::Error::Security(e.to_string())
    }
}

/// Persistent metadata stored alongside the key
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletMetadata {
    /// Human-readable label
    pub label: String,
    /// Creation timestamp
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Public key in hex
    pub public_key_hex: String,
    /// Network identifier
    pub network: String,
}

/// Ed25519 keypair with persistence support
pub struct WalletKeypair {
    signing_key: SigningKey,
    wallet_dir: PathBuf,
}

impl WalletKeypair {
    /// Generate a fresh Ed25519 keypair and persist to `wallet_dir`
    pub fn generate(
        wallet_dir: impl Into<PathBuf>,
        label: &str,
        network: &str,
    ) -> Result<Self, WalletError> {
        let wallet_dir = wallet_dir.into();
        fs::create_dir_all(&wallet_dir)?;

        let signing_key = SigningKey::generate(&mut OsRng);
        let wallet = Self {
            signing_key,
            wallet_dir,
        };

        wallet.save(label, network)?;
        info!(
            label,
            pubkey = %wallet.public_key_hex(),
            "Generated new Ed25519 wallet"
        );
        Ok(wallet)
    }

    /// Load an existing keypair from `wallet_dir`
    ///
    /// Supports both encrypted (ZEUS-ENC-V1 header) and legacy plaintext base64 formats.
    pub fn load(wallet_dir: impl Into<PathBuf>) -> Result<Self, WalletError> {
        let wallet_dir = wallet_dir.into();
        let key_path = wallet_dir.join("secret.key");

        if !key_path.exists() {
            return Err(WalletError::Key(format!(
                "No wallet found at {}",
                key_path.display()
            )));
        }

        let key_file = fs::read(&key_path)?;

        let mut secret = if key_file.starts_with(ENCRYPTED_HEADER) {
            // Encrypted format: ZEUS-ENC-V1\n<base64(nonce || ciphertext)>
            let b64_data = &key_file[ENCRYPTED_HEADER.len()..];
            let b64_str = std::str::from_utf8(b64_data)
                .map_err(|e| WalletError::Key(format!("Invalid key file encoding: {e}")))?
                .trim();
            let mut blob = BASE64
                .decode(b64_str)
                .map_err(|e| WalletError::Key(format!("Invalid encrypted key data: {e}")))?;

            if blob.len() < AES_NONCE_SIZE + 32 {
                blob.zeroize();
                return Err(WalletError::Key("Encrypted key data too short".into()));
            }

            let mut enc_key = Self::load_encryption_key(&wallet_dir)?;
            let cipher = Aes256Gcm::new_from_slice(&enc_key)
                .map_err(|e| WalletError::Key(format!("Invalid encryption key: {e}")))?;
            enc_key.zeroize();

            let nonce = Nonce::from_slice(&blob[..AES_NONCE_SIZE]);
            let mut plaintext = cipher
                .decrypt(nonce, &blob[AES_NONCE_SIZE..])
                .map_err(|e| WalletError::Key(format!("Decryption failed: {e}")))?;
            blob.zeroize();

            if plaintext.len() != 32 {
                plaintext.zeroize();
                return Err(WalletError::Key(format!(
                    "Decrypted key wrong length: expected 32, got {}",
                    plaintext.len()
                )));
            }

            let mut key = [0u8; 32];
            key.copy_from_slice(&plaintext);
            plaintext.zeroize();
            key
        } else {
            // Legacy plaintext base64 format
            warn!("Loading wallet with unencrypted key — consider re-generating");
            let mut decoded = BASE64
                .decode(&key_file)
                .map_err(|e| WalletError::Key(format!("Invalid base64 key: {e}")))?;

            if decoded.len() != 32 {
                decoded.zeroize();
                return Err(WalletError::Key(format!(
                    "Invalid key length: expected 32, got {}",
                    decoded.len()
                )));
            }

            let mut key = [0u8; 32];
            key.copy_from_slice(&decoded);
            decoded.zeroize();
            key
        };

        let signing_key = SigningKey::from_bytes(&secret);
        secret.zeroize();

        debug!(pubkey = %hex::encode(signing_key.verifying_key().as_bytes()), "Loaded wallet");
        Ok(Self {
            signing_key,
            wallet_dir,
        })
    }

    /// Load or generate: try to load, generate if not found
    pub fn load_or_generate(
        wallet_dir: impl Into<PathBuf>,
        label: &str,
        network: &str,
    ) -> Result<Self, WalletError> {
        let wallet_dir = wallet_dir.into();
        match Self::load(&wallet_dir) {
            Ok(kp) => Ok(kp),
            Err(_) => Self::generate(wallet_dir, label, network),
        }
    }

    /// Sign a message, returning the signature bytes
    pub fn sign(&self, message: &[u8]) -> Vec<u8> {
        let sig = self.signing_key.sign(message);
        sig.to_bytes().to_vec()
    }

    /// Sign a message, returning base64-encoded signature
    pub fn sign_base64(&self, message: &[u8]) -> String {
        BASE64.encode(self.sign(message))
    }

    /// Verify a signature against a message using our public key
    pub fn verify(&self, message: &[u8], signature: &[u8]) -> Result<(), WalletError> {
        let sig = Signature::from_slice(signature)
            .map_err(|e| WalletError::Signature(format!("Invalid signature bytes: {e}")))?;

        self.signing_key
            .verifying_key()
            .verify(message, &sig)
            .map_err(|e| WalletError::Signature(format!("Verification failed: {e}")))
    }

    /// Verify a signature from an external public key
    pub fn verify_external(
        public_key_bytes: &[u8; 32],
        message: &[u8],
        signature: &[u8],
    ) -> Result<(), WalletError> {
        let verifying_key = VerifyingKey::from_bytes(public_key_bytes)
            .map_err(|e| WalletError::Key(format!("Invalid public key: {e}")))?;
        let sig = Signature::from_slice(signature)
            .map_err(|e| WalletError::Signature(format!("Invalid signature bytes: {e}")))?;

        verifying_key
            .verify(message, &sig)
            .map_err(|e| WalletError::Signature(format!("Verification failed: {e}")))
    }

    /// Get the public key as a 32-byte array
    pub fn public_key_bytes(&self) -> [u8; 32] {
        self.signing_key.verifying_key().to_bytes()
    }

    /// Get the public key as hex string
    pub fn public_key_hex(&self) -> String {
        hex::encode(self.public_key_bytes())
    }

    /// Get the public key as base64 string
    pub fn public_key_base64(&self) -> String {
        BASE64.encode(self.public_key_bytes())
    }

    /// Get the public key as base58 string (Solana address format)
    pub fn public_key_base58(&self) -> String {
        bs58::encode(self.public_key_bytes()).into_string()
    }

    /// Get the wallet address as a base58-encoded Solana address.
    /// Alias for `public_key_base58()` — use this when the context is
    /// specifically Solana address display, airdrop requests, or x402 headers.
    pub fn address_base58(&self) -> String {
        self.public_key_base58()
    }

    /// SHA-256 hash of a message (utility for signing structured data)
    pub fn hash_message(message: &[u8]) -> Vec<u8> {
        let mut hasher = Sha256::new();
        hasher.update(message);
        hasher.finalize().to_vec()
    }

    /// Get the wallet directory path
    pub fn wallet_dir(&self) -> &Path {
        &self.wallet_dir
    }

    /// Read wallet metadata if it exists
    pub fn metadata(&self) -> Result<Option<WalletMetadata>, WalletError> {
        let meta_path = self.wallet_dir.join("metadata.json");
        if !meta_path.exists() {
            return Ok(None);
        }
        let data = fs::read_to_string(&meta_path)?;
        serde_json::from_str(&data)
            .map(Some)
            .map_err(|e| WalletError::Serialization(format!("Invalid metadata: {e}")))
    }

    // -- private --

    /// Load the AES-256-GCM encryption key from disk (does not generate)
    fn load_encryption_key(wallet_dir: &Path) -> Result<[u8; 32], WalletError> {
        let enc_key_path = wallet_dir.join("encryption.key");
        if !enc_key_path.exists() {
            return Err(WalletError::Key(
                "Encryption key missing — cannot decrypt wallet".into(),
            ));
        }
        let mut raw = fs::read(&enc_key_path)?;
        let mut decoded = BASE64
            .decode(&raw)
            .map_err(|e| WalletError::Key(format!("Invalid encryption key: {e}")))?;
        raw.zeroize();
        if decoded.len() != 32 {
            decoded.zeroize();
            return Err(WalletError::Key("Encryption key must be 32 bytes".into()));
        }
        let mut key = [0u8; 32];
        key.copy_from_slice(&decoded);
        decoded.zeroize();
        Ok(key)
    }

    /// Load or generate the AES-256-GCM encryption key
    fn ensure_encryption_key(wallet_dir: &Path) -> Result<[u8; 32], WalletError> {
        if let Ok(key) = Self::load_encryption_key(wallet_dir) {
            return Ok(key);
        }

        let enc_key_path = wallet_dir.join("encryption.key");
        let mut key = [0u8; 32];
        OsRng.fill_bytes(&mut key);

        let mut encoded = BASE64.encode(key);
        fs::write(&enc_key_path, encoded.as_bytes())?;
        encoded.zeroize();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&enc_key_path, fs::Permissions::from_mode(0o600))?;
        }

        Ok(key)
    }

    fn save(&self, label: &str, network: &str) -> Result<(), WalletError> {
        let key_path = self.wallet_dir.join("secret.key");
        let meta_path = self.wallet_dir.join("metadata.json");

        // Encrypt secret key with AES-256-GCM
        let mut enc_key = Self::ensure_encryption_key(&self.wallet_dir)?;
        let mut secret_bytes = self.signing_key.to_bytes();

        let cipher = Aes256Gcm::new_from_slice(&enc_key)
            .map_err(|e| WalletError::Key(format!("Invalid encryption key: {e}")))?;
        enc_key.zeroize();

        let mut nonce_bytes = [0u8; AES_NONCE_SIZE];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let mut ciphertext = cipher
            .encrypt(nonce, secret_bytes.as_ref())
            .map_err(|e| WalletError::Key(format!("Encryption failed: {e}")))?;
        secret_bytes.zeroize();

        // Format: nonce || ciphertext
        let mut blob = Vec::with_capacity(AES_NONCE_SIZE + ciphertext.len());
        blob.extend_from_slice(&nonce_bytes);
        blob.append(&mut ciphertext);

        let mut encoded = BASE64.encode(&blob);
        blob.zeroize();

        // Write: ZEUS-ENC-V1\n<base64(nonce || ciphertext)>
        let mut file_content = Vec::with_capacity(ENCRYPTED_HEADER.len() + encoded.len());
        file_content.extend_from_slice(ENCRYPTED_HEADER);
        file_content.extend_from_slice(encoded.as_bytes());
        encoded.zeroize();

        fs::write(&key_path, &file_content)?;
        file_content.zeroize();

        // Restrict permissions on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&key_path, fs::Permissions::from_mode(0o600))?;
        }

        // Write metadata
        let metadata = WalletMetadata {
            label: label.to_string(),
            created_at: chrono::Utc::now(),
            public_key_hex: self.public_key_hex(),
            network: network.to_string(),
        };
        let json = serde_json::to_string_pretty(&metadata)
            .map_err(|e| WalletError::Serialization(e.to_string()))?;
        fs::write(&meta_path, json)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_generate_and_load() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("wallet");

        // Generate
        let kp = WalletKeypair::generate(&dir, "test-agent", "solana-devnet").unwrap();
        let pubkey = kp.public_key_hex();
        assert_eq!(pubkey.len(), 64); // 32 bytes = 64 hex chars

        // Load
        let kp2 = WalletKeypair::load(&dir).unwrap();
        assert_eq!(kp2.public_key_hex(), pubkey);
    }

    #[test]
    fn test_load_or_generate() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("wallet");

        // First call generates
        let kp1 = WalletKeypair::load_or_generate(&dir, "test", "devnet").unwrap();
        let pubkey1 = kp1.public_key_hex();

        // Second call loads
        let kp2 = WalletKeypair::load_or_generate(&dir, "test", "devnet").unwrap();
        assert_eq!(kp2.public_key_hex(), pubkey1);
    }

    #[test]
    fn test_sign_and_verify() {
        let tmp = TempDir::new().unwrap();
        let kp = WalletKeypair::generate(tmp.path().join("w"), "test", "devnet").unwrap();

        let msg = b"hello zeus";
        let sig = kp.sign(msg);

        // Verify with own key
        assert!(kp.verify(msg, &sig).is_ok());

        // Wrong message fails
        assert!(kp.verify(b"wrong", &sig).is_err());
    }

    #[test]
    fn test_sign_base64() {
        let tmp = TempDir::new().unwrap();
        let kp = WalletKeypair::generate(tmp.path().join("w"), "test", "devnet").unwrap();

        let msg = b"payment:1000:usdc";
        let sig_b64 = kp.sign_base64(msg);

        // Decode and verify
        let sig_bytes = BASE64.decode(&sig_b64).unwrap();
        assert!(kp.verify(msg, &sig_bytes).is_ok());
    }

    #[test]
    fn test_external_verify() {
        let tmp = TempDir::new().unwrap();
        let kp = WalletKeypair::generate(tmp.path().join("w"), "test", "devnet").unwrap();

        let msg = b"external verification test";
        let sig = kp.sign(msg);
        let pubkey = kp.public_key_bytes();

        assert!(WalletKeypair::verify_external(&pubkey, msg, &sig).is_ok());
        assert!(WalletKeypair::verify_external(&pubkey, b"tampered", &sig).is_err());
    }

    #[test]
    fn test_hash_message() {
        let hash = WalletKeypair::hash_message(b"test");
        assert_eq!(hash.len(), 32); // SHA-256 = 32 bytes

        // Deterministic
        let hash2 = WalletKeypair::hash_message(b"test");
        assert_eq!(hash, hash2);

        // Different input = different hash
        let hash3 = WalletKeypair::hash_message(b"other");
        assert_ne!(hash, hash3);
    }

    #[test]
    fn test_metadata() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("wallet");
        let kp = WalletKeypair::generate(&dir, "my-agent", "solana-mainnet").unwrap();

        let meta = kp.metadata().unwrap().expect("metadata should exist");
        assert_eq!(meta.label, "my-agent");
        assert_eq!(meta.network, "solana-mainnet");
        assert_eq!(meta.public_key_hex, kp.public_key_hex());
    }

    #[test]
    fn test_public_key_base58() {
        let tmp = TempDir::new().unwrap();
        let kp = WalletKeypair::generate(tmp.path().join("w"), "test", "devnet").unwrap();

        let b58 = kp.public_key_base58();
        // Ed25519 pubkey is 32 bytes; base58-encoded that's typically 43–44 chars
        assert!(!b58.is_empty());
        assert!(b58.len() >= 40 && b58.len() <= 50);

        // Round-trip: decode and compare to raw bytes
        let decoded = bs58::decode(&b58).into_vec().unwrap();
        assert_eq!(decoded, kp.public_key_bytes().to_vec());

        // Consistent across calls
        assert_eq!(kp.public_key_base58(), b58);
    }

    #[test]
    fn test_load_missing_wallet() {
        let tmp = TempDir::new().unwrap();
        let result = WalletKeypair::load(tmp.path().join("nonexistent"));
        assert!(result.is_err());
    }

    #[test]
    fn test_file_permissions() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("wallet");
        WalletKeypair::generate(&dir, "test", "devnet").unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let key_path = dir.join("secret.key");
            let perms = fs::metadata(&key_path).unwrap().permissions();
            assert_eq!(perms.mode() & 0o777, 0o600);
        }
    }

    #[test]
    fn test_encrypted_key_format() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("wallet");
        WalletKeypair::generate(&dir, "test", "devnet").unwrap();

        // secret.key should start with ZEUS-ENC-V1 header
        let content = fs::read(dir.join("secret.key")).unwrap();
        assert!(content.starts_with(b"ZEUS-ENC-V1\n"));

        // encryption.key should exist
        assert!(dir.join("encryption.key").exists());
    }

    #[test]
    fn test_encryption_key_permissions() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("wallet");
        WalletKeypair::generate(&dir, "test", "devnet").unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = fs::metadata(dir.join("encryption.key"))
                .unwrap()
                .permissions();
            assert_eq!(perms.mode() & 0o777, 0o600);
        }
    }

    #[test]
    fn test_legacy_plaintext_load() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("wallet");
        fs::create_dir_all(&dir).unwrap();

        // Write a legacy plaintext base64 key file (pre-encryption format)
        let signing_key = SigningKey::generate(&mut OsRng);
        let key_bytes = signing_key.to_bytes();
        let encoded = BASE64.encode(key_bytes);
        fs::write(dir.join("secret.key"), encoded.as_bytes()).unwrap();

        // Should load successfully without encryption.key
        let kp = WalletKeypair::load(&dir).unwrap();
        assert_eq!(
            kp.public_key_bytes(),
            signing_key.verifying_key().to_bytes()
        );
    }

    #[test]
    fn test_encrypted_load_without_encryption_key_fails() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("wallet");
        WalletKeypair::generate(&dir, "test", "devnet").unwrap();

        // Remove encryption.key — load should fail
        fs::remove_file(dir.join("encryption.key")).unwrap();
        let result = WalletKeypair::load(&dir);
        assert!(result.is_err());
    }
}
