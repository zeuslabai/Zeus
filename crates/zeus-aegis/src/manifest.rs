//! Ed25519-signed agent manifests for cryptographic agent identity.
//!
//! Each agent has a keypair stored at `~/.zeus/agents/<id>/`. The manifest
//! describes the agent's identity, capabilities, and version. The gateway
//! verifies the signature before spawning — unsigned or tampered manifests
//! are rejected.
//!
//! ## Usage
//!
//! ```no_run
//! use zeus_aegis::manifest::{AgentManifest, generate_agent_keypair, sign_manifest, verify_manifest};
//!
//! // Generate keypair for a new agent
//! let signing_key = generate_agent_keypair("agent-1", "/path/to/agents").unwrap();
//!
//! // Create and sign a manifest
//! let manifest = AgentManifest::new("agent-1", "Scout Agent", "1.0.0");
//! let signed = sign_manifest(&signing_key, &manifest);
//!
//! // Verify before spawning
//! assert!(verify_manifest(&signed).is_ok());
//! ```

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use chrono::{DateTime, Utc};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use std::path::Path;
use zeus_core::{Error, Result};

// ============================================================================
// Agent manifest
// ============================================================================

/// Agent manifest — the identity document that gets signed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentManifest {
    /// Unique agent identifier
    pub agent_id: String,
    /// Human-readable name
    pub name: String,
    /// Semantic version
    pub version: String,
    /// Capabilities this agent is authorized to use (tool names, channel names)
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Public key in hex (32 bytes = 64 hex chars)
    pub public_key: String,
    /// When this manifest was created
    pub created_at: DateTime<Utc>,
}

impl AgentManifest {
    /// Create a new manifest (without public key — call `with_public_key` after keypair generation).
    pub fn new(agent_id: &str, name: &str, version: &str) -> Self {
        Self {
            agent_id: agent_id.to_string(),
            name: name.to_string(),
            version: version.to_string(),
            capabilities: Vec::new(),
            public_key: String::new(),
            created_at: Utc::now(),
        }
    }

    /// Set the public key from a signing key.
    pub fn with_public_key(mut self, signing_key: &SigningKey) -> Self {
        self.public_key = hex::encode(signing_key.verifying_key().as_bytes());
        self
    }

    /// Add a capability.
    pub fn with_capability(mut self, cap: &str) -> Self {
        self.capabilities.push(cap.to_string());
        self
    }

    /// Add multiple capabilities.
    pub fn with_capabilities(mut self, caps: &[&str]) -> Self {
        self.capabilities.extend(caps.iter().map(|c| c.to_string()));
        self
    }

    /// Canonical bytes for signing — deterministic JSON serialization.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        // Use sorted-key JSON for deterministic output
        serde_json::to_vec(self).unwrap_or_default()
    }
}

// ============================================================================
// Signed manifest
// ============================================================================

/// Signed manifest — manifest + Ed25519 signature.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedManifest {
    /// The agent manifest
    pub manifest: AgentManifest,
    /// Base64-encoded Ed25519 signature of `manifest.canonical_bytes()`
    pub signature: String,
}

impl SignedManifest {
    /// Load a signed manifest from a JSON file.
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| Error::Security(format!("Failed to read manifest: {e}")))?;
        serde_json::from_str(&content)
            .map_err(|e| Error::Security(format!("Invalid manifest JSON: {e}")))
    }

    /// Save the signed manifest to a JSON file.
    pub fn save(&self, path: &Path) -> Result<()> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| Error::Security(format!("Failed to serialize manifest: {e}")))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| Error::Security(format!("Failed to create directory: {e}")))?;
        }
        std::fs::write(path, json)
            .map_err(|e| Error::Security(format!("Failed to write manifest: {e}")))?;
        Ok(())
    }
}

// ============================================================================
// Key management
// ============================================================================

/// Generate a new Ed25519 keypair for an agent.
///
/// Stores the secret key at `{agents_dir}/{agent_id}/signing.key` (base64, chmod 600)
/// and returns the `SigningKey` for immediate use.
pub fn generate_agent_keypair(agent_id: &str, agents_dir: &str) -> Result<SigningKey> {
    let agent_dir = Path::new(agents_dir).join(agent_id);
    std::fs::create_dir_all(&agent_dir)
        .map_err(|e| Error::Security(format!("Failed to create agent directory: {e}")))?;

    let signing_key = SigningKey::generate(&mut OsRng);

    // Persist secret key as base64
    let key_path = agent_dir.join("signing.key");
    let encoded = BASE64.encode(signing_key.to_bytes());
    std::fs::write(&key_path, encoded.as_bytes())
        .map_err(|e| Error::Security(format!("Failed to write signing key: {e}")))?;

    // Restrict permissions (chmod 600)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600));
    }

    tracing::info!(
        agent_id,
        pubkey = %hex::encode(signing_key.verifying_key().as_bytes()),
        "Generated agent signing keypair"
    );

    Ok(signing_key)
}

/// Load an existing agent keypair from `{agents_dir}/{agent_id}/signing.key`.
pub fn load_agent_keypair(agent_id: &str, agents_dir: &str) -> Result<SigningKey> {
    let key_path = Path::new(agents_dir).join(agent_id).join("signing.key");
    if !key_path.exists() {
        return Err(Error::Security(format!(
            "No signing key found for agent '{agent_id}' at {}",
            key_path.display()
        )));
    }

    let content = std::fs::read_to_string(&key_path)
        .map_err(|e| Error::Security(format!("Failed to read signing key: {e}")))?;
    let decoded = BASE64
        .decode(content.trim().as_bytes())
        .map_err(|e| Error::Security(format!("Invalid base64 signing key: {e}")))?;

    if decoded.len() != 32 {
        return Err(Error::Security(format!(
            "Invalid signing key length: expected 32, got {}",
            decoded.len()
        )));
    }

    let mut secret = [0u8; 32];
    secret.copy_from_slice(&decoded);
    Ok(SigningKey::from_bytes(&secret))
}

/// Load or generate: try to load existing keypair, generate if not found.
pub fn load_or_generate_keypair(agent_id: &str, agents_dir: &str) -> Result<SigningKey> {
    match load_agent_keypair(agent_id, agents_dir) {
        Ok(key) => Ok(key),
        Err(_) => generate_agent_keypair(agent_id, agents_dir),
    }
}

// ============================================================================
// Signing & verification
// ============================================================================

/// Sign an agent manifest, producing a `SignedManifest`.
pub fn sign_manifest(signing_key: &SigningKey, manifest: &AgentManifest) -> SignedManifest {
    let canonical = manifest.canonical_bytes();
    let signature = signing_key.sign(&canonical);
    SignedManifest {
        manifest: manifest.clone(),
        signature: BASE64.encode(signature.to_bytes()),
    }
}

/// Verify a signed manifest's Ed25519 signature.
///
/// Extracts the public key from `manifest.public_key` (hex), decodes the
/// signature from `signed.signature` (base64), and verifies against the
/// canonical manifest bytes.
///
/// Returns `Ok(())` on success, `Err` if the signature is invalid, the
/// public key is malformed, or the manifest has been tampered with.
pub fn verify_manifest(signed: &SignedManifest) -> Result<()> {
    // Decode public key from manifest
    let pubkey_bytes = hex::decode(&signed.manifest.public_key)
        .map_err(|e| Error::Security(format!("Invalid public key hex: {e}")))?;

    if pubkey_bytes.len() != 32 {
        return Err(Error::Security(format!(
            "Invalid public key length: expected 32, got {}",
            pubkey_bytes.len()
        )));
    }

    let mut pubkey_arr = [0u8; 32];
    pubkey_arr.copy_from_slice(&pubkey_bytes);
    let verifying_key = VerifyingKey::from_bytes(&pubkey_arr)
        .map_err(|e| Error::Security(format!("Invalid Ed25519 public key: {e}")))?;

    // Decode signature
    let sig_bytes = BASE64
        .decode(&signed.signature)
        .map_err(|e| Error::Security(format!("Invalid signature base64: {e}")))?;
    let signature = Signature::from_slice(&sig_bytes)
        .map_err(|e| Error::Security(format!("Invalid Ed25519 signature: {e}")))?;

    // Verify
    let canonical = signed.manifest.canonical_bytes();
    verifying_key
        .verify(&canonical, &signature)
        .map_err(|e| Error::Security(format!("Manifest signature verification failed: {e}")))?;

    Ok(())
}

/// Verify a signed manifest against a specific trusted public key.
///
/// Unlike `verify_manifest`, this does NOT trust the public key embedded
/// in the manifest. Instead, it verifies against a separately-provided key.
/// Use this for gateway verification where the trusted key is stored in the
/// fleet registry, not in the manifest itself.
pub fn verify_manifest_with_key(signed: &SignedManifest, trusted_pubkey: &[u8; 32]) -> Result<()> {
    let verifying_key = VerifyingKey::from_bytes(trusted_pubkey)
        .map_err(|e| Error::Security(format!("Invalid trusted public key: {e}")))?;

    let sig_bytes = BASE64
        .decode(&signed.signature)
        .map_err(|e| Error::Security(format!("Invalid signature base64: {e}")))?;
    let signature = Signature::from_slice(&sig_bytes)
        .map_err(|e| Error::Security(format!("Invalid Ed25519 signature: {e}")))?;

    let canonical = signed.manifest.canonical_bytes();
    verifying_key
        .verify(&canonical, &signature)
        .map_err(|e| Error::Security(format!("Manifest verification failed against trusted key: {e}")))?;

    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn test_keypair() -> SigningKey {
        SigningKey::generate(&mut OsRng)
    }

    #[test]
    fn test_manifest_creation() {
        let manifest = AgentManifest::new("agent-1", "Scout", "1.0.0");
        assert_eq!(manifest.agent_id, "agent-1");
        assert_eq!(manifest.name, "Scout");
        assert_eq!(manifest.version, "1.0.0");
        assert!(manifest.capabilities.is_empty());
        assert!(manifest.public_key.is_empty());
    }

    #[test]
    fn test_manifest_with_public_key() {
        let key = test_keypair();
        let manifest = AgentManifest::new("agent-1", "Scout", "1.0.0").with_public_key(&key);
        assert_eq!(manifest.public_key.len(), 64); // 32 bytes = 64 hex
    }

    #[test]
    fn test_manifest_with_capabilities() {
        let manifest = AgentManifest::new("agent-1", "Scout", "1.0.0")
            .with_capabilities(&["shell", "read_file", "web_fetch"]);
        assert_eq!(manifest.capabilities.len(), 3);
        assert!(manifest.capabilities.contains(&"shell".to_string()));
    }

    #[test]
    fn test_sign_and_verify() {
        let key = test_keypair();
        let manifest = AgentManifest::new("agent-1", "Scout", "1.0.0").with_public_key(&key);
        let signed = sign_manifest(&key, &manifest);

        assert!(verify_manifest(&signed).is_ok());
    }

    #[test]
    fn test_verify_rejects_tampered_name() {
        let key = test_keypair();
        let manifest = AgentManifest::new("agent-1", "Scout", "1.0.0").with_public_key(&key);
        let mut signed = sign_manifest(&key, &manifest);

        // Tamper with the name
        signed.manifest.name = "Evil Agent".to_string();
        assert!(verify_manifest(&signed).is_err());
    }

    #[test]
    fn test_verify_rejects_tampered_capabilities() {
        let key = test_keypair();
        let manifest = AgentManifest::new("agent-1", "Scout", "1.0.0")
            .with_public_key(&key)
            .with_capability("read_file");
        let mut signed = sign_manifest(&key, &manifest);

        // Add unauthorized capability
        signed.manifest.capabilities.push("shell".to_string());
        assert!(verify_manifest(&signed).is_err());
    }

    #[test]
    fn test_verify_rejects_wrong_key() {
        let key1 = test_keypair();
        let key2 = test_keypair();
        let manifest = AgentManifest::new("agent-1", "Scout", "1.0.0").with_public_key(&key1);
        let mut signed = sign_manifest(&key1, &manifest);

        // Replace public key with different key
        signed.manifest.public_key = hex::encode(key2.verifying_key().as_bytes());
        assert!(verify_manifest(&signed).is_err());
    }

    #[test]
    fn test_verify_rejects_invalid_signature() {
        let key = test_keypair();
        let manifest = AgentManifest::new("agent-1", "Scout", "1.0.0").with_public_key(&key);
        let mut signed = sign_manifest(&key, &manifest);

        // Corrupt signature
        signed.signature = BASE64.encode(vec![0u8; 64]);
        assert!(verify_manifest(&signed).is_err());
    }

    #[test]
    fn test_verify_with_trusted_key() {
        let key = test_keypair();
        let manifest = AgentManifest::new("agent-1", "Scout", "1.0.0").with_public_key(&key);
        let signed = sign_manifest(&key, &manifest);

        let trusted = key.verifying_key().to_bytes();
        assert!(verify_manifest_with_key(&signed, &trusted).is_ok());
    }

    #[test]
    fn test_verify_with_wrong_trusted_key() {
        let key1 = test_keypair();
        let key2 = test_keypair();
        let manifest = AgentManifest::new("agent-1", "Scout", "1.0.0").with_public_key(&key1);
        let signed = sign_manifest(&key1, &manifest);

        let wrong_trusted = key2.verifying_key().to_bytes();
        assert!(verify_manifest_with_key(&signed, &wrong_trusted).is_err());
    }

    #[test]
    fn test_generate_and_load_keypair() {
        let tmp = tempfile::tempdir().unwrap();
        let agents_dir = tmp.path().to_str().unwrap();

        let key = generate_agent_keypair("test-agent", agents_dir).unwrap();
        let loaded = load_agent_keypair("test-agent", agents_dir).unwrap();

        // Same public key
        assert_eq!(
            key.verifying_key().as_bytes(),
            loaded.verifying_key().as_bytes()
        );
    }

    #[test]
    fn test_load_or_generate_creates_new() {
        let tmp = tempfile::tempdir().unwrap();
        let agents_dir = tmp.path().to_str().unwrap();

        let key1 = load_or_generate_keypair("new-agent", agents_dir).unwrap();
        let key2 = load_or_generate_keypair("new-agent", agents_dir).unwrap();

        // Second call loads the same key
        assert_eq!(
            key1.verifying_key().as_bytes(),
            key2.verifying_key().as_bytes()
        );
    }

    #[test]
    fn test_load_missing_keypair_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let agents_dir = tmp.path().to_str().unwrap();

        assert!(load_agent_keypair("nonexistent", agents_dir).is_err());
    }

    #[test]
    fn test_signed_manifest_save_and_load() {
        let tmp = tempfile::tempdir().unwrap();
        let key = test_keypair();
        let manifest = AgentManifest::new("agent-1", "Scout", "1.0.0").with_public_key(&key);
        let signed = sign_manifest(&key, &manifest);

        let path = tmp.path().join("manifest.json");
        signed.save(&path).unwrap();

        let loaded = SignedManifest::load(&path).unwrap();
        assert_eq!(loaded.manifest.agent_id, "agent-1");
        assert!(verify_manifest(&loaded).is_ok());
    }

    #[test]
    fn test_manifest_serialization_roundtrip() {
        let key = test_keypair();
        let manifest = AgentManifest::new("agent-1", "Scout", "1.0.0")
            .with_public_key(&key)
            .with_capabilities(&["shell", "read_file"]);
        let signed = sign_manifest(&key, &manifest);

        let json = serde_json::to_string(&signed).unwrap();
        let parsed: SignedManifest = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.manifest.agent_id, "agent-1");
        assert_eq!(parsed.manifest.capabilities.len(), 2);
        assert!(verify_manifest(&parsed).is_ok());
    }

    #[test]
    fn test_canonical_bytes_deterministic() {
        let key = test_keypair();
        let manifest = AgentManifest::new("agent-1", "Scout", "1.0.0").with_public_key(&key);

        let bytes1 = manifest.canonical_bytes();
        let bytes2 = manifest.canonical_bytes();
        assert_eq!(bytes1, bytes2);
    }

    #[test]
    fn test_empty_public_key_verify_fails() {
        let key = test_keypair();
        let manifest = AgentManifest::new("agent-1", "Scout", "1.0.0"); // no public key
        let signed = sign_manifest(&key, &manifest);

        // Empty public key should fail verification
        assert!(verify_manifest(&signed).is_err());
    }

    #[test]
    #[cfg(unix)]
    fn test_keypair_file_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let agents_dir = tmp.path().to_str().unwrap();
        generate_agent_keypair("perm-test", agents_dir).unwrap();

        let key_path = tmp.path().join("perm-test").join("signing.key");
        let perms = std::fs::metadata(&key_path).unwrap().permissions();
        assert_eq!(perms.mode() & 0o777, 0o600);
    }
}
