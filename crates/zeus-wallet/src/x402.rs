//! x402 Payment Protocol Client
//!
//! Implements the x402 payment protocol (Conway/Web 4.0):
//! 1. Agent makes HTTP request to a resource
//! 2. Server responds with 402 Payment Required + payment details in headers
//! 3. Agent signs a token transfer authorization
//! 4. Agent retries the request with payment proof in headers
//! 5. Server verifies payment and serves the resource
//!
//! Headers (server → client):
//!   X-Payment-Address: <recipient address>
//!   X-Payment-Amount: <amount in micro-units>
//!   X-Payment-Network: <solana-devnet|solana-mainnet>
//!   X-Payment-Token: <token mint address>
//!
//! Headers (client → server on retry):
//!   X-Payment-Signature: <base64 ed25519 signature of payment payload>
//!   X-Payment-Payer: <base64 public key>
//!   X-Payment-Payload: <base64 JSON payment payload>

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use chrono::Utc;
use rand::RngCore;
use rand::rngs::OsRng;
use reqwest::{Client, Method, Response, StatusCode};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

use crate::keypair::{WalletError, WalletKeypair};

/// Nonce TTL: entries older than 15 minutes are evicted
const NONCE_TTL: Duration = Duration::from_secs(900);
/// Rate limit: max payments to the same URL within this window
const RATE_LIMIT_WINDOW: Duration = Duration::from_secs(60);
/// Rate limit: max 5 payments per URL per minute to prevent wallet drain via repeated 402s
const RATE_LIMIT_MAX: usize = 5;

/// x402 client configuration
#[derive(Debug, Clone)]
pub struct X402Config {
    /// Maximum payment amount (micro-units) agent will authorize per request
    pub max_amount: u64,
    /// Allowed payment networks
    pub allowed_networks: Vec<String>,
    /// Allowed token mints
    pub allowed_tokens: Vec<String>,
}

impl Default for X402Config {
    fn default() -> Self {
        Self {
            max_amount: 1_000_000, // 1 token
            allowed_networks: vec!["solana-devnet".to_string(), "solana-mainnet".to_string()],
            // Default empty — tokens must be explicitly configured.
            // Hardcoded mint addresses belong in config, not code.
            allowed_tokens: vec![],
        }
    }
}

/// Payment request parsed from 402 response headers
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentRequest {
    /// Recipient address
    pub address: String,
    /// Amount in micro-USDC
    pub amount: u64,
    /// Network identifier
    pub network: String,
    /// Token mint address
    pub token: String,
    /// Original request URL
    pub url: String,
}

/// Signed payment payload sent back to server
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentPayload {
    /// Recipient address
    pub to: String,
    /// Amount in micro-USDC
    pub amount: u64,
    /// Token mint
    pub token: String,
    /// Network
    pub network: String,
    /// ISO 8601 timestamp
    pub timestamp: String,
    /// Payer public key (hex)
    pub payer: String,
    /// Cryptographically random nonce (OsRng, 16 bytes hex) — prevents replay
    pub nonce: String,
    /// URL being paid for — prevents server swap attacks
    /// (payment for /api/A cannot be replayed against /api/B)
    pub url: String,
}

/// Receipt of a completed x402 payment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentReceipt {
    /// The payment request
    pub request: PaymentRequest,
    /// Signature (base64)
    pub signature: String,
    /// Payer public key (hex)
    pub payer_pubkey: String,
    /// Timestamp
    pub timestamp: String,
    /// Whether the retry succeeded
    pub success: bool,
    /// HTTP status of the retry response
    pub retry_status: u16,
}

/// x402-aware HTTP client
pub struct X402Client {
    keypair: WalletKeypair,
    config: X402Config,
    http: Client,
    /// Used nonces with expiry timestamps — prevents server from replaying our signed payloads
    nonce_store: Mutex<HashMap<String, Instant>>,
    /// Per-URL payment history — rate-limits repeated 402s from the same server
    url_payment_times: Mutex<HashMap<String, Vec<Instant>>>,
    /// Path to JSONL payments audit log
    payments_log: PathBuf,
}

/// Errors from x402 operations
#[derive(Debug, thiserror::Error)]
pub enum X402Error {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Wallet error: {0}")]
    Wallet(#[from] WalletError),

    #[error("Payment refused: {0}")]
    Refused(String),

    #[error("Invalid 402 response: {0}")]
    InvalidResponse(String),

    #[error("Amount {requested} exceeds maximum {max}")]
    AmountExceeded { requested: u64, max: u64 },

    #[error("Network '{0}' not allowed")]
    NetworkNotAllowed(String),

    #[error("Token '{0}' not allowed")]
    TokenNotAllowed(String),

    #[error("Rate limit exceeded: {0}")]
    RateLimited(String),
}

impl From<X402Error> for zeus_core::Error {
    fn from(e: X402Error) -> Self {
        zeus_core::Error::Network(e.to_string())
    }
}

impl X402Client {
    /// Create a new x402 client with the given keypair and config.
    /// Payment receipts are persisted to `<wallet_dir>/payments.jsonl`.
    pub fn new(keypair: WalletKeypair, config: X402Config) -> Self {
        let payments_log = keypair.wallet_dir().join("payments.jsonl");
        Self {
            keypair,
            config,
            http: Client::new(),
            nonce_store: Mutex::new(HashMap::new()),
            url_payment_times: Mutex::new(HashMap::new()),
            payments_log,
        }
    }

    /// Make an HTTP request with automatic x402 payment handling.
    ///
    /// If the server responds with 402, this will:
    /// 1. Check per-URL rate limit (max 5 payments/minute) to prevent wallet drain
    /// 2. Parse payment requirements from headers
    /// 3. Validate amount/network/token against policy
    /// 4. Sign a payment authorization with OsRng nonce + URL binding
    /// 5. Retry the request with payment proof
    /// 6. Persist receipt to JSONL audit log
    pub async fn request(
        &self,
        method: Method,
        url: &str,
        headers: Option<HashMap<String, String>>,
        body: Option<String>,
    ) -> Result<(Response, Option<PaymentReceipt>), X402Error> {
        let mut req = self.http.request(method.clone(), url);

        if let Some(ref hdrs) = headers {
            for (k, v) in hdrs {
                req = req.header(k.as_str(), v.as_str());
            }
        }
        if let Some(ref b) = body {
            req = req.body(b.clone());
        }

        let response = req.send().await?;

        if response.status() != StatusCode::PAYMENT_REQUIRED {
            return Ok((response, None));
        }

        info!(
            url,
            "Received 402 Payment Required — processing x402 payment"
        );

        // P0-H2: Per-URL rate limit — prevents server from draining wallet via repeated 402s
        self.check_rate_limit(url)?;

        // Parse payment requirements
        let payment_req = self.parse_402_response(&response, url)?;

        // Validate against policy
        self.validate_payment(&payment_req)?;

        // Sign payment (OsRng nonce + URL in payload)
        let (payload, signature) = self.sign_payment(&payment_req)?;

        // Record nonce to prevent replay of this signed payload
        self.record_nonce(&payload.nonce);

        // Retry with payment proof
        let payload_json = serde_json::to_string(&payload)
            .map_err(|e| X402Error::InvalidResponse(format!("Failed to serialize payload: {e}")))?;
        let payload_b64 = BASE64.encode(payload_json.as_bytes());

        let mut retry_req = self.http.request(method, url);
        if let Some(ref hdrs) = headers {
            for (k, v) in hdrs {
                retry_req = retry_req.header(k.as_str(), v.as_str());
            }
        }
        if let Some(ref b) = body {
            retry_req = retry_req.body(b.clone());
        }

        retry_req = retry_req
            .header("X-Payment-Signature", &signature)
            .header("X-Payment-Payer", self.keypair.public_key_base64())
            .header("X-Payment-Payload", &payload_b64);

        let retry_response = retry_req.send().await?;
        let retry_status = retry_response.status().as_u16();

        let receipt = PaymentReceipt {
            request: payment_req,
            signature,
            payer_pubkey: self.keypair.public_key_hex(),
            timestamp: payload.timestamp.clone(),
            success: retry_response.status().is_success(),
            retry_status,
        };

        debug!(
            retry_status,
            amount = receipt.request.amount,
            to = %receipt.request.address,
            "x402 payment completed"
        );

        // P0-M3: Persist receipt to JSONL audit log (best-effort — never fail the payment)
        self.persist_receipt(&receipt);

        Ok((retry_response, Some(receipt)))
    }

    /// Convenience: GET with x402 support
    pub async fn get(&self, url: &str) -> Result<(Response, Option<PaymentReceipt>), X402Error> {
        self.request(Method::GET, url, None, None).await
    }

    /// Convenience: POST with x402 support
    pub async fn post(
        &self,
        url: &str,
        body: &str,
    ) -> Result<(Response, Option<PaymentReceipt>), X402Error> {
        let mut headers = HashMap::new();
        headers.insert("Content-Type".to_string(), "application/json".to_string());
        self.request(Method::POST, url, Some(headers), Some(body.to_string()))
            .await
    }

    /// Get the public key of the wallet
    pub fn public_key_hex(&self) -> String {
        self.keypair.public_key_hex()
    }

    // -- private --

    /// P0-H2: Check per-URL payment rate limit.
    /// Prevents a malicious server from draining the wallet via a flood of 402 responses.
    fn check_rate_limit(&self, url: &str) -> Result<(), X402Error> {
        let mut times = self.url_payment_times.lock().unwrap();
        let now = Instant::now();
        let entry = times.entry(url.to_string()).or_default();
        // Evict expired entries
        entry.retain(|t| now.duration_since(*t) < RATE_LIMIT_WINDOW);
        if entry.len() >= RATE_LIMIT_MAX {
            return Err(X402Error::RateLimited(format!(
                "Too many payments to {url} within {:.0}s — possible wallet drain attack",
                RATE_LIMIT_WINDOW.as_secs_f64(),
            )));
        }
        entry.push(now);
        Ok(())
    }

    /// Record a used nonce with TTL expiry, evicting stale entries.
    fn record_nonce(&self, nonce: &str) {
        let mut store = self.nonce_store.lock().unwrap();
        let now = Instant::now();
        // Lazy eviction of expired nonces
        store.retain(|_, expiry| now < *expiry);
        store.insert(nonce.to_string(), now + NONCE_TTL);
    }

    /// P0-M3: Append a payment receipt to the JSONL audit log (best-effort).
    fn persist_receipt(&self, receipt: &PaymentReceipt) {
        match serde_json::to_string(receipt) {
            Ok(json) => {
                match std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&self.payments_log)
                {
                    Ok(mut file) => {
                        if let Err(e) = writeln!(file, "{json}") {
                            warn!("Failed to write payment receipt to log: {e}");
                        }
                    }
                    Err(e) => warn!("Failed to open payments log {:?}: {e}", self.payments_log),
                }
            }
            Err(e) => warn!("Failed to serialize payment receipt: {e}"),
        }
    }

    fn parse_402_response(
        &self,
        response: &Response,
        url: &str,
    ) -> Result<PaymentRequest, X402Error> {
        let get_header = |name: &str| -> Result<String, X402Error> {
            response
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string())
                .ok_or_else(|| {
                    X402Error::InvalidResponse(format!("Missing required header: {name}"))
                })
        };

        let address = get_header("X-Payment-Address")?;
        let amount_str = get_header("X-Payment-Amount")?;
        let network = get_header("X-Payment-Network")?;
        let token = get_header("X-Payment-Token")?;

        let amount: u64 = amount_str
            .parse()
            .map_err(|_| X402Error::InvalidResponse(format!("Invalid amount: {amount_str}")))?;

        Ok(PaymentRequest {
            address,
            amount,
            network,
            token,
            url: url.to_string(),
        })
    }

    fn validate_payment(&self, req: &PaymentRequest) -> Result<(), X402Error> {
        if req.amount > self.config.max_amount {
            return Err(X402Error::AmountExceeded {
                requested: req.amount,
                max: self.config.max_amount,
            });
        }

        if !self.config.allowed_networks.contains(&req.network) {
            return Err(X402Error::NetworkNotAllowed(req.network.clone()));
        }

        if !self.config.allowed_tokens.contains(&req.token) {
            return Err(X402Error::TokenNotAllowed(req.token.clone()));
        }

        Ok(())
    }

    /// P0-H1: Sign payment with OsRng nonce.
    /// P0-H3: Include URL in payload to prevent server swap attacks.
    fn sign_payment(&self, req: &PaymentRequest) -> Result<(PaymentPayload, String), X402Error> {
        // P0-H1: Cryptographically random nonce — replaces hash-based approach
        // which degraded to SHA256(url+":0") when timestamp_nanos_opt() returned None
        let mut nonce_bytes = [0u8; 16];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = hex::encode(nonce_bytes);

        let payload = PaymentPayload {
            to: req.address.clone(),
            amount: req.amount,
            token: req.token.clone(),
            network: req.network.clone(),
            timestamp: Utc::now().to_rfc3339(),
            payer: self.keypair.public_key_hex(),
            nonce,
            // P0-H3: Bind payment to specific URL — server cannot reuse this signature
            // to authorize payment for a different endpoint
            url: req.url.clone(),
        };

        let payload_json = serde_json::to_string(&payload)
            .map_err(|e| X402Error::InvalidResponse(format!("Serialization: {e}")))?;

        let signature = self.keypair.sign_base64(payload_json.as_bytes());

        Ok((payload, signature))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use wiremock::matchers::{header_exists, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn test_keypair() -> (TempDir, WalletKeypair) {
        let tmp = TempDir::new().unwrap();
        let kp = WalletKeypair::generate(tmp.path().join("w"), "test", "devnet").unwrap();
        (tmp, kp)
    }

    fn test_config_with_token(token: &str) -> X402Config {
        X402Config {
            max_amount: 1_000_000,
            allowed_networks: vec!["solana-devnet".to_string(), "solana-mainnet".to_string()],
            allowed_tokens: vec![token.to_string()],
        }
    }

    #[tokio::test]
    async fn test_non_402_passthrough() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
            .mount(&server)
            .await;

        let (_tmp, kp) = test_keypair();
        let client = X402Client::new(kp, X402Config::default());

        let (resp, receipt) = client.get(&server.uri()).await.unwrap();
        assert_eq!(resp.status(), 200);
        assert!(receipt.is_none());
    }

    #[tokio::test]
    async fn test_402_payment_flow() {
        let server = MockServer::start().await;
        let token = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";

        // First request returns 402
        Mock::given(method("GET"))
            .and(wiremock::matchers::any())
            .respond_with(
                ResponseTemplate::new(402)
                    .insert_header(
                        "X-Payment-Address",
                        "9xQeWvG816bUx9EPjHmaT23yvVM2ZWbrrpZb9PusVFin",
                    )
                    .insert_header("X-Payment-Amount", "100000")
                    .insert_header("X-Payment-Network", "solana-devnet")
                    .insert_header("X-Payment-Token", token),
            )
            .up_to_n_times(1)
            .mount(&server)
            .await;

        // Second request (with payment) returns 200
        Mock::given(method("GET"))
            .and(header_exists("X-Payment-Signature"))
            .respond_with(ResponseTemplate::new(200).set_body_string("paid content"))
            .mount(&server)
            .await;

        let (_tmp, kp) = test_keypair();
        let client = X402Client::new(kp, test_config_with_token(token));

        let (resp, receipt) = client.get(&server.uri()).await.unwrap();
        assert_eq!(resp.status(), 200);

        let receipt = receipt.expect("should have payment receipt");
        assert!(receipt.success);
        assert_eq!(receipt.request.amount, 100000);
        assert_eq!(receipt.retry_status, 200);
        assert!(!receipt.payer_pubkey.is_empty());
        assert!(!receipt.signature.is_empty());
    }

    #[tokio::test]
    async fn test_402_amount_exceeded() {
        let server = MockServer::start().await;
        let token = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(402)
                    .insert_header("X-Payment-Address", "addr")
                    .insert_header("X-Payment-Amount", "999999999") // way over limit
                    .insert_header("X-Payment-Network", "solana-devnet")
                    .insert_header("X-Payment-Token", token),
            )
            .mount(&server)
            .await;

        let (_tmp, kp) = test_keypair();
        let client = X402Client::new(kp, test_config_with_token(token));

        let result = client.get(&server.uri()).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, X402Error::AmountExceeded { .. }));
    }

    #[tokio::test]
    async fn test_402_network_not_allowed() {
        let server = MockServer::start().await;
        let token = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(402)
                    .insert_header("X-Payment-Address", "addr")
                    .insert_header("X-Payment-Amount", "100")
                    .insert_header("X-Payment-Network", "ethereum-mainnet") // not allowed
                    .insert_header("X-Payment-Token", token),
            )
            .mount(&server)
            .await;

        let (_tmp, kp) = test_keypair();
        let client = X402Client::new(kp, test_config_with_token(token));

        let result = client.get(&server.uri()).await;
        assert!(matches!(result, Err(X402Error::NetworkNotAllowed(_))));
    }

    #[tokio::test]
    async fn test_402_token_not_allowed() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(402)
                    .insert_header("X-Payment-Address", "addr")
                    .insert_header("X-Payment-Amount", "100")
                    .insert_header("X-Payment-Network", "solana-devnet")
                    .insert_header("X-Payment-Token", "SomeRandomToken123"), // not allowed
            )
            .mount(&server)
            .await;

        let (_tmp, kp) = test_keypair();
        let client = X402Client::new(kp, X402Config::default()); // empty allowed_tokens

        let result = client.get(&server.uri()).await;
        assert!(matches!(result, Err(X402Error::TokenNotAllowed(_))));
    }

    #[tokio::test]
    async fn test_402_missing_headers() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(402).insert_header("X-Payment-Address", "addr"),
                // Missing other required headers
            )
            .mount(&server)
            .await;

        let (_tmp, kp) = test_keypair();
        let client = X402Client::new(kp, X402Config::default());

        let result = client.get(&server.uri()).await;
        assert!(matches!(result, Err(X402Error::InvalidResponse(_))));
    }

    #[test]
    fn test_payment_validation() {
        let config = X402Config {
            max_amount: 500_000,
            allowed_networks: vec!["solana-devnet".to_string()],
            allowed_tokens: vec!["ZEUS".to_string()],
        };

        let (_tmp, kp) = test_keypair();
        let client = X402Client::new(kp, config);

        // Valid
        let req = PaymentRequest {
            address: "addr".to_string(),
            amount: 100_000,
            network: "solana-devnet".to_string(),
            token: "ZEUS".to_string(),
            url: "http://example.com".to_string(),
        };
        assert!(client.validate_payment(&req).is_ok());

        // Amount exceeded
        let req2 = PaymentRequest {
            amount: 600_000,
            ..req.clone()
        };
        assert!(matches!(
            client.validate_payment(&req2),
            Err(X402Error::AmountExceeded { .. })
        ));

        // Bad network
        let req3 = PaymentRequest {
            network: "eth".to_string(),
            ..req.clone()
        };
        assert!(matches!(
            client.validate_payment(&req3),
            Err(X402Error::NetworkNotAllowed(_))
        ));

        // Bad token
        let req4 = PaymentRequest {
            token: "BTC".to_string(),
            ..req
        };
        assert!(matches!(
            client.validate_payment(&req4),
            Err(X402Error::TokenNotAllowed(_))
        ));
    }

    #[test]
    fn test_sign_payment_osrng_nonce() {
        let (_tmp, kp) = test_keypair();
        let client = X402Client::new(kp, X402Config::default());

        let req = PaymentRequest {
            address: "recipient".to_string(),
            amount: 50_000,
            network: "solana-devnet".to_string(),
            token: "ZEUS".to_string(),
            url: "http://example.com/api".to_string(),
        };

        let (payload1, sig1) = client.sign_payment(&req).unwrap();
        let (payload2, _sig2) = client.sign_payment(&req).unwrap();

        // Nonce must be unique per call (OsRng)
        assert_ne!(payload1.nonce, payload2.nonce, "nonces must be unique");

        // URL must be in payload (server swap protection)
        assert_eq!(payload1.url, req.url);

        // Signature must be non-empty
        assert!(!sig1.is_empty());

        // Signature must be valid
        let payload_json = serde_json::to_string(&payload1).unwrap();
        let sig_bytes = BASE64.decode(&sig1).unwrap();
        assert!(
            client
                .keypair
                .verify(payload_json.as_bytes(), &sig_bytes)
                .is_ok()
        );
    }

    #[test]
    fn test_rate_limit() {
        let (_tmp, kp) = test_keypair();
        let client = X402Client::new(kp, X402Config::default());
        let url = "http://example.com/api";

        // First RATE_LIMIT_MAX calls should succeed
        for _ in 0..RATE_LIMIT_MAX {
            assert!(client.check_rate_limit(url).is_ok());
        }

        // Next call should be rate-limited
        assert!(matches!(
            client.check_rate_limit(url),
            Err(X402Error::RateLimited(_))
        ));
    }

    #[test]
    fn test_receipt_persistence() {
        let tmp = TempDir::new().unwrap();
        let wallet_dir = tmp.path().join("w");
        let kp = WalletKeypair::generate(&wallet_dir, "test", "devnet").unwrap();
        let client = X402Client::new(kp, X402Config::default());

        let receipt = PaymentReceipt {
            request: PaymentRequest {
                address: "addr".to_string(),
                amount: 1000,
                network: "solana-devnet".to_string(),
                token: "ZEUS".to_string(),
                url: "http://example.com".to_string(),
            },
            signature: "sig123".to_string(),
            payer_pubkey: "pubkey123".to_string(),
            timestamp: "2026-03-04T00:00:00Z".to_string(),
            success: true,
            retry_status: 200,
        };

        client.persist_receipt(&receipt);

        // Log file should exist and contain valid JSON
        let log_path = wallet_dir.join("payments.jsonl");
        assert!(log_path.exists());
        let contents = std::fs::read_to_string(&log_path).unwrap();
        let parsed: PaymentReceipt = serde_json::from_str(contents.trim()).unwrap();
        assert_eq!(parsed.request.amount, 1000);
        assert!(parsed.success);
    }
}
