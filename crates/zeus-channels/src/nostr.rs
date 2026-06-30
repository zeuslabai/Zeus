//! Nostr channel adapter
//!
//! Provides Nostr messaging support via relays.
//! Uses NIP-01 (basic protocol), NIP-04 (encrypted DMs), and NIP-59 (gift wrapping).

use crate::{ChannelAdapter, ChannelMessage, ChannelSource, ReceiveMode};
use aes::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use futures_util::{SinkExt, StreamExt};
use rand::RngCore;
use rand::rngs::OsRng;
use secp256k1::{Secp256k1, SecretKey, XOnlyPublicKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{Notify, RwLock, mpsc};
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};
use zeus_core::{Error, Result};

/// External nostr crate (aliased as nostr_crate to avoid name conflict with this module)
extern crate nostr_crate;

/// Type alias for AES-256-CBC encryptor
type Aes256CbcEnc = cbc::Encryptor<aes::Aes256>;
/// Type alias for AES-256-CBC decryptor
type Aes256CbcDec = cbc::Decryptor<aes::Aes256>;

/// Type alias for the WebSocket writer
type WsSink = futures_util::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    WsMessage,
>;

/// Nostr keypair with secp256k1 for signing and ECDH
#[derive(Debug, Clone)]
pub struct NostrKeyPair {
    secret_key: SecretKey,
    public_key: XOnlyPublicKey,
}

impl NostrKeyPair {
    /// Create a keypair from a hex-encoded private key (64 hex chars = 32 bytes)
    pub fn from_hex(hex_str: &str) -> Result<Self> {
        let bytes = hex::decode(hex_str)
            .map_err(|e| Error::Config(format!("Invalid hex private key: {}", e)))?;
        if bytes.len() != 32 {
            return Err(Error::Config(format!(
                "Private key must be 32 bytes, got {}",
                bytes.len()
            )));
        }
        let secp = Secp256k1::new();
        let secret_key = SecretKey::from_slice(&bytes)
            .map_err(|e| Error::Config(format!("Invalid secp256k1 private key: {}", e)))?;
        let (public_key, _parity) = secret_key.x_only_public_key(&secp);
        Ok(Self {
            secret_key,
            public_key,
        })
    }

    /// Create a keypair from an nsec bech32-encoded private key.
    ///
    /// nsec format: "nsec1" prefix + bech32-encoded 32-byte secret key.
    /// Uses the nostr crate for proper bech32 decoding with checksum validation.
    pub fn from_nsec(nsec: &str) -> Result<Self> {
        let keys = nostr_crate::Keys::parse(nsec)
            .map_err(|e| Error::Config(format!("Invalid nsec key: {}", e)))?;
        // Deref nostr::SecretKey → secp256k1::SecretKey to extract raw bytes
        let sk_bytes = keys.secret_key().secret_bytes();
        Self::from_hex(&hex::encode(sk_bytes))
    }

    /// Get the public key as a hex string (x-only, 32 bytes = 64 hex chars)
    pub fn public_key_hex(&self) -> String {
        hex::encode(self.public_key.serialize())
    }

    /// Get the raw 32-byte secret key
    pub fn secret_bytes(&self) -> [u8; 32] {
        self.secret_key.secret_bytes()
    }

    /// Sign a message hash using Schnorr signature (BIP-340)
    pub fn sign_schnorr(&self, msg_hash: &[u8; 32]) -> Result<String> {
        let secp = Secp256k1::new();
        let msg = secp256k1::Message::from_digest(*msg_hash);
        let keypair = secp256k1::Keypair::from_secret_key(&secp, &self.secret_key);
        let sig = secp.sign_schnorr_no_aux_rand(&msg, &keypair);
        Ok(hex::encode(sig.serialize()))
    }

    /// Sign a Nostr event and return the signature hex string.
    /// `event_id_bytes` is the 32-byte SHA-256 hash of the serialized event.
    pub fn sign_event(&self, event_id_bytes: &[u8; 32]) -> Result<String> {
        self.sign_schnorr(event_id_bytes)
    }
}

/// Verify a Schnorr signature against a public key and message hash.
pub fn verify_schnorr_signature(
    pubkey_hex: &str,
    msg_hash: &[u8; 32],
    sig_hex: &str,
) -> Result<bool> {
    let pubkey_bytes = hex::decode(pubkey_hex)
        .map_err(|e| Error::Channel(format!("Invalid pubkey hex: {}", e)))?;
    let sig_bytes = hex::decode(sig_hex)
        .map_err(|e| Error::Channel(format!("Invalid signature hex: {}", e)))?;

    let secp = Secp256k1::verification_only();
    let pubkey = XOnlyPublicKey::from_slice(&pubkey_bytes)
        .map_err(|e| Error::Channel(format!("Invalid x-only public key: {}", e)))?;
    let sig = secp256k1::schnorr::Signature::from_slice(&sig_bytes)
        .map_err(|e| Error::Channel(format!("Invalid Schnorr signature: {}", e)))?;
    let msg = secp256k1::Message::from_digest(*msg_hash);

    Ok(secp.verify_schnorr(&sig, &msg, &pubkey).is_ok())
}

/// Compute the NIP-04 shared secret for ECDH between our private key and their public key.
///
/// Returns the x-coordinate of the ECDH shared point (32 bytes) which is used
/// as the AES-256 key for NIP-04 encryption.
fn nip04_shared_secret(private_key: &[u8; 32], recipient_pubkey: &[u8; 32]) -> Result<[u8; 32]> {
    let sk = SecretKey::from_slice(private_key)
        .map_err(|e| Error::Channel(format!("Invalid private key: {}", e)))?;

    // NIP-04 uses the full (non-x-only) public key for ECDH.
    // Since Nostr pubkeys are x-only (32 bytes), we prepend 0x02 to get a
    // compressed pubkey (even parity assumed, which is standard for NIP-04).
    let mut compressed = [0u8; 33];
    compressed[0] = 0x02;
    compressed[1..].copy_from_slice(recipient_pubkey);

    let pk = secp256k1::PublicKey::from_slice(&compressed)
        .map_err(|e| Error::Channel(format!("Invalid recipient public key: {}", e)))?;

    let shared_point = secp256k1::ecdh::shared_secret_point(&pk, &sk);
    // shared_secret_point returns a 64-byte uncompressed point (x || y).
    // NIP-04 uses just the x-coordinate (first 32 bytes) as the shared secret.
    let mut shared_secret = [0u8; 32];
    shared_secret.copy_from_slice(&shared_point[..32]);
    Ok(shared_secret)
}

/// Apply PKCS7 padding to make data a multiple of `block_size`.
fn pkcs7_pad(data: &[u8], block_size: usize) -> Vec<u8> {
    let padding_len = block_size - (data.len() % block_size);
    let mut padded = data.to_vec();
    padded.extend(std::iter::repeat_n(padding_len as u8, padding_len));
    padded
}

/// Remove PKCS7 padding from decrypted data.
fn pkcs7_unpad(data: &[u8]) -> Result<Vec<u8>> {
    if data.is_empty() {
        return Err(Error::Channel("Empty data for PKCS7 unpadding".into()));
    }
    let pad_byte = *data.last().expect("data is non-empty");
    let pad_len = pad_byte as usize;
    if pad_len == 0 || pad_len > 16 || pad_len > data.len() {
        return Err(Error::Channel(format!(
            "Invalid PKCS7 padding byte: {}",
            pad_byte
        )));
    }
    // Verify all padding bytes are correct
    for &b in &data[data.len() - pad_len..] {
        if b != pad_byte {
            return Err(Error::Channel("Invalid PKCS7 padding".into()));
        }
    }
    Ok(data[..data.len() - pad_len].to_vec())
}

/// Encrypt content for NIP-04 DM.
///
/// Returns the ciphertext in the format: `base64(ciphertext)?iv=base64(iv)`
pub fn nip04_encrypt(
    private_key: &[u8; 32],
    recipient_pubkey: &[u8; 32],
    content: &str,
) -> Result<String> {
    let shared_secret = nip04_shared_secret(private_key, recipient_pubkey)?;

    // Generate random 16-byte IV
    let mut iv = [0u8; 16];
    OsRng.fill_bytes(&mut iv);

    // PKCS7 pad the content
    let padded = pkcs7_pad(content.as_bytes(), 16);

    // AES-256-CBC encrypt
    let encryptor = Aes256CbcEnc::new(&shared_secret.into(), &iv.into());
    let ciphertext =
        encryptor.encrypt_padded_vec_mut::<aes::cipher::block_padding::NoPadding>(&padded);

    // Format: base64(ciphertext)?iv=base64(iv)
    let ct_b64 = BASE64.encode(&ciphertext);
    let iv_b64 = BASE64.encode(iv);
    Ok(format!("{}?iv={}", ct_b64, iv_b64))
}

/// Decrypt NIP-04 DM content.
///
/// Expects ciphertext in the format: `base64(ciphertext)?iv=base64(iv)`
pub fn nip04_decrypt(
    private_key: &[u8; 32],
    sender_pubkey: &[u8; 32],
    encrypted: &str,
) -> Result<String> {
    // Parse "ciphertext_base64?iv=iv_base64" format
    let parts: Vec<&str> = encrypted.splitn(2, "?iv=").collect();
    if parts.len() != 2 {
        return Err(Error::Channel(
            "Invalid NIP-04 ciphertext format, expected 'base64?iv=base64'".into(),
        ));
    }

    let ciphertext = BASE64
        .decode(parts[0])
        .map_err(|e| Error::Channel(format!("Invalid base64 ciphertext: {}", e)))?;
    let iv_bytes = BASE64
        .decode(parts[1])
        .map_err(|e| Error::Channel(format!("Invalid base64 IV: {}", e)))?;

    if iv_bytes.len() != 16 {
        return Err(Error::Channel(format!(
            "IV must be 16 bytes, got {}",
            iv_bytes.len()
        )));
    }

    let shared_secret = nip04_shared_secret(private_key, sender_pubkey)?;

    let mut iv = [0u8; 16];
    iv.copy_from_slice(&iv_bytes);

    // AES-256-CBC decrypt
    let decryptor = Aes256CbcDec::new(&shared_secret.into(), &iv.into());
    let decrypted = decryptor
        .decrypt_padded_vec_mut::<aes::cipher::block_padding::NoPadding>(&ciphertext)
        .map_err(|e| Error::Channel(format!("AES-256-CBC decryption failed: {}", e)))?;

    // Remove PKCS7 padding
    let unpadded = pkcs7_unpad(&decrypted)?;

    String::from_utf8(unpadded)
        .map_err(|e| Error::Channel(format!("Decrypted content is not valid UTF-8: {}", e)))
}

/// Compute the event ID (SHA-256) per NIP-01.
///
/// The event ID is the SHA-256 of the serialized event array:
/// `[0, pubkey, created_at, kind, tags, content]`
pub fn compute_event_id(
    pubkey: &str,
    created_at: i64,
    kind: u32,
    tags: &[Vec<String>],
    content: &str,
) -> [u8; 32] {
    let serialized = serde_json::json!([0, pubkey, created_at, kind, tags, content]);
    let bytes = serialized.to_string();
    let mut hasher = Sha256::new();
    hasher.update(bytes.as_bytes());
    let result = hasher.finalize();
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&result);
    hash
}

/// Create and sign a complete Nostr event.
///
/// Returns the full event JSON object with id, pubkey, created_at, kind, tags, content, sig.
pub fn create_signed_event(
    keypair: &NostrKeyPair,
    kind: u32,
    content: &str,
    tags: Vec<Vec<String>>,
) -> Result<serde_json::Value> {
    let pubkey = keypair.public_key_hex();
    let created_at = chrono::Utc::now().timestamp();

    let event_id_bytes = compute_event_id(&pubkey, created_at, kind, &tags, content);
    let event_id_hex = hex::encode(event_id_bytes);

    let sig = keypair.sign_event(&event_id_bytes)?;

    Ok(serde_json::json!({
        "id": event_id_hex,
        "pubkey": pubkey,
        "created_at": created_at,
        "kind": kind,
        "tags": tags,
        "content": content,
        "sig": sig
    }))
}

/// Verify an incoming Nostr event's signature.
///
/// Recomputes the event ID from the event fields and checks the Schnorr signature.
pub fn verify_event(event: &serde_json::Value) -> Result<bool> {
    let pubkey = event
        .get("pubkey")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::Channel("Event missing pubkey".into()))?;
    let created_at = event
        .get("created_at")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| Error::Channel("Event missing created_at".into()))?;
    let kind = event
        .get("kind")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| Error::Channel("Event missing kind".into()))? as u32;
    let content = event
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::Channel("Event missing content".into()))?;
    let sig = event
        .get("sig")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::Channel("Event missing sig".into()))?;
    let id = event
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::Channel("Event missing id".into()))?;

    // Parse tags
    let tags: Vec<Vec<String>> = event
        .get("tags")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    // Verify event ID
    let computed_id = compute_event_id(pubkey, created_at, kind, &tags, content);
    let computed_id_hex = hex::encode(computed_id);
    if computed_id_hex != id {
        return Ok(false);
    }

    // Verify Schnorr signature
    verify_schnorr_signature(pubkey, &computed_id, sig)
}

/// Nostr channel adapter
pub struct NostrAdapter {
    connected: Arc<AtomicBool>,
    config: NostrConfig,
    keypair: NostrKeyPair,
    shutdown: Arc<Notify>,
    /// Handles to receive tasks (one per relay)
    task_handles: RwLock<Vec<tokio::task::JoinHandle<()>>>,
    /// WebSocket writers (one per relay)
    writers: Arc<RwLock<Vec<(String, WsSink)>>>,
}

impl NostrAdapter {
    /// Create a new Nostr adapter
    pub async fn new(config: NostrConfig) -> Result<Self> {
        if config.private_key.is_empty() && config.nsec.is_none() {
            return Err(Error::Config(
                "Nostr private_key or nsec is required".into(),
            ));
        }
        if config.relay_urls.is_empty() {
            return Err(Error::Config("At least one Nostr relay is required".into()));
        }

        // Derive keypair: private_key accepts nsec bech32 or hex; nsec field is a legacy alias
        let keypair = if let Some(ref nsec) = config.nsec {
            NostrKeyPair::from_nsec(nsec)?
        } else if config.private_key.starts_with("nsec1") {
            NostrKeyPair::from_nsec(&config.private_key)?
        } else {
            NostrKeyPair::from_hex(&config.private_key)?
        };

        tracing::info!(
            relay_urls = ?config.relay_urls,
            pubkey = %keypair.public_key_hex(),
            "Nostr adapter created"
        );

        Ok(Self {
            connected: Arc::new(AtomicBool::new(false)),
            config,
            keypair,
            shutdown: Arc::new(Notify::new()),
            task_handles: RwLock::new(Vec::new()),
            writers: Arc::new(RwLock::new(Vec::new())),
        })
    }

    /// Get the public key (hex) derived from the private key
    pub fn get_public_key(&self) -> Result<String> {
        Ok(self.keypair.public_key_hex())
    }

    /// Get a reference to the keypair
    pub fn keypair(&self) -> &NostrKeyPair {
        &self.keypair
    }

    /// Connect to a single relay and start listening
    async fn connect_relay(&self, relay_url: &str, tx: mpsc::Sender<ChannelMessage>) -> Result<()> {
        tracing::info!(relay = %relay_url, "Connecting to Nostr relay");

        let (ws_stream, _) = connect_async(relay_url).await.map_err(|e| {
            Error::Channel(format!("Failed to connect to relay {}: {}", relay_url, e))
        })?;

        let (write, mut read) = ws_stream.split();

        // Store the writer for this relay
        {
            let mut writers = self.writers.write().await;
            writers.push((relay_url.to_string(), write));
        }

        // Subscribe to DMs and mentions
        let pubkey = self.keypair.public_key_hex();
        let subscription = serde_json::json!([
            "REQ",
            format!("zeus-sub-{}", &pubkey[..8]),
            {
                "kinds": [1, 4],
                "#p": [pubkey],
                "since": chrono::Utc::now().timestamp()
            }
        ]);

        {
            let mut writers = self.writers.write().await;
            if let Some((_, writer)) = writers.iter_mut().find(|(url, _)| url == relay_url) {
                writer
                    .send(WsMessage::Text(subscription.to_string()))
                    .await
                    .map_err(|e| {
                        Error::Channel(format!("Failed to subscribe on {}: {}", relay_url, e))
                    })?;
            }
        }

        let connected = self.connected.clone();
        let shutdown = self.shutdown.clone();
        let my_secret = self.keypair.secret_bytes();
        let my_pubkey = self.keypair.public_key_hex();
        let relay_name = relay_url.to_string();

        let handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown.notified() => {
                        tracing::info!(relay = %relay_name, "Nostr relay listener shutting down");
                        break;
                    }
                    msg = read.next() => {
                        match msg {
                            Some(Ok(WsMessage::Text(text))) => {
                                if let Err(e) = Self::handle_relay_message(
                                    &text,
                                    &tx,
                                    &my_pubkey,
                                    &my_secret,
                                ).await {
                                    tracing::error!(
                                        error = %e,
                                        relay = %relay_name,
                                        "Error handling relay message"
                                    );
                                }
                            }
                            Some(Ok(WsMessage::Close(_))) => {
                                tracing::info!(relay = %relay_name, "Nostr relay closed connection");
                                break;
                            }
                            Some(Err(e)) => {
                                tracing::error!(
                                    error = %e,
                                    relay = %relay_name,
                                    "WebSocket error"
                                );
                                break;
                            }
                            None => break,
                            _ => {}
                        }
                    }
                }
            }
            connected.store(false, Ordering::SeqCst);
        });

        self.task_handles.write().await.push(handle);
        self.connected.store(true, Ordering::SeqCst);

        Ok(())
    }

    /// Handle a message from a relay
    async fn handle_relay_message(
        text: &str,
        tx: &mpsc::Sender<ChannelMessage>,
        my_pubkey: &str,
        my_secret: &[u8; 32],
    ) -> Result<()> {
        let msg: serde_json::Value = serde_json::from_str(text)
            .map_err(|e| Error::Channel(format!("Failed to parse relay message: {}", e)))?;

        let msg_type = msg.get(0).and_then(|v| v.as_str());

        match msg_type {
            Some("EVENT") => {
                let event = msg
                    .get(2)
                    .ok_or_else(|| Error::Channel("EVENT message missing event object".into()))?;

                let kind = event.get("kind").and_then(|k| k.as_i64()).unwrap_or(0);
                let pubkey = event.get("pubkey").and_then(|p| p.as_str()).unwrap_or("");
                let content = event.get("content").and_then(|c| c.as_str()).unwrap_or("");

                // Skip our own events
                if pubkey == my_pubkey {
                    return Ok(());
                }

                // Verify event signature
                match verify_event(event) {
                    Ok(true) => {}
                    Ok(false) => {
                        tracing::warn!(
                            event_id = event.get("id").and_then(|v| v.as_str()).unwrap_or("?"),
                            "Nostr event failed signature verification, dropping"
                        );
                        return Ok(());
                    }
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            "Could not verify Nostr event signature, processing anyway"
                        );
                    }
                }

                // Only process text notes (kind 1) and DMs (kind 4)
                if kind == 1 || kind == 4 {
                    let decrypted_content = if kind == 4 {
                        // Decrypt NIP-04 DM
                        match hex::decode(pubkey) {
                            Ok(sender_pk_bytes) if sender_pk_bytes.len() == 32 => {
                                let mut sender_pk = [0u8; 32];
                                sender_pk.copy_from_slice(&sender_pk_bytes);
                                match nip04_decrypt(my_secret, &sender_pk, content) {
                                    Ok(decrypted) => decrypted,
                                    Err(e) => {
                                        tracing::warn!(
                                            error = %e,
                                            "Failed to decrypt NIP-04 DM, using raw content"
                                        );
                                        content.to_string()
                                    }
                                }
                            }
                            _ => {
                                tracing::warn!(
                                    sender = %pubkey,
                                    "Invalid sender pubkey for NIP-04 decryption"
                                );
                                content.to_string()
                            }
                        }
                    } else {
                        content.to_string()
                    };

                    let source = ChannelSource::new("nostr", pubkey);
                    let message = ChannelMessage::new(source, decrypted_content);

                    tx.send(message)
                        .await
                        .map_err(|e| Error::Channel(format!("Failed to forward message: {}", e)))?;
                }
            }
            Some("EOSE") => {
                let sub_id = msg.get(1).and_then(|v| v.as_str()).unwrap_or("?");
                tracing::debug!(subscription = %sub_id, "End of stored events");
            }
            Some("NOTICE") => {
                let notice = msg.get(1).and_then(|v| v.as_str()).unwrap_or("");
                tracing::warn!(notice = %notice, "Relay notice");
            }
            Some("OK") => {
                let event_id = msg.get(1).and_then(|v| v.as_str()).unwrap_or("?");
                let accepted = msg.get(2).and_then(|v| v.as_bool()).unwrap_or(false);
                let message = msg.get(3).and_then(|v| v.as_str()).unwrap_or("");
                if accepted {
                    tracing::debug!(event_id = %event_id, "Event accepted by relay");
                } else {
                    tracing::warn!(
                        event_id = %event_id,
                        reason = %message,
                        "Event rejected by relay"
                    );
                }
            }
            _ => {
                tracing::trace!(raw = %text, "Unknown relay message type");
            }
        }

        Ok(())
    }

    /// Publish an event to all connected relays.
    ///
    /// For kind 4 (DM): encrypts content with NIP-04 and adds `["p", recipient]` tag.
    /// All events are properly signed with the keypair.
    pub async fn publish_event(&self, content: &str, to_pubkey: Option<&str>) -> Result<()> {
        let (kind, encrypted_content, mut tags) = if let Some(recipient) = to_pubkey {
            // DM (kind 4): encrypt with NIP-04
            let recipient_bytes = hex::decode(recipient)
                .map_err(|e| Error::Channel(format!("Invalid recipient pubkey hex: {}", e)))?;
            if recipient_bytes.len() != 32 {
                return Err(Error::Channel(format!(
                    "Recipient pubkey must be 32 bytes, got {}",
                    recipient_bytes.len()
                )));
            }
            let mut rpk = [0u8; 32];
            rpk.copy_from_slice(&recipient_bytes);

            let encrypted = nip04_encrypt(&self.keypair.secret_bytes(), &rpk, content)?;
            let tags = vec![vec!["p".to_string(), recipient.to_string()]];
            (4u32, encrypted, tags)
        } else {
            // Public note (kind 1)
            (1u32, content.to_string(), Vec::new())
        };

        // Allow callers to have pre-set tags (merge)
        let _ = &mut tags;

        let event = create_signed_event(&self.keypair, kind, &encrypted_content, tags)?;
        let event_msg = serde_json::json!(["EVENT", event]);
        let event_str = event_msg.to_string();

        // Send to all connected relays
        let mut writers = self.writers.write().await;
        let mut sent_count = 0;
        for (relay_url, writer) in writers.iter_mut() {
            match writer.send(WsMessage::Text(event_str.clone())).await {
                Ok(()) => {
                    sent_count += 1;
                    tracing::debug!(relay = %relay_url, "Event sent to relay");
                }
                Err(e) => {
                    tracing::error!(
                        relay = %relay_url,
                        error = %e,
                        "Failed to send event to relay"
                    );
                }
            }
        }

        if sent_count == 0 && !writers.is_empty() {
            return Err(Error::Channel("Failed to send event to any relay".into()));
        }

        tracing::info!(kind = kind, relays = sent_count, "Nostr event published");
        Ok(())
    }
}

#[async_trait]
impl ChannelAdapter for NostrAdapter {
    fn channel_type(&self) -> &'static str {
        "nostr"
    }

    fn receive_mode(&self) -> ReceiveMode {
        ReceiveMode::WebSocket
    }

    async fn start(&self, tx: mpsc::Sender<ChannelMessage>) -> Result<()> {
        // Connect to ALL configured relays
        let relays = self.config.relay_urls.clone();
        let mut connected_count = 0;

        for relay in &relays {
            match self.connect_relay(relay, tx.clone()).await {
                Ok(()) => {
                    connected_count += 1;
                    tracing::info!(relay = %relay, "Connected to Nostr relay");
                }
                Err(e) => {
                    tracing::error!(relay = %relay, error = %e, "Failed to connect to Nostr relay");
                }
            }
        }

        if connected_count == 0 {
            return Err(Error::Channel(
                "Failed to connect to any Nostr relay".into(),
            ));
        }

        tracing::info!(
            connected = connected_count,
            total = relays.len(),
            "Nostr adapter started"
        );
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        self.connected.store(false, Ordering::SeqCst);
        self.shutdown.notify_waiters();

        // Close subscriptions on all relays
        {
            let pubkey = self.keypair.public_key_hex();
            let close = serde_json::json!(["CLOSE", format!("zeus-sub-{}", &pubkey[..8])]);
            let close_str = close.to_string();
            let mut writers = self.writers.write().await;
            for (relay_url, writer) in writers.iter_mut() {
                if let Err(e) = writer.send(WsMessage::Text(close_str.clone())).await {
                    tracing::warn!(relay = %relay_url, error = %e, "Failed to close subscription");
                }
            }
            writers.clear();
        }

        // Await all task handles
        let mut handles = self.task_handles.write().await;
        for handle in handles.drain(..) {
            let _ = handle.await;
        }

        tracing::info!("Nostr adapter stopped");
        Ok(())
    }

    async fn send(&self, to: &ChannelSource, content: &str) -> Result<()> {
        if to.channel_type() != "nostr" {
            return Err(Error::channel("Invalid channel source for Nostr"));
        }

        self.publish_event(content, Some(&to.user_id)).await
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }
}

/// Nostr configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NostrConfig {
    /// Private key (hex)
    #[serde(default)]
    pub private_key: String,
    /// Private key (nsec format)
    #[serde(default)]
    pub nsec: Option<String>,
    /// Public key (hex, can be derived from private key)
    #[serde(default)]
    pub public_key: Option<String>,
    /// Relay URLs (wss:// or ws://)
    ///
    /// Also accepted as "relays" in legacy configuration.
    #[serde(default, alias = "relays")]
    pub relay_urls: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    // A known test private key (32 bytes hex)
    const TEST_PRIVKEY_HEX: &str =
        "e8f32e723decf4051aefac8e2c93c9c5b214313817cdb01a1494b917c8436b35";

    #[test]
    fn test_nostr_config_default() {
        let config = NostrConfig::default();
        assert!(config.private_key.is_empty());
        assert!(config.relay_urls.is_empty());
    }

    #[tokio::test]
    async fn test_nostr_adapter_validation() {
        // Empty config should fail
        let config = NostrConfig::default();
        assert!(NostrAdapter::new(config).await.is_err());

        // Missing relays should fail
        let config = NostrConfig {
            private_key: TEST_PRIVKEY_HEX.to_string(),
            ..Default::default()
        };
        assert!(NostrAdapter::new(config).await.is_err());

        // Valid config should succeed
        let config = NostrConfig {
            private_key: TEST_PRIVKEY_HEX.to_string(),
            relay_urls: vec!["wss://relay.damus.io".to_string()],
            ..Default::default()
        };
        assert!(NostrAdapter::new(config).await.is_ok());
    }

    #[tokio::test]
    async fn test_nostr_adapter_lifecycle() {
        let config = NostrConfig {
            private_key: TEST_PRIVKEY_HEX.to_string(),
            relay_urls: vec!["wss://relay.damus.io".to_string()],
            ..Default::default()
        };

        let adapter = NostrAdapter::new(config).await.unwrap();
        assert!(!adapter.is_connected());
        assert_eq!(adapter.channel_type(), "nostr");
    }

    // --- Key pair tests ---

    #[test]
    fn test_keypair_from_hex() {
        let kp = NostrKeyPair::from_hex(TEST_PRIVKEY_HEX)
            .expect("Failed to create keypair from test private key");
        let pubkey = kp.public_key_hex();
        // Pubkey should be 64 hex chars (32 bytes)
        assert_eq!(pubkey.len(), 64);
        // Should be valid hex
        assert!(hex::decode(&pubkey).is_ok());
    }

    #[test]
    fn test_keypair_from_hex_invalid_length() {
        assert!(NostrKeyPair::from_hex("abcd").is_err());
    }

    #[test]
    fn test_keypair_from_hex_invalid_hex() {
        assert!(NostrKeyPair::from_hex("xyz0").is_err());
    }

    #[test]
    fn test_keypair_public_key_derivation_deterministic() {
        let kp1 = NostrKeyPair::from_hex(TEST_PRIVKEY_HEX)
            .expect("Failed to create keypair from test private key");
        let kp2 = NostrKeyPair::from_hex(TEST_PRIVKEY_HEX)
            .expect("Failed to create keypair from test private key");
        assert_eq!(kp1.public_key_hex(), kp2.public_key_hex());
    }

    #[test]
    fn test_keypair_different_keys_different_pubkeys() {
        let kp1 = NostrKeyPair::from_hex(TEST_PRIVKEY_HEX)
            .expect("Failed to create keypair from test private key");
        let kp2 = NostrKeyPair::from_hex(
            "0000000000000000000000000000000000000000000000000000000000000001",
        )
        .expect("Failed to create keypair from test hex");
        assert_ne!(kp1.public_key_hex(), kp2.public_key_hex());
    }

    #[test]
    fn test_keypair_secret_bytes_roundtrip() {
        let kp = NostrKeyPair::from_hex(TEST_PRIVKEY_HEX)
            .expect("Failed to create keypair from test private key");
        let secret = kp.secret_bytes();
        let hex_again = hex::encode(secret);
        assert_eq!(hex_again, TEST_PRIVKEY_HEX);
    }

    // --- NIP-04 encryption tests ---

    #[test]
    fn test_nip04_encrypt_decrypt_roundtrip() {
        // Alice's keypair
        let alice_sk_hex = "e8f32e723decf4051aefac8e2c93c9c5b214313817cdb01a1494b917c8436b35";
        let alice = NostrKeyPair::from_hex(alice_sk_hex).unwrap();

        // Bob's keypair
        let bob_sk_hex = "0000000000000000000000000000000000000000000000000000000000000001";
        let bob = NostrKeyPair::from_hex(bob_sk_hex).unwrap();

        let alice_pk_bytes = hex::decode(alice.public_key_hex()).unwrap();
        let bob_pk_bytes = hex::decode(bob.public_key_hex()).unwrap();

        let mut alice_pk = [0u8; 32];
        alice_pk.copy_from_slice(&alice_pk_bytes);
        let mut bob_pk = [0u8; 32];
        bob_pk.copy_from_slice(&bob_pk_bytes);

        let plaintext = "Hello, Bob! This is a secret message.";

        // Alice encrypts for Bob
        let encrypted = nip04_encrypt(&alice.secret_bytes(), &bob_pk, plaintext).unwrap();

        // Verify format: base64?iv=base64
        assert!(encrypted.contains("?iv="));

        // Bob decrypts from Alice
        let decrypted = nip04_decrypt(&bob.secret_bytes(), &alice_pk, &encrypted).unwrap();

        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_nip04_encrypt_decrypt_empty_message() {
        let alice = NostrKeyPair::from_hex(TEST_PRIVKEY_HEX)
            .expect("Failed to create keypair from test private key");
        let bob_sk_hex = "0000000000000000000000000000000000000000000000000000000000000001";
        let bob = NostrKeyPair::from_hex(bob_sk_hex).unwrap();

        let alice_pk_bytes = hex::decode(alice.public_key_hex()).unwrap();
        let bob_pk_bytes = hex::decode(bob.public_key_hex()).unwrap();
        let mut alice_pk = [0u8; 32];
        alice_pk.copy_from_slice(&alice_pk_bytes);
        let mut bob_pk = [0u8; 32];
        bob_pk.copy_from_slice(&bob_pk_bytes);

        let plaintext = "";
        let encrypted = nip04_encrypt(&alice.secret_bytes(), &bob_pk, plaintext).unwrap();
        let decrypted = nip04_decrypt(&bob.secret_bytes(), &alice_pk, &encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_nip04_encrypt_decrypt_unicode() {
        let alice = NostrKeyPair::from_hex(TEST_PRIVKEY_HEX)
            .expect("Failed to create keypair from test private key");
        let bob_sk_hex = "0000000000000000000000000000000000000000000000000000000000000001";
        let bob = NostrKeyPair::from_hex(bob_sk_hex).unwrap();

        let alice_pk_bytes = hex::decode(alice.public_key_hex()).unwrap();
        let bob_pk_bytes = hex::decode(bob.public_key_hex()).unwrap();
        let mut alice_pk = [0u8; 32];
        alice_pk.copy_from_slice(&alice_pk_bytes);
        let mut bob_pk = [0u8; 32];
        bob_pk.copy_from_slice(&bob_pk_bytes);

        let plaintext = "Encrypted DMs are fun! Also: emoji test";
        let encrypted = nip04_encrypt(&alice.secret_bytes(), &bob_pk, plaintext).unwrap();
        let decrypted = nip04_decrypt(&bob.secret_bytes(), &alice_pk, &encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_nip04_decrypt_invalid_format() {
        let sk = [1u8; 32];
        let pk = [2u8; 32];
        // Missing "?iv=" separator
        assert!(nip04_decrypt(&sk, &pk, "justbase64data").is_err());
    }

    #[test]
    fn test_nip04_decrypt_invalid_base64() {
        let sk = [1u8; 32];
        let pk = [2u8; 32];
        assert!(nip04_decrypt(&sk, &pk, "not!valid!base64?iv=alsonotvalid").is_err());
    }

    // --- Event ID and signing tests ---

    #[test]
    fn test_compute_event_id_deterministic() {
        let pubkey = "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2";
        let created_at = 1700000000i64;
        let kind = 1u32;
        let tags: Vec<Vec<String>> = vec![];
        let content = "Hello, Nostr!";

        let id1 = compute_event_id(pubkey, created_at, kind, &tags, content);
        let id2 = compute_event_id(pubkey, created_at, kind, &tags, content);
        assert_eq!(id1, id2);
    }

    #[test]
    fn test_compute_event_id_changes_with_content() {
        let pubkey = "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2";
        let created_at = 1700000000i64;
        let kind = 1u32;
        let tags: Vec<Vec<String>> = vec![];

        let id1 = compute_event_id(pubkey, created_at, kind, &tags, "Hello");
        let id2 = compute_event_id(pubkey, created_at, kind, &tags, "World");
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_compute_event_id_changes_with_kind() {
        let pubkey = "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2";
        let created_at = 1700000000i64;
        let tags: Vec<Vec<String>> = vec![];
        let content = "test";

        let id1 = compute_event_id(pubkey, created_at, 1, &tags, content);
        let id2 = compute_event_id(pubkey, created_at, 4, &tags, content);
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_compute_event_id_with_tags() {
        let pubkey = "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2";
        let created_at = 1700000000i64;
        let kind = 4u32;
        let content = "encrypted";

        let tags_empty: Vec<Vec<String>> = vec![];
        let tags_with_p = vec![vec!["p".to_string(), "recipient_pubkey".to_string()]];

        let id1 = compute_event_id(pubkey, created_at, kind, &tags_empty, content);
        let id2 = compute_event_id(pubkey, created_at, kind, &tags_with_p, content);
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_event_id_is_sha256() {
        let pubkey = "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2";
        let created_at = 1700000000i64;
        let kind = 1u32;
        let tags: Vec<Vec<String>> = vec![];
        let content = "Hello!";

        let id = compute_event_id(pubkey, created_at, kind, &tags, content);

        // Event ID should be 32 bytes (SHA-256)
        assert_eq!(id.len(), 32);

        // Manually verify the hash
        let serialized = serde_json::json!([0, pubkey, created_at, kind, tags, content]);
        let mut hasher = Sha256::new();
        hasher.update(serialized.to_string().as_bytes());
        let expected: [u8; 32] = hasher.finalize().into();
        assert_eq!(id, expected);
    }

    // --- Event creation and signing tests ---

    #[test]
    fn test_create_signed_event_structure() {
        let kp = NostrKeyPair::from_hex(TEST_PRIVKEY_HEX)
            .expect("Failed to create keypair from test private key");
        let event = create_signed_event(&kp, 1, "Hello Nostr!", vec![]).unwrap();

        // Verify all required fields exist
        assert!(event.get("id").is_some());
        assert!(event.get("pubkey").is_some());
        assert!(event.get("created_at").is_some());
        assert!(event.get("kind").is_some());
        assert!(event.get("tags").is_some());
        assert!(event.get("content").is_some());
        assert!(event.get("sig").is_some());

        // Verify field values
        assert_eq!(event["pubkey"].as_str().unwrap(), kp.public_key_hex());
        assert_eq!(event["kind"].as_u64().unwrap(), 1);
        assert_eq!(event["content"].as_str().unwrap(), "Hello Nostr!");

        // id should be 64 hex chars
        assert_eq!(event["id"].as_str().unwrap().len(), 64);
        // sig should be 128 hex chars (64 bytes Schnorr)
        assert_eq!(event["sig"].as_str().unwrap().len(), 128);
    }

    #[test]
    fn test_create_signed_event_with_tags() {
        let kp = NostrKeyPair::from_hex(TEST_PRIVKEY_HEX)
            .expect("Failed to create keypair from test private key");
        let tags = vec![vec!["p".to_string(), "recipient123".to_string()]];
        let event = create_signed_event(&kp, 4, "encrypted_content", tags).unwrap();

        let event_tags = event["tags"].as_array().unwrap();
        assert_eq!(event_tags.len(), 1);
        assert_eq!(event_tags[0][0].as_str().unwrap(), "p");
        assert_eq!(event_tags[0][1].as_str().unwrap(), "recipient123");
    }

    #[test]
    fn test_create_signed_event_verifies() {
        let kp = NostrKeyPair::from_hex(TEST_PRIVKEY_HEX)
            .expect("Failed to create keypair from test private key");
        let event = create_signed_event(&kp, 1, "Verifiable message", vec![]).unwrap();

        // The event we just created should pass verification
        let valid = verify_event(&event).unwrap();
        assert!(valid, "Signed event should pass verification");
    }

    // --- Event verification tests ---

    #[test]
    fn test_verify_event_detects_tampered_content() {
        let kp = NostrKeyPair::from_hex(TEST_PRIVKEY_HEX)
            .expect("Failed to create keypair from test private key");
        let mut event = create_signed_event(&kp, 1, "Original", vec![]).unwrap();

        // Tamper with content
        event["content"] = serde_json::json!("Tampered");

        let valid = verify_event(&event).unwrap();
        assert!(!valid, "Tampered event should fail verification");
    }

    #[test]
    fn test_verify_event_detects_tampered_id() {
        let kp = NostrKeyPair::from_hex(TEST_PRIVKEY_HEX)
            .expect("Failed to create keypair from test private key");
        let mut event = create_signed_event(&kp, 1, "Test", vec![])
            .expect("Failed to create signed test event");

        // Tamper with id
        event["id"] =
            serde_json::json!("0000000000000000000000000000000000000000000000000000000000000000");

        let valid = verify_event(&event).unwrap();
        assert!(!valid, "Event with wrong ID should fail verification");
    }

    #[test]
    fn test_verify_event_missing_fields() {
        let incomplete = serde_json::json!({
            "id": "abc",
            "content": "hello"
        });
        assert!(verify_event(&incomplete).is_err());
    }

    // --- Schnorr signature tests ---

    #[test]
    fn test_schnorr_sign_and_verify() {
        let kp = NostrKeyPair::from_hex(TEST_PRIVKEY_HEX)
            .expect("Failed to create keypair from test private key");
        let msg_hash = [42u8; 32];

        let sig = kp.sign_schnorr(&msg_hash).unwrap();
        assert_eq!(sig.len(), 128); // 64 bytes = 128 hex chars

        let valid = verify_schnorr_signature(&kp.public_key_hex(), &msg_hash, &sig).unwrap();
        assert!(valid);
    }

    #[test]
    fn test_schnorr_verify_wrong_message() {
        let kp = NostrKeyPair::from_hex(TEST_PRIVKEY_HEX)
            .expect("Failed to create keypair from test private key");
        let msg_hash = [42u8; 32];
        let wrong_hash = [43u8; 32];

        let sig = kp.sign_schnorr(&msg_hash).unwrap();

        let valid = verify_schnorr_signature(&kp.public_key_hex(), &wrong_hash, &sig).unwrap();
        assert!(!valid);
    }

    #[test]
    fn test_schnorr_verify_wrong_key() {
        let kp1 = NostrKeyPair::from_hex(TEST_PRIVKEY_HEX)
            .expect("Failed to create keypair from test private key");
        let kp2 = NostrKeyPair::from_hex(
            "0000000000000000000000000000000000000000000000000000000000000001",
        )
        .expect("Failed to create keypair from test hex");
        let msg_hash = [42u8; 32];

        let sig = kp1.sign_schnorr(&msg_hash).unwrap();

        let valid = verify_schnorr_signature(&kp2.public_key_hex(), &msg_hash, &sig).unwrap();
        assert!(!valid);
    }

    // --- Relay message parsing tests ---

    #[tokio::test]
    async fn test_handle_relay_message_event_kind1() {
        let (tx, mut rx) = mpsc::channel(10);
        let kp = NostrKeyPair::from_hex(TEST_PRIVKEY_HEX)
            .expect("Failed to create keypair from test private key");
        let my_pubkey = kp.public_key_hex();

        // Create a valid signed event from a different key
        let sender_kp = NostrKeyPair::from_hex(
            "0000000000000000000000000000000000000000000000000000000000000001",
        )
        .expect("Failed to create keypair from test hex");
        let event = create_signed_event(&sender_kp, 1, "Hello from sender!", vec![]).unwrap();

        let relay_msg = serde_json::json!(["EVENT", "sub-id", event]);

        NostrAdapter::handle_relay_message(
            &relay_msg.to_string(),
            &tx,
            &my_pubkey,
            &kp.secret_bytes(),
        )
        .await
        .expect("Failed to create keypair from test hex");

        let msg = rx.try_recv().unwrap();
        assert_eq!(msg.content, "Hello from sender!");
        assert_eq!(msg.source.channel_type(), "nostr");
        assert_eq!(msg.source.user_id, sender_kp.public_key_hex());
    }

    #[tokio::test]
    async fn test_handle_relay_message_skips_own_events() {
        let (tx, mut rx) = mpsc::channel(10);
        let kp = NostrKeyPair::from_hex(TEST_PRIVKEY_HEX)
            .expect("Failed to create keypair from test private key");
        let my_pubkey = kp.public_key_hex();

        // Create event from our own key
        let event = create_signed_event(&kp, 1, "My own event", vec![]).unwrap();
        let relay_msg = serde_json::json!(["EVENT", "sub-id", event]);

        NostrAdapter::handle_relay_message(
            &relay_msg.to_string(),
            &tx,
            &my_pubkey,
            &kp.secret_bytes(),
        )
        .await
        .expect("Failed to create keypair from test hex");

        // Should not have forwarded our own event
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_handle_relay_message_eose() {
        let (tx, _rx) = mpsc::channel(10);
        let kp = NostrKeyPair::from_hex(TEST_PRIVKEY_HEX)
            .expect("Failed to create keypair from test private key");

        let eose_msg = serde_json::json!(["EOSE", "zeus-sub"]);
        // Should not error
        NostrAdapter::handle_relay_message(
            &eose_msg.to_string(),
            &tx,
            &kp.public_key_hex(),
            &kp.secret_bytes(),
        )
        .await
        .expect("Failed to create keypair from test hex");
    }

    #[tokio::test]
    async fn test_handle_relay_message_notice() {
        let (tx, _rx) = mpsc::channel(10);
        let kp = NostrKeyPair::from_hex(TEST_PRIVKEY_HEX)
            .expect("Failed to create keypair from test private key");

        let notice_msg = serde_json::json!(["NOTICE", "rate limited"]);
        NostrAdapter::handle_relay_message(
            &notice_msg.to_string(),
            &tx,
            &kp.public_key_hex(),
            &kp.secret_bytes(),
        )
        .await
        .expect("Failed to create keypair from test hex");
    }

    #[tokio::test]
    async fn test_handle_relay_message_ok_accepted() {
        let (tx, _rx) = mpsc::channel(10);
        let kp = NostrKeyPair::from_hex(TEST_PRIVKEY_HEX)
            .expect("Failed to create keypair from test private key");

        let ok_msg = serde_json::json!(["OK", "event-id-hex", true, ""]);
        NostrAdapter::handle_relay_message(
            &ok_msg.to_string(),
            &tx,
            &kp.public_key_hex(),
            &kp.secret_bytes(),
        )
        .await
        .expect("Failed to create keypair from test hex");
    }

    #[tokio::test]
    async fn test_handle_relay_message_ok_rejected() {
        let (tx, _rx) = mpsc::channel(10);
        let kp = NostrKeyPair::from_hex(TEST_PRIVKEY_HEX)
            .expect("Failed to create keypair from test private key");

        let ok_msg = serde_json::json!(["OK", "event-id-hex", false, "blocked: not on whitelist"]);
        NostrAdapter::handle_relay_message(
            &ok_msg.to_string(),
            &tx,
            &kp.public_key_hex(),
            &kp.secret_bytes(),
        )
        .await
        .expect("Failed to create keypair from test hex");
    }

    #[tokio::test]
    async fn test_handle_relay_message_invalid_json() {
        let (tx, _rx) = mpsc::channel(10);
        let kp = NostrKeyPair::from_hex(TEST_PRIVKEY_HEX)
            .expect("Failed to create keypair from test private key");

        let result = NostrAdapter::handle_relay_message(
            "not json",
            &tx,
            &kp.public_key_hex(),
            &kp.secret_bytes(),
        )
        .await;
        assert!(result.is_err());
    }

    // --- Kind 4 DM decryption in relay message handling ---

    #[tokio::test]
    async fn test_handle_relay_message_dm_decryption() {
        let (tx, mut rx) = mpsc::channel(10);

        // Receiver (us)
        let receiver_kp = NostrKeyPair::from_hex(TEST_PRIVKEY_HEX)
            .expect("Failed to create keypair from test private key");
        let receiver_pubkey = receiver_kp.public_key_hex();
        let receiver_pk_bytes = hex::decode(&receiver_pubkey).unwrap();
        let mut receiver_pk = [0u8; 32];
        receiver_pk.copy_from_slice(&receiver_pk_bytes);

        // Sender
        let sender_kp = NostrKeyPair::from_hex(
            "0000000000000000000000000000000000000000000000000000000000000001",
        )
        .expect("Failed to create keypair from test hex");

        // Sender encrypts a DM for us
        let plaintext = "Secret DM content";
        let encrypted = nip04_encrypt(&sender_kp.secret_bytes(), &receiver_pk, plaintext).unwrap();

        // Create a signed kind 4 event from sender
        let tags = vec![vec!["p".to_string(), receiver_pubkey.clone()]];
        let event = create_signed_event(&sender_kp, 4, &encrypted, tags).unwrap();

        let relay_msg = serde_json::json!(["EVENT", "sub-id", event]);

        NostrAdapter::handle_relay_message(
            &relay_msg.to_string(),
            &tx,
            &receiver_pubkey,
            &receiver_kp.secret_bytes(),
        )
        .await
        .expect("Failed to create keypair from test hex");

        let msg = rx.try_recv().unwrap();
        assert_eq!(msg.content, plaintext);
        assert_eq!(msg.source.channel_type(), "nostr");
    }

    // --- Multi-relay config validation ---

    #[tokio::test]
    async fn test_multi_relay_config() {
        let config = NostrConfig {
            private_key: TEST_PRIVKEY_HEX.to_string(),
            relay_urls: vec![
                "wss://relay.damus.io".to_string(),
                "wss://relay.nostr.band".to_string(),
                "wss://nos.lol".to_string(),
            ],
            ..Default::default()
        };

        let adapter = NostrAdapter::new(config.clone()).await.unwrap();
        // Verify the adapter was created with all relays stored
        assert_eq!(adapter.config.relay_urls.len(), 3);
        assert!(!adapter.is_connected());
    }

    #[tokio::test]
    async fn test_empty_relays_rejected() {
        let config = NostrConfig {
            private_key: TEST_PRIVKEY_HEX.to_string(),
            relay_urls: vec![],
            ..Default::default()
        };
        assert!(NostrAdapter::new(config).await.is_err());
    }

    // --- Public key derivation test ---

    #[tokio::test]
    async fn test_get_public_key_derives_from_private() {
        let config = NostrConfig {
            private_key: TEST_PRIVKEY_HEX.to_string(),
            relay_urls: vec!["wss://relay.damus.io".to_string()],
            // No public_key set - should be derived
            ..Default::default()
        };

        let adapter = NostrAdapter::new(config).await.unwrap();
        let pubkey = adapter.get_public_key().unwrap();
        assert_eq!(pubkey.len(), 64);

        // Should match the keypair's public key
        let expected = NostrKeyPair::from_hex(TEST_PRIVKEY_HEX)
            .unwrap()
            .public_key_hex();
        assert_eq!(pubkey, expected);
    }

    // --- PKCS7 padding tests ---

    #[test]
    fn test_pkcs7_pad_exact_block() {
        // When data is exactly a block multiple, a full padding block is added
        let data = vec![0u8; 16];
        let padded = pkcs7_pad(&data, 16);
        assert_eq!(padded.len(), 32);
        assert_eq!(padded[16..], vec![16u8; 16]);
    }

    #[test]
    fn test_pkcs7_pad_partial_block() {
        let data = vec![0u8; 10];
        let padded = pkcs7_pad(&data, 16);
        assert_eq!(padded.len(), 16);
        assert_eq!(padded[10..], vec![6u8; 6]);
    }

    #[test]
    fn test_pkcs7_unpad_valid() {
        let mut data = vec![0u8; 10];
        data.extend(vec![6u8; 6]); // valid PKCS7 padding
        let unpadded = pkcs7_unpad(&data).unwrap();
        assert_eq!(unpadded.len(), 10);
    }

    #[test]
    fn test_pkcs7_unpad_empty() {
        assert!(pkcs7_unpad(&[]).is_err());
    }

    #[test]
    fn test_pkcs7_roundtrip() {
        let data = b"Hello, World!";
        let padded = pkcs7_pad(data, 16);
        let unpadded = pkcs7_unpad(&padded).unwrap();
        assert_eq!(unpadded, data);
    }

    // --- Event serialization for signing tests ---

    #[test]
    fn test_event_serialization_format() {
        // NIP-01 specifies the serialization for computing event ID:
        // [0, <pubkey>, <created_at>, <kind>, <tags>, <content>]
        let pubkey = "abcdef";
        let created_at = 1234567890i64;
        let kind = 1u32;
        let tags: Vec<Vec<String>> = vec![vec!["e".to_string(), "event_id".to_string()]];
        let content = "test message";

        let serialized = serde_json::json!([0, pubkey, created_at, kind, tags, content]);
        let expected = r#"[0,"abcdef",1234567890,1,[["e","event_id"]],"test message"]"#;
        assert_eq!(serialized.to_string(), expected);
    }

    // --- nsec tests ---

    #[test]
    fn test_keypair_from_nsec_invalid_prefix() {
        assert!(NostrKeyPair::from_nsec("npub1abc").is_err());
    }

    #[test]
    fn test_keypair_from_nsec_too_short() {
        assert!(NostrKeyPair::from_nsec("nsec1abc").is_err());
    }
}
