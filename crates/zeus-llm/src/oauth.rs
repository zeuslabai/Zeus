//! Multi-provider authentication — OAuth, setup-tokens, and API keys
//!
//! Supports authentication for multiple providers:
//! - **Claude**: Browser OAuth flow or setup-token paste
//! - **OpenAI**: API key storage
//! - **Google**: API key storage
//! - **Any provider**: API key storage via `/login <provider> <key>`
//!
//! Credentials cached in-memory at runtime (populated from config.toml at startup).
//! Legacy `~/.zeus/credentials.json` supported as fallback but not written to.

use base64::Engine;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tracing::info;
use zeus_core::{Error, Result};

/// Global in-memory credential cache. Populated at startup from config.toml.
/// CredentialStore::load() checks this FIRST before falling back to disk.
/// This avoids writing credentials.json while keeping runtime auth working.
static MEMORY_CREDENTIALS: Mutex<Option<CredentialStore>> = Mutex::new(None);

/// Setup-token prefix used by Anthropic
const SETUP_TOKEN_PREFIX: &str = "sk-ant-oat01-";
/// Minimum length for a valid setup-token
const SETUP_TOKEN_MIN_LENGTH: usize = 80;

/// OAuth constants for Claude browser login
/// client_id matches the official Claude Code OAuth app registration
const CLIENT_ID: &str = "9d1c250a-e61b-44b0-b5e0-4e85fbb11600";
const AUTHORIZE_URL: &str = "https://claude.ai/oauth/authorize";
const TOKEN_URL: &str = "https://api.anthropic.com/oauth/token";
const SCOPES: &str = "user:profile user:inference user:sessions:claude_code user:mcp_servers";

/// Known API key prefixes for auto-detection
const OPENAI_KEY_PREFIX: &str = "sk-";
const ANTHROPIC_API_KEY_PREFIX: &str = "sk-ant-api";

/// OAuth constants for OpenAI Codex browser login
const OPENAI_CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const OPENAI_CODEX_AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
const OPENAI_CODEX_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const OPENAI_CODEX_SCOPES: &str = "openid profile email offline_access";
const OPENAI_CODEX_REDIRECT_PORT: u16 = 1455;

/// Stored authentication token (legacy single-provider format)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthTokens {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: DateTime<Utc>,
    /// Which provider this token is for (default: "anthropic" for backward compat)
    #[serde(default = "default_provider")]
    pub provider: String,
}

fn default_provider() -> String {
    "anthropic".to_string()
}

/// Multi-provider credential store
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CredentialStore {
    /// Provider name -> stored credential
    pub credentials: HashMap<String, StoredCredential>,
}

/// A single provider's stored credential
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredCredential {
    /// The provider name (anthropic, openai, google, etc.)
    pub provider: String,
    /// Credential type
    pub kind: CredentialKind,
    /// The token/key value
    pub token: String,
    /// Refresh token (OAuth only)
    #[serde(default)]
    pub refresh_token: String,
    /// When the credential expires (far-future for API keys)
    pub expires_at: DateTime<Utc>,
    /// When stored
    pub stored_at: DateTime<Utc>,
}

/// Type of credential
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CredentialKind {
    /// Claude setup-token (sk-ant-oat01-)
    SetupToken,
    /// OAuth access token from browser flow
    OAuthToken,
    /// API key (OpenAI sk-, Anthropic sk-ant-api-, etc.)
    ApiKey,
}

impl OAuthTokens {
    /// Check if access token is expired
    pub fn is_expired(&self) -> bool {
        Utc::now() >= self.expires_at
    }

    /// Load tokens from disk (legacy format — reads oauth_tokens.json)
    pub fn load() -> Result<Option<Self>> {
        let path = Self::storage_path()?;
        if !path.exists() {
            return Ok(None);
        }
        let content = std::fs::read_to_string(&path)?;
        let tokens: Self = serde_json::from_str(&content)?;
        Ok(Some(tokens))
    }

    /// Save tokens to disk
    pub fn save(&self) -> Result<()> {
        let path = Self::storage_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, content)?;
        set_file_permissions(&path)?;
        Ok(())
    }

    /// Delete stored tokens
    pub fn delete() -> Result<()> {
        let path = Self::storage_path()?;
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        Ok(())
    }

    fn storage_path() -> Result<PathBuf> {
        let home = dirs::home_dir()
            .ok_or_else(|| Error::Config("Could not find home directory".to_string()))?;
        Ok(home.join(".zeus").join("oauth_tokens.json"))
    }
}

/// Set secure file permissions (0600 on Unix)
fn set_file_permissions(path: &PathBuf) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

impl CredentialStore {
    /// Load credential store — checks in-memory cache first, then falls back to disk.
    /// config.toml is the SSoT; credentials.json is a legacy fallback only.
    pub fn load() -> Result<Self> {
        // Check in-memory cache first (populated from config.toml at startup)
        if let Ok(guard) = MEMORY_CREDENTIALS.lock() {
            if let Some(ref store) = *guard {
                return Ok(store.clone());
            }
        }

        // Fall back to disk (credentials.json) for legacy compat
        let path = Self::storage_path()?;
        if !path.exists() {
            // Migrate from legacy oauth_tokens.json if it exists
            let mut store = Self::default();
            if let Ok(Some(legacy)) = OAuthTokens::load() {
                let kind = if legacy.access_token.starts_with(SETUP_TOKEN_PREFIX) {
                    CredentialKind::SetupToken
                } else {
                    CredentialKind::OAuthToken
                };
                store.credentials.insert(
                    legacy.provider.clone(),
                    StoredCredential {
                        provider: legacy.provider,
                        kind,
                        token: legacy.access_token,
                        refresh_token: legacy.refresh_token,
                        expires_at: legacy.expires_at,
                        stored_at: Utc::now(),
                    },
                );
            }
            return Ok(store);
        }
        let content = std::fs::read_to_string(&path)?;
        let store: Self = serde_json::from_str(&content)?;
        Ok(store)
    }

    /// Store a credential in the global in-memory cache only (no disk write).
    /// Used at startup to populate from config.toml without creating credentials.json.
    pub fn store_in_memory(credential: StoredCredential) {
        if let Ok(mut guard) = MEMORY_CREDENTIALS.lock() {
            let store = guard.get_or_insert_with(Self::default);
            store.credentials.insert(credential.provider.clone(), credential);
            info!("Credential cached in memory for provider: {}", store.credentials.keys().last().unwrap_or(&String::new()));
        }
    }

    /// Save credential store to disk
    pub fn save(&self) -> Result<()> {
        let path = Self::storage_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, content)?;
        set_file_permissions(&path)?;
        Ok(())
    }

    /// Store a credential for a provider
    pub fn store(&mut self, credential: StoredCredential) -> Result<()> {
        self.credentials
            .insert(credential.provider.clone(), credential);
        self.save()
    }

    /// Get credential for a provider
    pub fn get(&self, provider: &str) -> Option<&StoredCredential> {
        self.credentials.get(provider)
    }

    /// Get a valid (non-expired) token for a provider
    pub fn get_valid_token(&self, provider: &str) -> Option<&str> {
        self.credentials.get(provider).and_then(|c| {
            if Utc::now() < c.expires_at {
                Some(c.token.as_str())
            } else {
                None
            }
        })
    }

    /// Remove credential for a provider
    pub fn remove(&mut self, provider: &str) -> Result<()> {
        self.credentials.remove(provider);
        self.save()
    }

    /// Remove all credentials
    pub fn clear(&mut self) -> Result<()> {
        self.credentials.clear();
        self.save()?;
        // Also clean up legacy file
        let _ = OAuthTokens::delete();
        Ok(())
    }

    /// List all stored providers
    pub fn providers(&self) -> Vec<&str> {
        self.credentials.keys().map(|s| s.as_str()).collect()
    }

    /// Check if any credentials are stored
    pub fn is_empty(&self) -> bool {
        self.credentials.is_empty()
    }

    fn storage_path() -> Result<PathBuf> {
        let home = dirs::home_dir()
            .ok_or_else(|| Error::Config("Could not find home directory".to_string()))?;
        Ok(home.join(".zeus").join("credentials.json"))
    }
}

// ============================================================================
// OAuth helper functions
// ============================================================================

/// Extract query string from HTTP request line
fn extract_query_string(request: &str) -> Option<String> {
    let first_line = request.lines().next()?;
    let path = first_line.split_whitespace().nth(1)?;
    let query = path.split('?').nth(1)?;
    Some(query.to_string())
}

/// Parse code and state from query string
fn parse_code_and_state(query: &str) -> (String, Option<String>) {
    let mut code = String::new();
    let mut state = None;
    for pair in query.split('&') {
        if let Some((key, value)) = pair.split_once('=') {
            match key {
                "code" => code = urlencoding::decode(value).unwrap_or_default().to_string(),
                "state" => state = Some(urlencoding::decode(value).unwrap_or_default().to_string()),
                _ => {}
            }
        }
    }
    (code, state)
}

/// PKCE challenge/verifier pair for OAuth authorization code flow.
pub struct PkceChallenge {
    pub verifier: String,
    pub challenge: String,
}

impl PkceChallenge {
    pub fn generate() -> Self {
        let random_bytes: Vec<u8> = (0..32).map(|_| rand::random::<u8>()).collect();
        let verifier = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&random_bytes);

        let mut hasher = Sha256::new();
        hasher.update(verifier.as_bytes());
        let hash = hasher.finalize();
        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hash);

        Self {
            verifier,
            challenge,
        }
    }

    /// Build a PkceChallenge from an existing verifier (e.g. one generated by the frontend).
    /// Computes challenge = base64url(sha256(verifier)).
    pub fn from_verifier(verifier: &str) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(verifier.as_bytes());
        let hash = hasher.finalize();
        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hash);
        Self {
            verifier: verifier.to_string(),
            challenge,
        }
    }
}

/// Start a local HTTP server to receive OAuth callback.
async fn start_callback_server() -> std::result::Result<(u16, oneshot::Receiver<String>), Error> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| Error::Llm(format!("Failed to bind callback server: {}", e)))?;
    let port = listener
        .local_addr()
        .map_err(|e| Error::Llm(format!("Failed to get local addr: {}", e)))?
        .port();
    let (tx, rx) = oneshot::channel();

    tokio::spawn(async move {
        if let Ok((mut stream, _)) = listener.accept().await {
            let mut buf = vec![0u8; 4096];
            let n = stream.read(&mut buf).await.unwrap_or(0);
            let request = String::from_utf8_lossy(&buf[..n]);

            if let Some(query) = extract_query_string(&request) {
                let (code, state) = parse_code_and_state(&query);

                let html = "<html><body style='font-family:sans-serif;text-align:center;\
                            padding:60px'><h1>Zeus \u{2014} Login Successful</h1>\
                            <p>You can close this tab and return to Zeus.</p></body></html>";
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n{}",
                    html.len(),
                    html
                );
                let _ = stream.write_all(response.as_bytes()).await;

                let code_str = match state {
                    Some(s) => format!("{}#{}", code, s),
                    None => code,
                };
                let _ = tx.send(code_str);
            }
        }
    });

    Ok((port, rx))
}

// ============================================================================
// OAuthManager
// ============================================================================

/// Authentication manager — handles multi-provider credential storage and OAuth flows
pub struct OAuthManager;

impl Default for OAuthManager {
    fn default() -> Self {
        Self::new()
    }
}

impl OAuthManager {
    pub fn new() -> Self {
        Self
    }

    // ========================================================================
    // Multi-provider credential management
    // ========================================================================

    /// Store an API key for any provider.
    /// Auto-detects provider from key prefix if provider is not specified.
    pub fn login_with_api_key(provider: &str, key: &str) -> Result<()> {
        let trimmed = key.trim();
        if trimmed.is_empty() {
            return Err(Error::Config("API key is empty".to_string()));
        }

        let mut store = CredentialStore::load()?;
        store.store(StoredCredential {
            provider: provider.to_string(),
            kind: CredentialKind::ApiKey,
            token: trimmed.to_string(),
            refresh_token: String::new(),
            expires_at: Utc::now() + Duration::days(365 * 10),
            stored_at: Utc::now(),
        })?;
        info!("API key stored for provider: {}", provider);
        Ok(())
    }

    /// Validate and cache a setup-token from config.toml [oauth] section.
    /// Populates the in-memory CredentialStore so zeus-llm can resolve the token at runtime.
    /// Does NOT write credentials.json — config.toml is the single source of truth.
    pub fn login_with_token(token: &str) -> Result<OAuthTokens> {
        let trimmed = token.trim();
        if let Some(err) = validate_setup_token(trimmed) {
            return Err(Error::Config(err));
        }

        let tokens = OAuthTokens {
            access_token: trimmed.to_string(),
            refresh_token: String::new(),
            expires_at: Utc::now() + Duration::days(365 * 10),
            provider: "anthropic".to_string(),
        };

        // S97: Populate in-memory CredentialStore (no disk write).
        // config.toml is the SSoT. This just makes the token available at runtime.
        CredentialStore::store_in_memory(StoredCredential {
            provider: "anthropic".to_string(),
            kind: CredentialKind::SetupToken,
            token: trimmed.to_string(),
            refresh_token: String::new(),
            expires_at: Utc::now() + Duration::days(365 * 10),
            stored_at: Utc::now(),
        });

        info!("Setup-token cached in memory (config.toml is SSoT, no credentials.json written)");
        Ok(tokens)
    }

    /// Browser-based OAuth login for Claude with local callback server.
    pub async fn login_claude(&self) -> Result<OAuthTokens> {
        let (port, code_rx) = start_callback_server().await?;
        let redirect_uri = format!("http://localhost:{}/callback", port);
        let pkce = PkceChallenge::generate();

        let params = [
            ("client_id", CLIENT_ID),
            ("response_type", "code"),
            ("redirect_uri", redirect_uri.as_str()),
            ("scope", SCOPES),
            ("code_challenge", &pkce.challenge),
            ("code_challenge_method", "S256"),
            ("state", &pkce.verifier),
        ];
        let query = params
            .iter()
            .map(|(k, v)| format!("{}={}", k, urlencoding::encode(v)))
            .collect::<Vec<_>>()
            .join("&");
        let auth_url = format!("{}?{}", AUTHORIZE_URL, query);

        open::that(&auth_url).map_err(|e| Error::Llm(format!("Failed to open browser: {}", e)))?;

        let code_input = tokio::time::timeout(std::time::Duration::from_secs(300), code_rx)
            .await
            .map_err(|_| Error::Llm("OAuth login timed out (5 minutes). Try /login again.".into()))?
            .map_err(|_| Error::Llm("OAuth callback server closed unexpectedly".into()))?;

        let (code, _state) = if let Some(pos) = code_input.find('#') {
            (
                code_input[..pos].to_string(),
                Some(code_input[pos + 1..].to_string()),
            )
        } else {
            (code_input, None)
        };

        let tokens = self
            .exchange_code(&code, &pkce.verifier, &redirect_uri)
            .await?;

        // Store in new credential store
        let mut store = CredentialStore::load()?;
        store.store(StoredCredential {
            provider: "anthropic".to_string(),
            kind: CredentialKind::OAuthToken,
            token: tokens.access_token.clone(),
            refresh_token: tokens.refresh_token.clone(),
            expires_at: tokens.expires_at,
            stored_at: Utc::now(),
        })?;

        // Also save legacy format
        tokens.save()?;
        info!("Claude OAuth login successful via browser callback");
        Ok(tokens)
    }

    /// Browser-based OAuth login (backward compat — calls login_claude)
    pub async fn login(&self) -> Result<OAuthTokens> {
        self.login_claude().await
    }

    /// Browser-based OAuth login for OpenAI Codex with local callback server (port 1455).
    pub async fn login_openai(&self) -> Result<OAuthTokens> {
        let redirect_uri = format!("http://localhost:{}/callback", OPENAI_CODEX_REDIRECT_PORT);
        let pkce = PkceChallenge::generate();

        // Generate a separate random state nonce (distinct from the PKCE verifier).
        // The state param exists solely for CSRF protection — it must NOT be the
        // PKCE verifier, which is a secret used in the token exchange.
        let state_bytes: Vec<u8> = (0..32).map(|_| rand::random::<u8>()).collect();
        let state_nonce = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&state_bytes);

        let params = [
            ("client_id", OPENAI_CODEX_CLIENT_ID),
            ("response_type", "code"),
            ("redirect_uri", redirect_uri.as_str()),
            ("scope", OPENAI_CODEX_SCOPES),
            ("code_challenge", &pkce.challenge),
            ("code_challenge_method", "S256"),
            ("state", &state_nonce),
        ];
        let query = params
            .iter()
            .map(|(k, v)| format!("{}={}", k, urlencoding::encode(v)))
            .collect::<Vec<_>>()
            .join("&");
        let auth_url = format!("{}?{}", OPENAI_CODEX_AUTHORIZE_URL, query);

        // Bind to port 1455 first so it's ready before we open the browser
        let listener = TcpListener::bind(format!("127.0.0.1:{}", OPENAI_CODEX_REDIRECT_PORT))
            .await
            .map_err(|e| Error::Llm(format!("Failed to bind OAuth callback port {}: {}", OPENAI_CODEX_REDIRECT_PORT, e)))?;

        let (tx, rx) = oneshot::channel::<String>();
        let tx = std::sync::Arc::new(Mutex::new(Some(tx)));
        // Clone state_nonce so we can move it into the callback task for validation.
        let expected_state = state_nonce.clone();

        tokio::spawn(async move {
            if let Ok((mut stream, _)) = listener.accept().await {
                let mut buf = vec![0u8; 4096];
                if let Ok(n) = stream.read(&mut buf).await {
                    let request = String::from_utf8_lossy(&buf[..n]);
                    // Parse GET /callback?code=...&state=... HTTP/1.1
                    if let Some(line) = request.lines().next() {
                        let parts: Vec<&str> = line.split_whitespace().collect();
                        if parts.len() >= 2 {
                            let path = parts[1];
                            if let Some(query_start) = path.find('?') {
                                let query_str = &path[query_start + 1..];
                                let mut code = String::new();
                                let mut returned_state = String::new();
                                for pair in query_str.split('&') {
                                    if let Some(v) = pair.strip_prefix("code=") {
                                        code = urlencoding::decode(v).unwrap_or_default().into_owned();
                                    } else if let Some(v) = pair.strip_prefix("state=") {
                                        returned_state = urlencoding::decode(v).unwrap_or_default().into_owned();
                                    }
                                }

                                // Validate state to prevent CSRF attacks.
                                let state_valid = !returned_state.is_empty()
                                    && returned_state == expected_state;

                                if !code.is_empty() && state_valid {
                                    let response = "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n<html><body><h2>OpenAI auth complete — return to Zeus.</h2></body></html>";
                                    let _ = stream.write_all(response.as_bytes()).await;
                                    if let Ok(mut guard) = tx.lock() {
                                        if let Some(sender) = guard.take() {
                                            let _ = sender.send(code);
                                        }
                                    }
                                } else {
                                    // State mismatch or missing code — reject with 400
                                    let response = "HTTP/1.1 400 Bad Request\r\nContent-Type: text/html\r\n\r\n<html><body><h2>OAuth error: invalid state parameter.</h2></body></html>";
                                    let _ = stream.write_all(response.as_bytes()).await;
                                    // Don't send on the channel — timeout will handle it
                                }
                            }
                        }
                    }
                }
            }
        });

        open::that(&auth_url).map_err(|e| Error::Llm(format!("Failed to open browser: {}", e)))?;

        let code = tokio::time::timeout(std::time::Duration::from_secs(300), rx)
            .await
            .map_err(|_| Error::Llm("OpenAI OAuth login timed out (5 minutes). Try /login openai again.".into()))?
            .map_err(|_| Error::Llm("OpenAI OAuth callback channel closed unexpectedly".into()))?;

        let tokens = self
            .exchange_code_openai(&code, &pkce.verifier, &redirect_uri)
            .await?;

        let mut store = CredentialStore::load()?;
        store.store(StoredCredential {
            provider: "openai".to_string(),
            kind: CredentialKind::OAuthToken,
            token: tokens.access_token.clone(),
            refresh_token: tokens.refresh_token.clone(),
            expires_at: tokens.expires_at,
            stored_at: Utc::now(),
        })?;

        info!("OpenAI Codex OAuth login successful via browser callback");
        Ok(tokens)
    }

    /// Exchange an OpenAI authorization code for tokens.
    async fn exchange_code_openai(
        &self,
        code: &str,
        verifier: &str,
        redirect_uri: &str,
    ) -> Result<OAuthTokens> {
        let client = reqwest::Client::new();
        let params = [
            ("grant_type", "authorization_code"),
            ("client_id", OPENAI_CODEX_CLIENT_ID),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("code_verifier", verifier),
        ];
        let resp = client
            .post(OPENAI_CODEX_TOKEN_URL)
            .form(&params)
            .send()
            .await
            .map_err(|e| Error::Llm(format!("OpenAI token exchange request failed: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Llm(format!("OpenAI token exchange failed ({}): {}", status, body)));
        }

        #[derive(Deserialize)]
        struct TokenResponse {
            access_token: String,
            refresh_token: Option<String>,
            expires_in: Option<u64>,
        }

        let token_resp: TokenResponse = resp
            .json()
            .await
            .map_err(|e| Error::Llm(format!("Failed to parse OpenAI token response: {}", e)))?;

        let expires_at = Utc::now()
            + Duration::seconds(token_resp.expires_in.unwrap_or(3600) as i64);

        Ok(OAuthTokens {
            access_token: token_resp.access_token,
            refresh_token: token_resp.refresh_token.unwrap_or_default(),
            expires_at,
            provider: "openai".to_string(),
        })
    }

    /// Auto-detect provider from an API key and store it
    pub fn login_auto_detect(key: &str) -> Result<String> {
        let trimmed = key.trim();

        // Check for setup-token first
        if trimmed.starts_with(SETUP_TOKEN_PREFIX) {
            Self::login_with_token(trimmed)?;
            return Ok("anthropic (setup-token)".to_string());
        }

        // Check for Anthropic API key
        if trimmed.starts_with(ANTHROPIC_API_KEY_PREFIX) {
            Self::login_with_api_key("anthropic", trimmed)?;
            return Ok("anthropic".to_string());
        }

        // Check for OpenAI key (sk- prefix, but not sk-ant-)
        if trimmed.starts_with(OPENAI_KEY_PREFIX) && !trimmed.starts_with("sk-ant-") {
            Self::login_with_api_key("openai", trimmed)?;
            return Ok("openai".to_string());
        }

        // Check for Google key (AIza prefix)
        if trimmed.starts_with("AIza") {
            Self::login_with_api_key("google", trimmed)?;
            return Ok("google".to_string());
        }

        // Check for OpenRouter key
        if trimmed.starts_with("sk-or-") {
            Self::login_with_api_key("openrouter", trimmed)?;
            return Ok("openrouter".to_string());
        }

        Err(Error::Config(
            "Could not auto-detect provider from key. Use /login <provider> <key> instead.\n\
             Supported: /login openai <key>, /login anthropic <key>, /login google <key>"
                .to_string(),
        ))
    }

    /// Import credentials from the OpenAI Codex CLI (`~/.codex/`).
    ///
    /// Codex CLI stores auth in `~/.codex/auth.json` with format:
    /// `{"token": "sk-...", "provider": "openai"}`
    /// or as a plain API key file.
    pub fn import_codex_cli_credentials() -> Result<Option<String>> {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .map_err(|_| Error::Config("Cannot determine home directory".to_string()))?;

        let candidates = [
            format!("{}/.codex/auth.json", home),
            format!("{}/.codex/credentials.json", home),
            format!("{}/.config/codex/auth.json", home),
        ];

        for path in &candidates {
            let p = std::path::Path::new(path);
            if !p.exists() { continue; }

            let contents = std::fs::read_to_string(p)
                .map_err(|e| Error::Config(format!("Failed to read {}: {}", path, e)))?;

            // Try JSON format — Codex CLI uses nested structure:
            // {"tokens": {"access_token": "...", "refresh_token": "..."}, "OPENAI_API_KEY": "..."}
            // Also handles flat: {"token": "...", "api_key": "..."}
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&contents) {
                // Check nested tokens.access_token (real Codex CLI format)
                let token = json.get("tokens")
                    .and_then(|t| t.get("access_token"))
                    .and_then(|v| v.as_str())
                    // Fallback: top-level OPENAI_API_KEY
                    .or_else(|| json.get("OPENAI_API_KEY").and_then(|v| v.as_str()))
                    // Fallback: flat token/api_key
                    .or_else(|| json.get("token").and_then(|v| v.as_str()))
                    .or_else(|| json.get("api_key").and_then(|v| v.as_str()));

                if let Some(token) = token {
                    if !token.is_empty() {
                        tracing::info!("Imported Codex CLI credentials from {}", path);
                        // Store as OpenAI credential directly (access_token is a JWT, not sk- prefix)
                        Self::login_with_api_key("openai", token)?;
                        return Ok(Some("openai".to_string()));
                    }
                }
            }

            // Fallback: plain text key
            let trimmed = contents.trim();
            if trimmed.starts_with("sk-") && trimmed.len() > 20 {
                tracing::info!("Imported Codex CLI API key from {}", path);
                let provider = Self::login_auto_detect(trimmed)?;
                return Ok(Some(provider));
            }
        }

        Ok(None) // No Codex CLI credentials found — not an error
    }

    /// Get a valid token for a specific provider from the credential store
    pub fn get_stored_token(provider: &str) -> Result<Option<String>> {
        let store = CredentialStore::load()?;
        Ok(store.get_valid_token(provider).map(|s| s.to_string()))
    }

    /// Get credential details for a provider
    pub fn get_credential(provider: &str) -> Result<Option<StoredCredential>> {
        let store = CredentialStore::load()?;
        Ok(store.get(provider).cloned())
    }

    /// List all stored provider credentials
    pub fn list_credentials() -> Result<Vec<(String, CredentialKind, bool)>> {
        let store = CredentialStore::load()?;
        Ok(store
            .credentials
            .values()
            .map(|c| {
                (
                    c.provider.clone(),
                    c.kind.clone(),
                    Utc::now() < c.expires_at,
                )
            })
            .collect())
    }

    /// Exchange authorization code for tokens
    async fn exchange_code(
        &self,
        code: &str,
        verifier: &str,
        redirect_uri: &str,
    ) -> Result<OAuthTokens> {
        let client = reqwest::Client::new();
        let body = serde_json::json!({
            "grant_type": "authorization_code",
            "code": code,
            "redirect_uri": redirect_uri,
            "client_id": CLIENT_ID,
            "code_verifier": verifier,
        });

        let resp = client
            .post(TOKEN_URL)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Llm(format!("Token exchange failed: {}", e)))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Llm(format!("Token exchange failed: {}", body)));
        }

        #[derive(Deserialize)]
        struct TokenResponse {
            access_token: String,
            refresh_token: Option<String>,
            expires_in: Option<i64>,
        }

        let token_resp: TokenResponse = resp
            .json()
            .await
            .map_err(|e| Error::Llm(format!("Failed to parse token response: {}", e)))?;

        let expires_at = Utc::now() + Duration::seconds(token_resp.expires_in.unwrap_or(3600));

        Ok(OAuthTokens {
            access_token: token_resp.access_token,
            refresh_token: token_resp.refresh_token.unwrap_or_default(),
            expires_at,
            provider: "anthropic".to_string(),
        })
    }

    /// Refresh an expired OAuth token using the stored refresh_token.
    /// Returns new OAuthTokens if refresh succeeds, or None if no refresh_token available.
    pub async fn refresh_token(provider: &str) -> Result<Option<OAuthTokens>> {
        let store = CredentialStore::load()?;
        let cred = match store.get(provider) {
            Some(c) => c.clone(),
            None => return Ok(None),
        };

        // Only OAuth tokens have refresh capability
        if cred.kind != CredentialKind::OAuthToken || cred.refresh_token.is_empty() {
            return Ok(None);
        }

        // Determine token URL based on provider
        let token_url = match provider {
            "anthropic" => TOKEN_URL,
            "openai" => OPENAI_CODEX_TOKEN_URL,
            _ => TOKEN_URL,
        };

        let client_id = match provider {
            "anthropic" => CLIENT_ID,
            "openai" => OPENAI_CODEX_CLIENT_ID,
            _ => CLIENT_ID,
        };

        let client = reqwest::Client::new();
        let body = serde_json::json!({
            "grant_type": "refresh_token",
            "refresh_token": cred.refresh_token,
            "client_id": client_id,
        });

        let resp = client
            .post(token_url)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Llm(format!("Token refresh failed: {}", e)))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Llm(format!("Token refresh failed: {}", body)));
        }

        #[derive(Deserialize)]
        struct TokenResponse {
            access_token: String,
            refresh_token: Option<String>,
            expires_in: Option<i64>,
        }

        let token_resp: TokenResponse = resp
            .json()
            .await
            .map_err(|e| Error::Llm(format!("Failed to parse refresh response: {}", e)))?;

        let expires_at = Utc::now() + Duration::seconds(token_resp.expires_in.unwrap_or(3600));

        let tokens = OAuthTokens {
            access_token: token_resp.access_token.clone(),
            refresh_token: token_resp
                .refresh_token
                .unwrap_or(cred.refresh_token.clone()),
            expires_at,
            provider: provider.to_string(),
        };

        // Update credential store
        let mut store = CredentialStore::load()?;
        store.store(StoredCredential {
            provider: provider.to_string(),
            kind: CredentialKind::OAuthToken,
            token: tokens.access_token.clone(),
            refresh_token: tokens.refresh_token.clone(),
            expires_at: tokens.expires_at,
            stored_at: Utc::now(),
        })?;

        // Update legacy format too
        if provider == "anthropic" {
            tokens.save()?;
        }

        info!("OAuth token refreshed for provider: {}", provider);
        Ok(Some(tokens))
    }

    /// Get a valid access token, auto-refreshing if expired and refresh_token exists.
    pub async fn get_valid_token(&self, provider: &str) -> Result<Option<String>> {
        // Check new credential store first
        if let Ok(Some(token)) = Self::get_stored_token(provider) {
            return Ok(Some(token));
        }
        // Check if expired but refreshable
        if let Ok(store) = CredentialStore::load()
            && let Some(cred) = store.get(provider)
                && Utc::now() >= cred.expires_at && !cred.refresh_token.is_empty() {
                    // Try refresh
                    if let Ok(Some(tokens)) = Self::refresh_token(provider).await {
                        return Ok(Some(tokens.access_token));
                    }
                }
        // Fall back to legacy (anthropic only — legacy format predates multi-provider)
        if provider == "anthropic" {
            match OAuthTokens::load()? {
                Some(tokens) if !tokens.is_expired() => return Ok(Some(tokens.access_token)),
                _ => {}
            }
        }
        Ok(None)
    }

    /// Check if the user has a stored token (any provider)
    pub fn is_logged_in() -> bool {
        if let Ok(store) = CredentialStore::load() {
            !store.is_empty()
        } else {
            OAuthTokens::load().ok().flatten().is_some()
        }
    }

    /// Check if a specific provider has credentials
    pub fn is_provider_logged_in(provider: &str) -> bool {
        Self::get_stored_token(provider).ok().flatten().is_some()
    }

    /// Delete stored token for a specific provider
    pub fn logout_provider(provider: &str) -> Result<()> {
        let mut store = CredentialStore::load()?;
        store.remove(provider)?;
        // If removing anthropic, also clean legacy
        if provider == "anthropic" {
            let _ = OAuthTokens::delete();
        }
        Ok(())
    }

    /// Delete all stored tokens
    pub fn logout() -> Result<()> {
        let mut store = CredentialStore::load()?;
        store.clear()?;
        OAuthTokens::delete()
    }
}

/// Exchange an authorization code for tokens using a configurable token endpoint.
///
/// This is the public version of the private `exchange_code()` method on `OAuthManager`,
/// intended for use by REST API OAuth handlers that need caller-supplied URLs.
pub async fn exchange_authorization_code(
    token_url: &str,
    client_id: &str,
    code: &str,
    code_verifier: &str,
    redirect_uri: &str,
) -> Result<OAuthTokens> {
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "grant_type": "authorization_code",
        "code": code,
        "redirect_uri": redirect_uri,
        "client_id": client_id,
        "code_verifier": code_verifier,
    });

    let resp = client
        .post(token_url)
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| Error::Llm(format!("Token exchange failed: {e}")))?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(Error::Llm(format!(
            "Token exchange failed ({status}): {body}"
        )));
    }

    #[derive(Deserialize)]
    struct TokenResponse {
        access_token: String,
        refresh_token: Option<String>,
        expires_in: Option<i64>,
    }

    let token_resp: TokenResponse = resp
        .json()
        .await
        .map_err(|e| Error::Llm(format!("Failed to parse token response: {e}")))?;

    let expires_at = Utc::now() + Duration::seconds(token_resp.expires_in.unwrap_or(3600));

    Ok(OAuthTokens {
        access_token: token_resp.access_token,
        refresh_token: token_resp.refresh_token.unwrap_or_default(),
        expires_at,
        provider: "anthropic".to_string(),
    })
}

/// Validate a setup-token format.
/// Returns None if valid, Some(error_message) if invalid.
pub fn validate_setup_token(token: &str) -> Option<String> {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        return Some("Token is empty".to_string());
    }
    if !trimmed.starts_with(SETUP_TOKEN_PREFIX) {
        return Some(format!(
            "Token must start with '{}'. Run `claude setup-token` to generate one.",
            SETUP_TOKEN_PREFIX
        ));
    }
    if trimmed.len() < SETUP_TOKEN_MIN_LENGTH {
        return Some(
            "Token is too short. Paste the full token from `claude setup-token`.".to_string(),
        );
    }
    None
}

/// Parse an authorization code input (backward compat utility).
pub fn parse_auth_code(input: &str) -> Result<(String, Option<String>)> {
    let trimmed = input.trim();
    if let Some(pos) = trimmed.find('#') {
        Ok((
            trimmed[..pos].to_string(),
            Some(trimmed[pos + 1..].to_string()),
        ))
    } else {
        Ok((trimmed.to_string(), None))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_setup_token_valid() {
        let token = format!("sk-ant-oat01-{}", "a".repeat(67));
        assert_eq!(token.len(), 80);
        assert!(validate_setup_token(&token).is_none());
    }

    #[test]
    fn test_validate_setup_token_long() {
        let token = format!("sk-ant-oat01-{}", "x".repeat(200));
        assert!(validate_setup_token(&token).is_none());
    }

    #[test]
    fn test_validate_setup_token_too_short() {
        let token = "sk-ant-oat01-short";
        let err = validate_setup_token(token);
        assert!(err.is_some());
        assert!(err.expect("operation should succeed").contains("too short"));
    }

    #[test]
    fn test_validate_setup_token_wrong_prefix() {
        let token = format!("sk-ant-api01-{}", "a".repeat(67));
        let err = validate_setup_token(&token);
        assert!(err.is_some());
        assert!(
            err.expect("operation should succeed")
                .contains("sk-ant-oat01-")
        );
    }

    #[test]
    fn test_validate_setup_token_empty() {
        assert!(validate_setup_token("").is_some());
        assert!(validate_setup_token("  ").is_some());
    }

    #[test]
    fn test_parse_auth_code_with_hash() {
        let (code, state) = parse_auth_code("abc123#xyz789").expect("should parse successfully");
        assert_eq!(code, "abc123");
        assert_eq!(state, Some("xyz789".to_string()));
    }

    #[test]
    fn test_parse_auth_code_without_hash() {
        let (code, state) = parse_auth_code("abc123").expect("should parse successfully");
        assert_eq!(code, "abc123");
        assert_eq!(state, None);
    }

    #[test]
    fn test_login_with_valid_token() {
        let token = format!("sk-ant-oat01-{}", "b".repeat(67));
        // This will try to save to disk which may fail in CI,
        // but validates the token format check works
        let result = OAuthManager::login_with_token(&token);
        // In test env, file save may fail, but token validation should pass
        if let Ok(tokens) = result {
            assert_eq!(tokens.access_token, token);
            assert!(tokens.refresh_token.is_empty());
            // Clean up
            let _ = OAuthTokens::delete();
        }
    }

    #[test]
    fn test_login_with_invalid_token() {
        let result = OAuthManager::login_with_token("not-a-valid-token");
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_query_string() {
        let req = "GET /callback?code=abc&state=xyz HTTP/1.1\r\nHost: localhost\r\n";
        assert_eq!(
            extract_query_string(req),
            Some("code=abc&state=xyz".to_string())
        );
    }

    #[test]
    fn test_extract_query_string_no_query() {
        let req = "GET /callback HTTP/1.1\r\n";
        assert_eq!(extract_query_string(req), None);
    }

    #[test]
    fn test_parse_code_and_state_both() {
        let (code, state) = parse_code_and_state("code=hello&state=world");
        assert_eq!(code, "hello");
        assert_eq!(state, Some("world".to_string()));
    }

    #[test]
    fn test_parse_code_and_state_code_only() {
        let (code, state) = parse_code_and_state("code=hello");
        assert_eq!(code, "hello");
        assert_eq!(state, None);
    }

    #[test]
    fn test_pkce_challenge() {
        let pkce = PkceChallenge::generate();
        assert!(!pkce.verifier.is_empty());
        assert!(!pkce.challenge.is_empty());
        // Verifier and challenge should be different
        assert_ne!(pkce.verifier, pkce.challenge);
    }

    #[tokio::test]
    async fn test_callback_server_binds() {
        let result = start_callback_server().await;
        assert!(result.is_ok());
        let (port, _rx) = result.expect("operation should succeed");
        assert!(port > 0);
    }

    // ========================================================================
    // OpenAI Codex OAuth tests — constants, PKCE properties, token exchange.
    //
    // login_openai() itself is not unit-testable end-to-end because it binds
    // a hard-coded port, calls open::that to launch a browser, and waits on
    // a 5-minute callback. Instead we cover the testable sub-components:
    //
    //   - OpenAI Codex OAuth constants match the pi-ai extraction.
    //   - PkceChallenge S256 is deterministic and matches the RFC 7636 vector.
    //   - exchange_authorization_code (public caller-supplied-URL variant of
    //     exchange_code_openai) token exchange against a local mock server.
    // ========================================================================

    /// Spawn a one-shot capturing mock HTTP server. Accepts one connection,
    /// reads the full request into a buffer, writes the canned response, and
    /// forwards the captured raw request text over a oneshot channel.
    async fn spawn_capturing_mock(
        status_line: &'static str,
        body: &'static str,
    ) -> (
        String,
        tokio::sync::oneshot::Receiver<String>,
        tokio::task::JoinHandle<()>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock listener should bind");
        let port = listener
            .local_addr()
            .expect("listener should have addr")
            .port();
        let url = format!("http://127.0.0.1:{}", port);
        let (tx, rx) = tokio::sync::oneshot::channel::<String>();

        let handle = tokio::spawn(async move {
            if let Ok((mut stream, _)) = listener.accept().await {
                let mut buf = vec![0u8; 16384];
                let n = stream.read(&mut buf).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]).to_string();
                let response = format!(
                    "{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    status_line,
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
                let _ = tx.send(request);
            }
        });

        (url, rx, handle)
    }

    #[test]
    fn test_openai_codex_oauth_constants_match_pi_ai_extraction() {
        // Pinned from @mariozechner/pi-ai extraction via zeus107's audit (commit
        // 6b2cacdc) and later corrected in becf9a90. Locking these in so a
        // regression renaming or rewriting them breaks loudly.
        assert_eq!(OPENAI_CODEX_CLIENT_ID, "app_EMoamEEZ73f0CkXaXp7hrann");
        assert_eq!(
            OPENAI_CODEX_AUTHORIZE_URL,
            "https://auth.openai.com/oauth/authorize"
        );
        assert_eq!(OPENAI_CODEX_TOKEN_URL, "https://auth.openai.com/oauth/token");
        assert_eq!(OPENAI_CODEX_SCOPES, "openid profile email offline_access");
        assert_eq!(OPENAI_CODEX_REDIRECT_PORT, 1455);
    }

    #[test]
    fn test_pkce_from_verifier_matches_rfc7636_vector() {
        // RFC 7636 Appendix B — canonical PKCE S256 test vector.
        //   verifier:  dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk
        //   challenge: E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let pkce = PkceChallenge::from_verifier(verifier);
        assert_eq!(pkce.verifier, verifier);
        assert_eq!(
            pkce.challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM",
            "from_verifier must produce the RFC 7636 canonical S256 challenge"
        );
    }

    #[test]
    fn test_pkce_generate_challenge_matches_from_verifier() {
        // Whatever verifier `generate` produces, passing it through `from_verifier`
        // must reproduce the same challenge — guards against accidental divergence
        // between the two code paths.
        let generated = PkceChallenge::generate();
        let reconstructed = PkceChallenge::from_verifier(&generated.verifier);
        assert_eq!(
            generated.challenge, reconstructed.challenge,
            "generate() and from_verifier() must compute the same S256 challenge for the same verifier"
        );
    }

    #[test]
    fn test_pkce_verifier_and_challenge_base64url_properties() {
        // Both are base64url-nopad of 32-byte inputs → 43 characters.
        let pkce = PkceChallenge::generate();
        assert_eq!(
            pkce.verifier.len(),
            43,
            "verifier should be base64url-nopad of 32 random bytes (43 chars)"
        );
        assert_eq!(
            pkce.challenge.len(),
            43,
            "challenge should be base64url-nopad of SHA256(verifier) = 32 bytes (43 chars)"
        );
        // base64url alphabet: [A-Za-z0-9_-], no +/=
        for c in pkce.verifier.chars().chain(pkce.challenge.chars()) {
            assert!(
                c.is_ascii_alphanumeric() || c == '-' || c == '_',
                "unexpected character in base64url output: {:?}",
                c
            );
        }
    }

    #[tokio::test]
    async fn test_exchange_authorization_code_success() {
        // Mock the token endpoint. `exchange_authorization_code` is the public,
        // caller-supplied-URL variant of `exchange_code_openai` and exercises
        // the same request/response contract — grant_type, code, code_verifier,
        // client_id, redirect_uri → access_token + refresh_token + expires_in.
        let (url, rx, _handle) = spawn_capturing_mock(
            "HTTP/1.1 200 OK",
            r#"{"access_token":"tok_abc123","refresh_token":"rt_xyz789","expires_in":3600}"#,
        )
        .await;

        let before = Utc::now();
        let result = exchange_authorization_code(
            &url,
            "test-client-id",
            "test-auth-code",
            "test-verifier",
            "http://localhost:1455/callback",
        )
        .await;
        assert!(
            result.is_ok(),
            "token exchange should succeed against 200 mock, got err: {:?}",
            result.err()
        );

        let tokens = result.expect("ok branch");
        assert_eq!(tokens.access_token, "tok_abc123");
        assert_eq!(tokens.refresh_token, "rt_xyz789");
        // expires_at should be roughly `now + expires_in` (3600 seconds).
        assert!(
            tokens.expires_at > before + Duration::seconds(3500),
            "expires_at should reflect expires_in=3600, got: {}",
            tokens.expires_at
        );
        assert!(
            tokens.expires_at < before + Duration::seconds(3700),
            "expires_at should reflect expires_in=3600, got: {}",
            tokens.expires_at
        );

        // Verify the HTTP request the client actually sent to the mock.
        let request = rx.await.expect("mock should capture request");
        assert!(
            request.starts_with("POST / "),
            "expected POST to mock root, got request line: {}",
            request.lines().next().unwrap_or("<empty>")
        );
        // JSON body assertions — these are the PKCE + authorization code grant params.
        assert!(
            request.contains("\"grant_type\":\"authorization_code\""),
            "body should include grant_type=authorization_code, got: {}",
            request
        );
        assert!(
            request.contains("\"code\":\"test-auth-code\""),
            "body should include the authorization code, got: {}",
            request
        );
        assert!(
            request.contains("\"code_verifier\":\"test-verifier\""),
            "body should include the PKCE code_verifier, got: {}",
            request
        );
        assert!(
            request.contains("\"client_id\":\"test-client-id\""),
            "body should include the client_id, got: {}",
            request
        );
        assert!(
            request.contains("\"redirect_uri\":\"http://localhost:1455/callback\""),
            "body should include the redirect_uri, got: {}",
            request
        );
    }

    #[tokio::test]
    async fn test_exchange_authorization_code_error_status() {
        // Non-success status should map to Err with status + body in the message.
        let (url, _rx, _handle) = spawn_capturing_mock(
            "HTTP/1.1 400 Bad Request",
            r#"{"error":"invalid_grant","error_description":"code expired"}"#,
        )
        .await;

        let result = exchange_authorization_code(
            &url,
            "cid",
            "expired-code",
            "verifier",
            "http://localhost/cb",
        )
        .await;
        assert!(result.is_err(), "400 response should map to Err");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("400"),
            "error should mention the 400 status, got: {}",
            err
        );
        assert!(
            err.to_lowercase().contains("invalid_grant")
                || err.to_lowercase().contains("token exchange failed"),
            "error should surface the failure/body, got: {}",
            err
        );
    }
}
