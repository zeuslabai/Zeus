//! TOTP 2FA handlers for blog admin (S23 Track H).
//!
//! Provides Google Authenticator-compatible TOTP setup, verification,
//! recovery codes, and JWT session tokens for blog write operations.
//!
//! Single-admin model: user ID is always "blog_admin".

use axum::{
    Json,
    extract::State,
    http::StatusCode,
};
use chrono::{Duration, Utc};
use data_encoding::BASE32;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use totp_rs::{Secret, TOTP};
use tracing::warn;

use crate::SharedState;
use super::totp_store::TotpStore;

/// The single admin user ID.
const ADMIN_USER: &str = "blog_admin";

/// JWT expiry in seconds (24 hours).
const JWT_EXPIRY_SECS: i64 = 86400;

// ============================================================================
// Request / response types
// ============================================================================

#[derive(Debug, Serialize)]
pub struct SetupResponse {
    pub secret_base32: String,
    pub otpauth_uri: String,
    pub recovery_codes: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct EnableRequest {
    pub code: String,
}

#[derive(Debug, Serialize)]
pub struct EnableResponse {
    pub enabled: bool,
}

#[derive(Debug, Deserialize)]
pub struct VerifyRequest {
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub recovery_code: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct VerifyResponse {
    pub token: String,
    pub expires_in: i64,
}

#[derive(Debug, Serialize)]
pub struct StatusResponse {
    pub enabled: bool,
    pub recovery_codes_remaining: usize,
}

#[derive(Debug, Serialize)]
pub struct DisableResponse {
    pub disabled: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TotpClaims {
    pub sub: String,
    pub iat: i64,
    pub exp: i64,
    pub purpose: String,
}

// ============================================================================
// Helpers
// ============================================================================

/// Get the JWT secret from env or generate a default (not recommended for production).
fn jwt_secret() -> Vec<u8> {
    std::env::var("ZEUS_TOTP_JWT_SECRET")
        .unwrap_or_else(|_| {
            warn!("ZEUS_TOTP_JWT_SECRET not set — using fallback key (set this in ~/.zeus/.env for production)");
            "zeus-totp-default-secret-change-me".to_string()
        })
        .into_bytes()
}

/// Hash a string with SHA-256 and return hex.
fn sha256_hex(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    hex::encode(hasher.finalize())
}

/// Generate 8 random 8-character alphanumeric recovery codes.
fn generate_recovery_codes() -> Vec<String> {
    let mut rng = rand::thread_rng();
    let charset = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789"; // no 0/O/1/I to avoid confusion
    (0..8)
        .map(|_| {
            (0..8)
                .map(|_| {
                    let idx = rng.gen_range(0..charset.len());
                    charset[idx] as char
                })
                .collect()
        })
        .collect()
}

/// Build a TOTP instance from a base32 secret.
fn build_totp(secret_base32: &str) -> Result<TOTP, String> {
    let secret_bytes = BASE32
        .decode(secret_base32.as_bytes())
        .map_err(|e| format!("Invalid base32 secret: {e}"))?;
    TOTP::new(
        totp_rs::Algorithm::SHA1,
        6,
        1,
        30,
        secret_bytes,
        Some("Zeus".to_string()),
        ADMIN_USER.to_string(),
    )
    .map_err(|e| format!("Failed to create TOTP: {e}"))
}

/// Generate a signed JWT for a verified 2FA session.
fn generate_jwt() -> Result<(String, i64), StatusCode> {
    let now = Utc::now();
    let exp = now + Duration::seconds(JWT_EXPIRY_SECS);
    let claims = TotpClaims {
        sub: ADMIN_USER.to_string(),
        iat: now.timestamp(),
        exp: exp.timestamp(),
        purpose: "totp_2fa".to_string(),
    };
    let secret = jwt_secret();
    let token = encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(&secret),
    )
    .map_err(|e| {
        warn!("Failed to encode JWT: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok((token, JWT_EXPIRY_SECS))
}

/// Validate a JWT and return the claims if valid.
pub fn validate_jwt(token: &str) -> Option<TotpClaims> {
    let secret = jwt_secret();
    let mut validation = Validation::new(Algorithm::HS256);
    validation.set_required_spec_claims(&["sub", "exp", "iat"]);
    decode::<TotpClaims>(token, &DecodingKey::from_secret(&secret), &validation)
        .ok()
        .map(|data| data.claims)
        .filter(|c| c.purpose == "totp_2fa")
}

/// Get the TotpStore from shared state.
async fn get_store(state: &SharedState) -> TotpStore {
    state.read().await.totp_store.clone()
}

// ============================================================================
// Handlers
// ============================================================================

/// `POST /v1/auth/totp/setup` — Generate TOTP secret + recovery codes.
///
/// Requires existing auth (bearer/API key). Returns secret, otpauth URI,
/// and recovery codes. TOTP is NOT enabled until `/enable` is called.
pub async fn totp_setup(
    State(state): State<SharedState>,
) -> Result<Json<SetupResponse>, StatusCode> {
    let store = get_store(&state).await;

    // Generate a 160-bit (20 byte) secret
    let secret = Secret::generate_secret();
    let secret_base32 = secret.to_encoded().to_string();

    // Build TOTP to get otpauth URI
    let totp = build_totp(&secret_base32).map_err(|e| {
        warn!("TOTP setup failed: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let otpauth_uri = totp.get_url();

    // Store secret (not yet enabled)
    if !store.create_user(ADMIN_USER, &secret_base32).await {
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    // Generate and store recovery codes
    let codes = generate_recovery_codes();
    let code_hashes: Vec<String> = codes.iter().map(|c| sha256_hex(c)).collect();
    if !store.store_recovery_codes(ADMIN_USER, &code_hashes).await {
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    Ok(Json(SetupResponse {
        secret_base32,
        otpauth_uri,
        recovery_codes: codes,
    }))
}

/// `POST /v1/auth/totp/enable` — Verify a TOTP code and enable 2FA.
///
/// Must provide a valid 6-digit code from the authenticator app.
/// This confirms the user has successfully set up their authenticator.
pub async fn totp_enable(
    State(state): State<SharedState>,
    Json(body): Json<EnableRequest>,
) -> Result<Json<EnableResponse>, StatusCode> {
    let store = get_store(&state).await;

    let user = store.get_user(ADMIN_USER).await.ok_or_else(|| {
        warn!("TOTP enable: no setup found — call /setup first");
        StatusCode::BAD_REQUEST
    })?;

    if user.enabled {
        return Ok(Json(EnableResponse { enabled: true }));
    }

    // Validate the code
    let totp = build_totp(&user.secret_base32).map_err(|e| {
        warn!("TOTP build failed: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    if !totp.check_current(&body.code).map_err(|e| {
        warn!("TOTP check failed: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })? {
        return Err(StatusCode::UNAUTHORIZED);
    }

    // Enable 2FA
    if !store.enable_user(ADMIN_USER).await {
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    Ok(Json(EnableResponse { enabled: true }))
}

/// `POST /v1/auth/totp/verify` — Verify TOTP code or recovery code, return JWT.
///
/// Accepts either `{ "code": "123456" }` or `{ "recovery_code": "ABCD1234" }`.
/// Returns a 24-hour JWT to be used as `X-Zeus-2FA-Token` header.
pub async fn totp_verify(
    State(state): State<SharedState>,
    Json(body): Json<VerifyRequest>,
) -> Result<Json<VerifyResponse>, StatusCode> {
    let store = get_store(&state).await;

    let user = store.get_user(ADMIN_USER).await.ok_or_else(|| {
        warn!("TOTP verify: no user found");
        StatusCode::BAD_REQUEST
    })?;

    if !user.enabled {
        return Err(StatusCode::BAD_REQUEST);
    }

    let verified = if let Some(code) = &body.code {
        // TOTP code verification
        let totp = build_totp(&user.secret_base32).map_err(|e| {
            warn!("TOTP build failed: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
        totp.check_current(code).map_err(|e| {
            warn!("TOTP check failed: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
    } else if let Some(recovery) = &body.recovery_code {
        // Recovery code verification
        let hash = sha256_hex(recovery);
        store.use_recovery_code(ADMIN_USER, &hash).await
    } else {
        return Err(StatusCode::BAD_REQUEST);
    };

    if !verified {
        return Err(StatusCode::UNAUTHORIZED);
    }

    // Generate JWT
    let (token, expires_in) = generate_jwt()?;

    // Store session hash for server-side validation
    let token_hash = sha256_hex(&token);
    let expires_at = (Utc::now() + Duration::seconds(expires_in)).to_rfc3339();
    store
        .create_session(&token_hash, ADMIN_USER, &expires_at)
        .await;

    Ok(Json(VerifyResponse { token, expires_in }))
}

/// `GET /v1/auth/totp/status` — Check 2FA status.
pub async fn totp_status(
    State(state): State<SharedState>,
) -> Result<Json<StatusResponse>, StatusCode> {
    let store = get_store(&state).await;

    let (enabled, remaining) = match store.get_user(ADMIN_USER).await {
        Some(user) => {
            let remaining = store.remaining_recovery_codes(ADMIN_USER).await;
            (user.enabled, remaining)
        }
        None => (false, 0),
    };

    Ok(Json(StatusResponse {
        enabled,
        recovery_codes_remaining: remaining,
    }))
}

/// `DELETE /v1/auth/totp` — Disable 2FA.
///
/// Requires existing auth + valid 2FA token (X-Zeus-2FA-Token header).
/// The 2FA middleware enforces this since DELETE is a mutation.
pub async fn totp_disable(
    State(state): State<SharedState>,
) -> Result<Json<DisableResponse>, StatusCode> {
    let store = get_store(&state).await;

    if !store.disable_user(ADMIN_USER).await {
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    Ok(Json(DisableResponse { disabled: true }))
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_totp_code_validation() {
        // Generate a secret and verify that codes validate
        let secret = Secret::generate_secret();
        let secret_b32 = secret.to_encoded().to_string();
        let totp = build_totp(&secret_b32).expect("should build TOTP");

        // Generate current code and verify
        let code = totp.generate_current().expect("should generate code");
        assert_eq!(code.len(), 6);
        assert!(totp.check_current(&code).expect("check should not error"));

        // Wrong code should fail
        assert!(!totp.check_current("000000").unwrap_or(true));
    }

    #[test]
    fn test_jwt_generation() {
        // Temporarily set a known secret
        let _guard = TempEnv::set("ZEUS_TOTP_JWT_SECRET", "test-secret-key-12345");

        let (token, expires_in) = generate_jwt().expect("JWT generation should succeed");
        assert!(!token.is_empty());
        assert_eq!(expires_in, 86400);

        // Validate the token
        let claims = validate_jwt(&token).expect("JWT should validate");
        assert_eq!(claims.sub, "blog_admin");
        assert_eq!(claims.purpose, "totp_2fa");
        assert!(claims.exp > claims.iat);
    }

    #[test]
    fn test_recovery_code_format() {
        let codes = generate_recovery_codes();
        assert_eq!(codes.len(), 8);
        for code in &codes {
            assert_eq!(code.len(), 8);
            // Only uppercase letters (no O/I) and digits (no 0/1)
            assert!(code.chars().all(|c| c.is_ascii_alphanumeric()));
        }
        // All codes should be unique
        let mut unique = codes.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), 8);
    }

    #[test]
    fn test_sha256_hex() {
        let hash = sha256_hex("hello");
        assert_eq!(hash.len(), 64); // SHA-256 = 32 bytes = 64 hex chars
        // Known SHA-256 of "hello"
        assert_eq!(
            hash,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    /// Helper to temporarily set an env var for tests.
    struct TempEnv {
        key: String,
        original: Option<String>,
    }

    impl TempEnv {
        fn set(key: &str, value: &str) -> Self {
            let original = std::env::var(key).ok();
            // SAFETY: Tests are single-threaded in this module.
            unsafe { std::env::set_var(key, value); }
            Self {
                key: key.to_string(),
                original,
            }
        }
    }

    impl Drop for TempEnv {
        fn drop(&mut self) {
            // SAFETY: Tests are single-threaded in this module.
            unsafe {
                if let Some(ref val) = self.original {
                    std::env::set_var(&self.key, val);
                } else {
                    std::env::remove_var(&self.key);
                }
            }
        }
    }
}
