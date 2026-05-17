//! OAuth PKCE authentication flow.
//!
//! Implements the full Authorization Code + PKCE flow:
//! 1. Generate PKCE verifier/challenge + random state
//! 2. Open browser to authorization URL
//! 3. Capture callback at local HTTP server
//! 4. Exchange authorization code for tokens
//! 5. Return access + refresh tokens

use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::callback::start_callback_server;
use crate::pkce::{PkceChallenge, generate_state};

/// Cached Gemini CLI credentials to avoid repeated filesystem scans (Gap 13).
static GEMINI_CLI_CRED_CACHE: std::sync::OnceLock<Option<(String, String)>> = std::sync::OnceLock::new();

/// OAuth provider configuration.
#[derive(Debug, Clone)]
pub struct OAuthProvider {
    /// Provider name (e.g. "openai", "anthropic")
    pub name: String,
    /// Authorization endpoint URL
    pub authorize_url: String,
    /// Token exchange endpoint URL
    pub token_url: String,
    /// Client ID
    pub client_id: String,
    /// Scopes to request
    pub scopes: Vec<String>,
    /// Local callback port (default: 1455)
    pub callback_port: u16,
    /// Redirect URI host — "localhost" for OpenAI (Codex CLI compat), "127.0.0.1" for others
    pub redirect_host: String,
    /// Redirect URI path — "/auth/callback" for OpenAI, "/oauth2callback" for Google
    pub redirect_path: String,
    /// Client secret (required by Google OAuth, not used by OpenAI/Anthropic PKCE flows)
    pub client_secret: Option<String>,
    /// Extra query params appended to the authorization URL (e.g. access_type, prompt)
    pub extra_auth_params: Vec<(String, String)>,
}

impl OAuthProvider {
    /// OpenAI Codex OAuth provider.
    pub fn openai(client_id: &str) -> Self {
        Self {
            name: "openai".into(),
            authorize_url: "https://auth.openai.com/oauth/authorize".into(),
            token_url: "https://auth.openai.com/oauth/token".into(),
            client_id: client_id.into(),
            scopes: vec![
                "openid".into(),
                "profile".into(),
                "email".into(),
                "offline_access".into(),
                "model.request".into(),
            ],
            callback_port: 1455,
            redirect_host: "localhost".into(), // Must match Codex CLI exactly
            redirect_path: "/auth/callback".into(),
            client_secret: None, // OpenAI uses PKCE, no client_secret
            extra_auth_params: vec![],
        }
    }

    /// Anthropic OAuth provider.
    pub fn anthropic(client_id: &str) -> Self {
        Self {
            name: "anthropic".into(),
            authorize_url: "https://console.anthropic.com/oauth/authorize".into(),
            token_url: "https://console.anthropic.com/oauth/token".into(),
            client_id: client_id.into(),
            scopes: vec!["openid".into()],
            callback_port: 1455,
            redirect_host: "127.0.0.1".into(),
            redirect_path: "/auth/callback".into(),
            client_secret: None,
            extra_auth_params: vec![],
        }
    }

    /// Google / Gemini OAuth provider.
    ///
    /// Uses Google's OAuth 2.0 endpoints with the `generative-language` scope
    /// required for the Gemini API.

    /// Google Gemini CLI OAuth provider — routes through cloudcode-pa.googleapis.com.
    ///
    /// Uses the `cloud-platform` scope (required for Code Assist API access) instead of
    /// `generative-language`. This unlocks Gemini 3.x models only available via Code Assist.
    /// After auth, call `discover_google_project()` to obtain the `projectId`.
    /// Google Gemini CLI OAuth — uses real Gemini CLI credentials.
    /// client_id and client_secret are public (baked into gemini-cli source).
    pub fn google_gemini_cli() -> Self {
        Self {
            name: "google-gemini-cli".into(),
            authorize_url: "https://accounts.google.com/o/oauth2/v2/auth".into(),
            token_url: "https://oauth2.googleapis.com/token".into(),
            // Real Gemini CLI client_id (from google-gemini/gemini-cli source)
            client_id: "681255809395-oo8ft2oprdrnp9e3aqf6av3hmdib135j.apps.googleusercontent.com".into(),
            scopes: vec![
                "https://www.googleapis.com/auth/cloud-platform".into(),
                "https://www.googleapis.com/auth/userinfo.email".into(),
                "https://www.googleapis.com/auth/userinfo.profile".into(),
            ],
            callback_port: 1455,
            redirect_host: "localhost".into(),
            redirect_path: "/oauth2callback".into(),
            // Public client_secret (Google installed-app flow requires it)
            client_secret: Some("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".into()),
            extra_auth_params: vec![
                ("access_type".into(), "offline".into()),
            ],
        }
    }
    pub fn google(client_id: &str) -> Self {
        Self {
            name: "google".into(),
            authorize_url: "https://accounts.google.com/o/oauth2/v2/auth".into(),
            token_url: "https://oauth2.googleapis.com/token".into(),
            client_id: client_id.into(),
            scopes: vec![
                "openid".into(),
                "profile".into(),
                "email".into(),
                "https://www.googleapis.com/auth/generative-language".into(),
            ],
            callback_port: 1455,
            redirect_host: "localhost".into(),
            redirect_path: "/oauth2callback".into(),
            client_secret: None,
            extra_auth_params: vec![
                ("access_type".into(), "offline".into()),
            ],
        }
    }
}

/// Credentials imported from the Gemini CLI (`~/.gemini/` or `~/.config/gemini/`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeminiCliCredentials {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub client_secret: Option<String>,
    #[serde(default)]
    pub expiry: Option<String>,
}

/// Import existing Gemini CLI credentials from the local filesystem.
///
/// Scans `~/.gemini/oauth_creds.json` and `~/.config/gemini/oauth_creds.json`
/// (the locations used by the `gemini` CLI tool).
///
/// Returns the parsed credentials if found, or an error if no credentials exist.
pub fn import_gemini_cli_credentials() -> anyhow::Result<GeminiCliCredentials> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map_err(|_| anyhow::anyhow!("Cannot determine home directory"))?;

    let candidates = [
        format!("{}/.gemini/oauth_creds.json", home),
        format!("{}/.config/gemini/oauth_creds.json", home),
        format!("{}/.gemini/credentials.json", home),
        format!("{}/.config/gemini/credentials.json", home),
    ];

    for path in &candidates {
        let p = std::path::Path::new(path);
        if p.exists() {
            info!("Found Gemini CLI credentials at {}", path);
            let contents = std::fs::read_to_string(p)
                .map_err(|e| anyhow::anyhow!("Failed to read {}: {}", path, e))?;
            let creds: GeminiCliCredentials = serde_json::from_str(&contents)
                .map_err(|e| anyhow::anyhow!("Failed to parse {}: {}", path, e))?;
            if creds.access_token.is_empty() {
                warn!("Gemini CLI credentials at {} have empty access_token, skipping", path);
                continue;
            }
            return Ok(creds);
        }
    }

    Err(anyhow::anyhow!(
        "No Gemini CLI credentials found. Looked in: {}",
        candidates.join(", ")
    ))
}

/// Refresh Gemini CLI credentials using the stored refresh token + client credentials.
///
/// The Gemini CLI stores `client_id`, `client_secret`, and `refresh_token` in its
/// credential file. This uses those to get a fresh access token from Google's token endpoint.
pub async fn refresh_gemini_cli_token(creds: &GeminiCliCredentials) -> anyhow::Result<TokenResponse> {
    let refresh_token = creds.refresh_token.as_deref()
        .ok_or_else(|| anyhow::anyhow!("No refresh_token in Gemini CLI credentials"))?;
    let client_id = creds.client_id.as_deref()
        .ok_or_else(|| anyhow::anyhow!("No client_id in Gemini CLI credentials"))?;
    let client_secret = creds.client_secret.as_deref()
        .ok_or_else(|| anyhow::anyhow!("No client_secret in Gemini CLI credentials"))?;

    let client = reqwest::Client::new();
    let resp = client
        .post("https://oauth2.googleapis.com/token")
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", client_id),
            ("client_secret", client_secret),
        ])
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Gemini CLI token refresh failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("Gemini CLI token refresh failed ({}): {}", status, body));
    }

    resp.json().await
        .map_err(|e| anyhow::anyhow!("Failed to parse Gemini CLI refresh response: {}", e))
}

/// Token response from the OAuth provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub token_type: String,
    #[serde(default)]
    pub expires_in: Option<u64>,
    #[serde(default)]
    pub scope: Option<String>,
}

/// Run the full OAuth PKCE flow.
///
/// 1. Start local callback server
/// 2. Generate PKCE challenge + state
/// 3. Build authorization URL
/// 4. Open browser (or print URL for headless)
/// 5. Wait for callback
/// 6. Exchange code for tokens
pub async fn run_oauth_flow(provider: &OAuthProvider) -> anyhow::Result<TokenResponse> {
    // 1. Start callback server
    let callback_rx = start_callback_server(provider.callback_port).await?;
    let redirect_uri = format!("http://{}:{}{}", provider.redirect_host, provider.callback_port, provider.redirect_path);

    // 2. Generate PKCE + state
    let pkce = PkceChallenge::generate();
    let state = generate_state();

    // 3. Build authorization URL
    let scope = provider.scopes.join(" ");
    let mut auth_url = format!(
        "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}&code_challenge={}&code_challenge_method=S256",
        provider.authorize_url,
        urlencoding::encode(&provider.client_id),
        urlencoding::encode(&redirect_uri),
        urlencoding::encode(&scope),
        urlencoding::encode(&state),
        urlencoding::encode(&pkce.challenge),
    );
    for (k, v) in &provider.extra_auth_params {
        auth_url.push('&');
        auth_url.push_str(&urlencoding::encode(k));
        auth_url.push('=');
        auth_url.push_str(&urlencoding::encode(v));
    }

    // 4. Open browser — if it fails, return structured error so TUI can display the URL
    info!("Opening browser for {} OAuth login...", provider.name);
    if let Err(e) = open_browser(&auth_url) {
        warn!("Could not open browser: {}", e);
        return Err(anyhow::anyhow!(
            "OAUTH_MANUAL_REQUIRED:{}\nBrowser could not be opened ({}). \
            Copy this URL into your browser manually.",
            auth_url, e
        ));
    }

    // 5. Wait for callback (120 second timeout)
    let callback = tokio::time::timeout(
        std::time::Duration::from_secs(120),
        callback_rx,
    )
    .await
    .map_err(|_| anyhow::anyhow!(
        "OAUTH_MANUAL_REQUIRED:{}\nOAuth callback timed out (120s). \
        If the browser didn't open, copy this URL manually.",
        auth_url
    ))?
    .map_err(|_| anyhow::anyhow!("OAuth callback channel closed"))?;

    // Validate state
    if callback.state != state {
        return Err(anyhow::anyhow!(
            "OAuth state mismatch — possible CSRF attack. Expected {}, got {}",
            &state[..state.len().min(8)], &callback.state[..callback.state.len().min(8)]
        ));
    }

    info!("OAuth callback received, exchanging code for tokens...");

    // 6. Exchange code for tokens
    let client = reqwest::Client::new();
    let mut form_params: Vec<(&str, &str)> = vec![
        ("grant_type", "authorization_code"),
        ("code", &callback.code),
        ("redirect_uri", &redirect_uri),
        ("client_id", &provider.client_id),
        ("code_verifier", &pkce.verifier),
    ];
    // Google installed-app OAuth requires client_secret in token exchange
    let secret_ref;
    if let Some(ref secret) = provider.client_secret {
        secret_ref = secret.clone();
        form_params.push(("client_secret", &secret_ref));
    }
    let resp = client
        .post(&provider.token_url)
        .form(&form_params)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Token exchange request failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("Token exchange failed ({}): {}", status, body));
    }

    let tokens: TokenResponse = resp.json().await
        .map_err(|e| anyhow::anyhow!("Failed to parse token response: {}", e))?;

    info!("OAuth flow complete — {} tokens received", provider.name);
    Ok(tokens)
}

/// Open a URL in the default browser.
fn open_browser(url: &str) -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(url).spawn()?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open").arg(url).spawn()?;
    }
    #[cfg(target_os = "freebsd")]
    {
        std::process::Command::new("xdg-open").arg(url).spawn()?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd").args(["/c", "start", url]).spawn()?;
    }
    Ok(())
}

/// Refresh an expired OAuth token.
pub async fn refresh_token(
    provider: &OAuthProvider,
    refresh_token: &str,
) -> anyhow::Result<TokenResponse> {
    let client = reqwest::Client::new();
    let resp = client
        .post(&provider.token_url)
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", &provider.client_id),
        ])
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Token refresh failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("Token refresh failed ({}): {}", status, body));
    }

    resp.json().await
        .map_err(|e| anyhow::anyhow!("Failed to parse refresh response: {}", e))
}

/// Extract OAuth client credentials from the installed Gemini CLI binary's bundled oauth2.js.
pub fn extract_gemini_cli_credentials() -> Option<(String, String)> {
    // 1. Check env var overrides first
    let env_id = std::env::var("GEMINI_CLI_OAUTH_CLIENT_ID")
        .or_else(|_| std::env::var("OPENCLAW_GEMINI_OAUTH_CLIENT_ID"))
        .ok();
    let env_secret = std::env::var("GEMINI_CLI_OAUTH_CLIENT_SECRET")
        .or_else(|_| std::env::var("OPENCLAW_GEMINI_OAUTH_CLIENT_SECRET"))
        .ok();
    if let Some(id) = env_id {
        return Some((id, env_secret.unwrap_or_default()));
    }
    // 2. Check cache (Gap 13 — avoid repeated filesystem scans)
    if let Some(cached) = GEMINI_CLI_CRED_CACHE.get() {
        return cached.clone();
    }

    // 3. Find `gemini` on PATH
    let path_var = std::env::var("PATH").unwrap_or_default();
    let sep = if cfg!(windows) { ';' } else { ':' };
    let exts: &[&str] = if cfg!(windows) { &[".cmd", ".bat", ".exe", ""] } else { &[""] };
    let mut gemini_path: Option<std::path::PathBuf> = None;
    'outer: for dir in path_var.split(sep) {
        for ext in exts {
            let candidate = std::path::Path::new(dir).join(format!("gemini{}", ext));
            if candidate.exists() {
                gemini_path = Some(candidate);
                break 'outer;
            }
        }
    }
    let gemini_path = gemini_path?;
    let resolved = std::fs::canonicalize(&gemini_path).unwrap_or_else(|_| gemini_path.clone());
    let bin_dir = gemini_path.parent()?;
    let resolved_dir = resolved.parent()?;
    let candidates: Vec<std::path::PathBuf> = [
        // Gap 12: OpenClaw's first candidate — dirname(dirname(resolvedPath))
        resolved_dir.parent().and_then(|p| p.parent()).map(|p| p.to_path_buf()),
        resolved_dir.parent().map(|p| p.to_path_buf()),
        Some(resolved_dir.join("node_modules/@google/gemini-cli")),
        Some(bin_dir.join("node_modules/@google/gemini-cli")),
        bin_dir.parent().map(|p| p.join("node_modules/@google/gemini-cli")),
        bin_dir.parent().map(|p| p.join("lib/node_modules/@google/gemini-cli")),
    ].into_iter().flatten().collect();
    let mut content: Option<String> = None;
    for dir in &candidates {
        for sub in &[
            "node_modules/@google/gemini-cli-core/dist/src/code_assist/oauth2.js",
            "node_modules/@google/gemini-cli-core/dist/code_assist/oauth2.js",
        ] {
            let p = dir.join(sub);
            if p.exists() {
                if let Ok(c) = std::fs::read_to_string(&p) { content = Some(c); break; }
            }
        }
        if content.is_some() { break; }
        if let Some(found) = find_file_recursive(dir, "oauth2.js", 10) {
            if let Ok(c) = std::fs::read_to_string(&found) { content = Some(c); break; }
        }
    }
    let content = content?;
    let id_re = regex::Regex::new(r"(\d+-[a-z0-9]+\.apps\.googleusercontent\.com)").ok()?;
    let secret_re = regex::Regex::new(r"(GOCSPX-[A-Za-z0-9_-]+)").ok()?;
    let client_id = id_re.captures(&content)?.get(1)?.as_str().to_string();
    let client_secret = secret_re.captures(&content)?.get(1)?.as_str().to_string();
    let result = Some((client_id, client_secret));
    let _ = GEMINI_CLI_CRED_CACHE.set(result.clone());
    result
}

fn find_file_recursive(dir: &std::path::Path, name: &str, depth: usize) -> Option<std::path::PathBuf> {
    if depth == 0 { return None; }
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.is_file() && path.file_name().and_then(|n| n.to_str()) == Some(name) {
            return Some(path);
        }
        if path.is_dir() && !path.file_name().and_then(|n| n.to_str()).unwrap_or("").starts_with('.') {
            if let Some(found) = find_file_recursive(&path, name, depth - 1) {
                return Some(found);
            }
        }
    }
    None
}

/// Discover (or provision) the Google Cloud project for Code Assist API access.
pub async fn discover_google_project(access_token: &str) -> anyhow::Result<String> {
    const ENDPOINTS: &[&str] = &[
        "https://cloudcode-pa.googleapis.com",
        "https://daily-cloudcode-pa.sandbox.googleapis.com",
        "https://autopush-cloudcode-pa.sandbox.googleapis.com",
    ];
    const TIER_FREE: &str = "free-tier";
    let env_project = std::env::var("GOOGLE_CLOUD_PROJECT")
        .or_else(|_| std::env::var("GOOGLE_CLOUD_PROJECT_ID"))
        .ok();
    let platform = if cfg!(target_os = "macos") { "MACOS" }
        else if cfg!(target_os = "windows") { "WINDOWS" }
        else { "PLATFORM_UNSPECIFIED" };
    let metadata = serde_json::json!({
        "ideType": "ANTIGRAVITY",
        "platform": platform,
        "pluginType": "GEMINI",
    });
    let mut load_body = serde_json::json!({ "metadata": &metadata });
    if let Some(ref p) = env_project {
        load_body["cloudaicompanionProject"] = serde_json::Value::String(p.clone());
        load_body["metadata"]["duetProject"] = serde_json::Value::String(p.clone());
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;
    let mut headers_map = reqwest::header::HeaderMap::new();
    headers_map.insert("Authorization", format!("Bearer {}", access_token).parse()?);
    headers_map.insert("Content-Type", "application/json".parse()?);
    headers_map.insert("User-Agent", "google-api-nodejs-client/9.15.1".parse()?);
    headers_map.insert("X-Goog-Api-Client", format!("gl-rust/{}", env!("CARGO_PKG_VERSION")).parse()?);
    headers_map.insert("Client-Metadata", serde_json::to_string(&metadata).unwrap_or_default().parse()?);
    let mut data: Option<serde_json::Value> = None;
    let mut active_endpoint = ENDPOINTS[0];
    let mut load_error: Option<anyhow::Error> = None;
    for endpoint in ENDPOINTS {
        match client.post(format!("{}/v1internal:loadCodeAssist", endpoint))
            .headers(headers_map.clone()).json(&load_body).send().await {
            Ok(r) if r.status().is_success() => {
                let body: serde_json::Value = r.json().await
                    .map_err(|e| anyhow::anyhow!("loadCodeAssist parse error: {}", e))?;
                data = Some(body);
                active_endpoint = endpoint;
                load_error = None;
                break;
            }
            Ok(r) => {
                let status = r.status();
                if let Ok(payload) = r.json::<serde_json::Value>().await {
                    if payload["error"]["details"].as_array()
                        .map(|d| d.iter().any(|i| i["reason"].as_str() == Some("SECURITY_POLICY_VIOLATED")))
                        .unwrap_or(false) {
                        data = Some(serde_json::json!({ "currentTier": { "id": "standard-tier" } }));
                        active_endpoint = endpoint;
                        load_error = None;
                        break;
                    }
                }
                load_error = Some(anyhow::anyhow!("loadCodeAssist failed: {}", status));
            }
            Err(e) => { load_error = Some(anyhow::anyhow!("loadCodeAssist request failed: {}", e)); }
        }
    }
    let data = match data {
        Some(d) => d,
        None => {
            if let Some(ref p) = env_project { return Ok(p.clone()); }
            return Err(load_error.unwrap_or_else(|| anyhow::anyhow!("loadCodeAssist failed on all endpoints")));
        }
    };
    if data["currentTier"].is_object() || !data["currentTier"].is_null() {
        let project = &data["cloudaicompanionProject"];
        if let Some(s) = project.as_str().filter(|s| !s.is_empty()) { return Ok(s.to_string()); }
        if let Some(id) = project.get("id").and_then(|v| v.as_str()) { return Ok(id.to_string()); }
        if let Some(ref p) = env_project { return Ok(p.clone()); }
        return Err(anyhow::anyhow!("This account requires GOOGLE_CLOUD_PROJECT to be set."));
    }
    // Free-tier: onboard the user
    let tier_id = data["allowedTiers"].as_array()
        .and_then(|tiers| {
            if tiers.is_empty() { return Some("legacy-tier".to_string()); }
            tiers.iter()
                .find(|t| t["isDefault"].as_bool().unwrap_or(false))
                .or_else(|| tiers.first())
                .and_then(|t| t["id"].as_str()).map(String::from)
        })
        .unwrap_or_else(|| TIER_FREE.to_string());
    if tier_id != TIER_FREE && env_project.is_none() {
        return Err(anyhow::anyhow!("This account requires GOOGLE_CLOUD_PROJECT to be set."));
    }
    let mut onboard_body = serde_json::json!({ "tierId": &tier_id, "metadata": &metadata });
    if let Some(ref p) = env_project {
        onboard_body["cloudaicompanionProject"] = serde_json::Value::String(p.clone());
        onboard_body["metadata"]["duetProject"] = serde_json::Value::String(p.clone());
    }
    let onboard_resp = client.post(format!("{}/v1internal:onboardUser", active_endpoint))
        .headers(headers_map.clone()).json(&onboard_body).send().await
        .map_err(|e| anyhow::anyhow!("onboardUser failed: {}", e))?;
    if !onboard_resp.status().is_success() {
        let s = onboard_resp.status();
        let b = onboard_resp.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("onboardUser failed ({}): {}", s, b));
    }
    let mut lro: serde_json::Value = onboard_resp.json().await
        .map_err(|e| anyhow::anyhow!("onboardUser parse error: {}", e))?;
    if !lro["done"].as_bool().unwrap_or(false) {
        if let Some(op_name) = lro["name"].as_str().map(String::from) {
            for _ in 0..24 {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                if let Ok(r) = client.get(format!("{}/v1internal/{}", active_endpoint, op_name))
                    .headers(headers_map.clone()).send().await {
                    if r.status().is_success() {
                        if let Ok(polled) = r.json::<serde_json::Value>().await {
                            if polled["done"].as_bool().unwrap_or(false) { lro = polled; break; }
                        }
                    }
                }
            }
        }
    }
    if let Some(id) = lro["response"]["cloudaicompanionProject"]["id"].as_str() {
        return Ok(id.to_string());
    }
    if let Some(ref p) = env_project { return Ok(p.clone()); }
    Err(anyhow::anyhow!("Could not discover or provision a Google Cloud project. Set GOOGLE_CLOUD_PROJECT."))
}

/// Fetch the authenticated user's email from Google's userinfo endpoint.
///
/// Called after a successful OAuth token exchange to associate an email
/// address with the credential (Gap 10b).
pub async fn fetch_google_user_email(access_token: &str) -> Option<String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .ok()?;
    let resp = client
        .get("https://www.googleapis.com/oauth2/v1/userinfo?alt=json")
        .bearer_auth(access_token)
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body: serde_json::Value = resp.json().await.ok()?;
    body.get("email")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

// ============================================================================
// Device Code OAuth Flow (Qwen, MiniMax)
// ============================================================================

/// Provider configuration for Device Code OAuth flow.
/// Unlike PKCE browser flows, device code flow:
/// 1. Requests a device_code + user_code from the provider
/// 2. Displays user_code and verification URL to the user
/// 3. Polls the token endpoint until user authorizes
#[derive(Debug, Clone)]
pub struct DeviceCodeProvider {
    /// Provider name (e.g. "qwen", "minimax")
    pub name: String,
    /// Device code request endpoint
    pub device_code_url: String,
    /// Token polling endpoint
    pub token_url: String,
    /// Client ID
    pub client_id: String,
    /// Scopes to request
    pub scopes: Vec<String>,
}

impl DeviceCodeProvider {
    // NOTE: Qwen and MiniMax are NOT routed through this generic engine.
    // They use provider-specific modules (zeus-llm::qwen_oauth, zeus-llm::minimax)
    // because MiniMax requires a non-standard grant_type
    // ("urn:ietf:params:oauth:grant-type:user_code" vs RFC 8628
    // "urn:ietf:params:oauth:grant-type:device_code") and both need
    // custom token caching + inference URL handling.
    // Add constructors here only for providers that use standard RFC 8628.
}

/// Response from the device code request.
#[derive(Debug, Deserialize)]
pub struct DeviceCodeResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: Option<String>,
    #[serde(alias = "verification_url")]
    pub verification_uri_alt: Option<String>,
    #[serde(default = "default_interval")]
    pub interval: u64,
    #[serde(default = "default_expires_in")]
    pub expires_in: u64,
}

fn default_interval() -> u64 { 5 }
fn default_expires_in() -> u64 { 600 }

impl DeviceCodeResponse {
    pub fn verification_url(&self) -> &str {
        self.verification_uri.as_deref()
            .or(self.verification_uri_alt.as_deref())
            .unwrap_or("")
    }
}

/// Run the Device Code OAuth flow.
///
/// 1. Request device_code + user_code
/// 2. Display user_code and verification URL
/// 3. Poll token endpoint until authorized (or timeout)
pub async fn run_device_code_flow(provider: &DeviceCodeProvider) -> anyhow::Result<TokenResponse> {
    let client = reqwest::Client::new();

    // 1. Request device code
    info!("Requesting device code from {} ...", provider.name);
    let scope = provider.scopes.join(" ");
    let resp = client
        .post(&provider.device_code_url)
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "client_id": provider.client_id,
            "scope": scope,
        }))
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Device code request failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!("Device code request failed ({}): {}", status, body));
    }

    let device: DeviceCodeResponse = resp.json().await
        .map_err(|e| anyhow::anyhow!("Failed to parse device code response: {}", e))?;

    let verify_url = device.verification_url();
    info!(
        "{} OAuth: visit {} and enter code: {}",
        provider.name, verify_url, device.user_code
    );

    // 2. Try to open browser
    if let Err(e) = open_browser(verify_url) {
        warn!("Could not open browser: {}", e);
    }

    // 3. Poll for token
    let mut poll_interval = std::time::Duration::from_secs(device.interval.max(3));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(device.expires_in);

    loop {
        if std::time::Instant::now() > deadline {
            return Err(anyhow::anyhow!(
                "OAUTH_MANUAL_REQUIRED:{}\nDevice code expired ({}s). Visit the URL and enter code: {}",
                verify_url, device.expires_in, device.user_code
            ));
        }

        tokio::time::sleep(poll_interval).await;

        let poll_resp = client
            .post(&provider.token_url)
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "client_id": provider.client_id,
                "device_code": device.device_code,
                "grant_type": "urn:ietf:params:oauth:grant-type:device_code",
            }))
            .send()
            .await;

        let poll_resp = match poll_resp {
            Ok(r) => r,
            Err(e) => {
                warn!("Token poll error: {}", e);
                continue;
            }
        };

        let body: serde_json::Value = match poll_resp.json().await {
            Ok(v) => v,
            Err(_) => continue,
        };

        // Check for error responses
        if let Some(error) = body.get("error").and_then(|e| e.as_str()) {
            match error {
                "authorization_pending" => continue,
                "slow_down" => {
                    // RFC 8628 §3.5: MUST increase interval by 5s permanently
                    poll_interval += std::time::Duration::from_secs(5);
                    continue;
                }
                "expired_token" | "access_denied" => {
                    return Err(anyhow::anyhow!("{} OAuth: {}", provider.name, error));
                }
                _ => {
                    let desc = body.get("error_description").and_then(|d| d.as_str()).unwrap_or("");
                    return Err(anyhow::anyhow!("{} OAuth error: {} — {}", provider.name, error, desc));
                }
            }
        }

        // Success — extract tokens
        if let Some(token) = body.get("access_token").and_then(|t| t.as_str()) {
            info!("{} OAuth: authorized successfully", provider.name);
            return Ok(TokenResponse {
                access_token: token.to_string(),
                refresh_token: body.get("refresh_token").and_then(|r| r.as_str()).map(String::from),
                token_type: body.get("token_type").and_then(|t| t.as_str()).unwrap_or("Bearer").to_string(),
                expires_in: body.get("expires_in").and_then(|e| e.as_u64()),
                scope: body.get("scope").and_then(|s| s.as_str()).map(String::from),
            });
        }
    }
}
