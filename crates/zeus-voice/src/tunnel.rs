//! Tunnel auto-setup for exposing voice webhooks publicly
//!
//! Supports ngrok, Tailscale Funnel, and custom (static) tunnel providers.
//! Manages tunnel subprocesses and returns public HTTPS URLs for webhook use.

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

/// Tunnel provider selection
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TunnelProvider {
    /// ngrok — spawns `ngrok http {port}`, queries local API for public URL
    Ngrok,
    /// Tailscale Funnel — spawns `tailscale funnel {port}`, parses output
    Tailscale,
    /// User-provided static URL (no subprocess)
    Custom,
}

impl std::fmt::Display for TunnelProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TunnelProvider::Ngrok => write!(f, "ngrok"),
            TunnelProvider::Tailscale => write!(f, "tailscale"),
            TunnelProvider::Custom => write!(f, "custom"),
        }
    }
}

impl std::str::FromStr for TunnelProvider {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "ngrok" => Ok(TunnelProvider::Ngrok),
            "tailscale" => Ok(TunnelProvider::Tailscale),
            "custom" => Ok(TunnelProvider::Custom),
            _ => bail!(
                "Unknown tunnel provider: '{}'. Expected ngrok, tailscale, or custom",
                s
            ),
        }
    }
}

fn default_local_port() -> u16 {
    std::env::var("ZEUS_WEBHOOK_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8090)
}

/// Configuration for tunnel auto-setup
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TunnelConfig {
    /// Which tunnel provider to use
    pub provider: TunnelProvider,
    /// Local port to tunnel (default: 8090, same as webhook_port)
    #[serde(default = "default_local_port")]
    pub local_port: u16,
    /// ngrok auth token (optional for free tier)
    pub ngrok_auth_token: Option<String>,
    /// Custom ngrok domain (paid feature)
    pub ngrok_domain: Option<String>,
    /// Tailscale funnel hostname
    pub tailscale_hostname: Option<String>,
    /// Auto-start tunnel when voice system initializes
    #[serde(default)]
    pub auto_start: bool,
    /// Custom static URL (used with TunnelProvider::Custom)
    pub custom_url: Option<String>,
}

impl Default for TunnelConfig {
    fn default() -> Self {
        Self {
            provider: TunnelProvider::Ngrok,
            local_port: default_local_port(),
            ngrok_auth_token: None,
            ngrok_domain: None,
            tailscale_hostname: None,
            auto_start: false,
            custom_url: None,
        }
    }
}

impl TunnelConfig {
    /// Apply environment variable overrides to the config.
    ///
    /// Checks the following env vars:
    /// - `NGROK_AUTH_TOKEN` -> ngrok_auth_token
    /// - `TAILSCALE_HOSTNAME` -> tailscale_hostname
    /// - `ZEUS_TUNNEL_PROVIDER` -> provider ("ngrok", "tailscale", "custom")
    /// - `ZEUS_TUNNEL_URL` -> custom_url (also sets provider to Custom)
    pub fn with_env_overrides(mut self) -> Self {
        if let Ok(token) = std::env::var("NGROK_AUTH_TOKEN") {
            self.ngrok_auth_token = Some(token);
        }
        if let Ok(hostname) = std::env::var("TAILSCALE_HOSTNAME") {
            self.tailscale_hostname = Some(hostname);
        }
        if let Ok(provider_str) = std::env::var("ZEUS_TUNNEL_PROVIDER") {
            if let Ok(provider) = provider_str.parse::<TunnelProvider>() {
                self.provider = provider;
            } else {
                warn!(
                    "Invalid ZEUS_TUNNEL_PROVIDER value '{}', ignoring",
                    provider_str
                );
            }
        }
        if let Ok(url) = std::env::var("ZEUS_TUNNEL_URL") {
            self.custom_url = Some(url);
            self.provider = TunnelProvider::Custom;
        }
        self
    }

    /// Create a config from environment variables with defaults.
    pub fn from_env() -> Self {
        Self::default().with_env_overrides()
    }
}

/// Status of the tunnel
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TunnelStatus {
    /// Tunnel is not running
    Stopped,
    /// Tunnel process is starting up
    Starting,
    /// Tunnel is active and forwarding traffic
    Active,
    /// Tunnel process exited or encountered an error
    Error(String),
}

impl std::fmt::Display for TunnelStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TunnelStatus::Stopped => write!(f, "stopped"),
            TunnelStatus::Starting => write!(f, "starting"),
            TunnelStatus::Active => write!(f, "active"),
            TunnelStatus::Error(msg) => write!(f, "error: {}", msg),
        }
    }
}

/// Manages a tunnel subprocess and provides the public URL
pub struct TunnelManager {
    config: TunnelConfig,
    child: Option<tokio::process::Child>,
    public_url: Option<String>,
}

impl TunnelManager {
    /// Create a new TunnelManager with the given configuration
    pub fn new(config: TunnelConfig) -> Self {
        Self {
            config,
            child: None,
            public_url: None,
        }
    }

    /// Start the tunnel process and return the public HTTPS URL
    pub async fn start(&mut self) -> anyhow::Result<String> {
        match &self.config.provider {
            TunnelProvider::Ngrok => self.start_ngrok().await,
            TunnelProvider::Tailscale => self.start_tailscale().await,
            TunnelProvider::Custom => self.start_custom(),
        }
    }

    /// Stop the tunnel subprocess
    pub async fn stop(&mut self) -> anyhow::Result<()> {
        if let Some(ref mut child) = self.child {
            info!(
                "Stopping tunnel process (provider: {})",
                self.config.provider
            );
            child
                .kill()
                .await
                .context("Failed to kill tunnel process")?;
            self.child = None;
            self.public_url = None;
            info!("Tunnel process stopped");
        }
        Ok(())
    }

    /// Returns the current public URL, if the tunnel is running
    pub fn public_url(&self) -> Option<&str> {
        self.public_url.as_deref()
    }

    /// Whether the tunnel subprocess is currently running
    pub fn is_running(&self) -> bool {
        // For Custom provider, "running" means we have a URL
        if self.config.provider == TunnelProvider::Custom {
            return self.public_url.is_some();
        }
        self.child.is_some()
    }

    /// Check tunnel health by verifying the subprocess and/or URL
    pub async fn health_check(&mut self) -> anyhow::Result<TunnelStatus> {
        if self.config.provider == TunnelProvider::Custom {
            return if self.public_url.is_some() {
                Ok(TunnelStatus::Active)
            } else {
                Ok(TunnelStatus::Stopped)
            };
        }

        match &mut self.child {
            None => Ok(TunnelStatus::Stopped),
            Some(child) => {
                // Check if process is still alive
                match child.try_wait() {
                    Ok(Some(status)) => {
                        // Process exited
                        let msg = format!("Tunnel process exited with {}", status);
                        warn!("{}", msg);
                        self.child = None;
                        self.public_url = None;
                        Ok(TunnelStatus::Error(msg))
                    }
                    Ok(None) => {
                        // Still running
                        Ok(TunnelStatus::Active)
                    }
                    Err(e) => {
                        let msg = format!("Failed to check tunnel process: {}", e);
                        Ok(TunnelStatus::Error(msg))
                    }
                }
            }
        }
    }

    /// Start ngrok tunnel
    async fn start_ngrok(&mut self) -> anyhow::Result<String> {
        let port = self.config.local_port;
        info!("Starting ngrok tunnel on port {}", port);

        let mut cmd = tokio::process::Command::new("ngrok");
        cmd.arg("http").arg(port.to_string());

        if let Some(ref token) = self.config.ngrok_auth_token {
            cmd.arg("--authtoken").arg(token);
        }
        if let Some(ref domain) = self.config.ngrok_domain {
            cmd.arg("--domain").arg(domain);
        }

        // Suppress ngrok's stdout/stderr to avoid cluttering logs
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());

        let child = cmd
            .spawn()
            .context("Failed to spawn ngrok. Is ngrok installed and in PATH?")?;
        self.child = Some(child);

        // Give ngrok time to start and register its tunnel
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        // Query the ngrok local API for the public URL
        let url = self
            .query_ngrok_api()
            .await
            .context("Failed to get public URL from ngrok API")?;

        info!("ngrok tunnel active: {}", url);
        self.public_url = Some(url.clone());
        Ok(url)
    }

    /// Query the ngrok local API to retrieve the public HTTPS URL
    async fn query_ngrok_api(&self) -> anyhow::Result<String> {
        let ngrok_api = std::env::var("ZEUS_NGROK_API_URL")
            .unwrap_or_else(|_| "http://localhost:4040".to_string());
        let client = reqwest::Client::new();
        let resp = client
            .get(format!("{}/api/tunnels", ngrok_api))
            .send()
            .await
            .with_context(|| format!("Failed to connect to ngrok API at {}", ngrok_api))?;

        let body = resp
            .text()
            .await
            .context("Failed to read ngrok API response")?;

        parse_ngrok_api_response(&body)
    }

    /// Start Tailscale Funnel tunnel
    async fn start_tailscale(&mut self) -> anyhow::Result<String> {
        let port = self.config.local_port;

        // If hostname is provided, we can construct the URL directly
        if let Some(ref hostname) = self.config.tailscale_hostname {
            let url = format!("https://{}.ts.net", hostname);
            info!(
                "Starting Tailscale Funnel on port {} (hostname: {})",
                port, hostname
            );

            let mut cmd = tokio::process::Command::new("tailscale");
            cmd.arg("funnel").arg(port.to_string());
            cmd.stdout(std::process::Stdio::piped());
            cmd.stderr(std::process::Stdio::piped());

            let child = cmd
                .spawn()
                .context("Failed to spawn tailscale. Is tailscale installed and in PATH?")?;
            self.child = Some(child);

            // Give Tailscale time to set up the funnel
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;

            info!("Tailscale Funnel active: {}", url);
            self.public_url = Some(url.clone());
            return Ok(url);
        }

        // No hostname provided — spawn and parse stdout
        info!(
            "Starting Tailscale Funnel on port {} (parsing output for URL)",
            port
        );

        let mut cmd = tokio::process::Command::new("tailscale");
        cmd.arg("funnel").arg(port.to_string());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let mut child = cmd
            .spawn()
            .context("Failed to spawn tailscale. Is tailscale installed and in PATH?")?;

        // Read stdout to find the URL
        let stdout = child
            .stdout
            .take()
            .context("Failed to capture tailscale stdout")?;

        let url = parse_tailscale_output_async(stdout).await?;

        info!("Tailscale Funnel active: {}", url);
        self.child = Some(child);
        self.public_url = Some(url.clone());
        Ok(url)
    }

    /// Start custom (static URL) tunnel — no subprocess needed
    fn start_custom(&mut self) -> anyhow::Result<String> {
        let url = self
            .config
            .custom_url
            .clone()
            .context("Custom tunnel provider requires custom_url to be set")?;

        if url.is_empty() {
            bail!("Custom tunnel URL is empty");
        }

        info!("Using custom tunnel URL: {}", url);
        self.public_url = Some(url.clone());
        Ok(url)
    }
}

/// Parse the ngrok API JSON response to extract the HTTPS public URL.
///
/// Expected response format:
/// ```json
/// {
///   "tunnels": [
///     {
///       "public_url": "https://abc123.ngrok.io",
///       "proto": "https",
///       ...
///     },
///     {
///       "public_url": "http://abc123.ngrok.io",
///       "proto": "http",
///       ...
///     }
///   ]
/// }
/// ```
pub fn parse_ngrok_api_response(body: &str) -> anyhow::Result<String> {
    let json: serde_json::Value =
        serde_json::from_str(body).context("Failed to parse ngrok API JSON")?;

    let tunnels = json
        .get("tunnels")
        .and_then(|t| t.as_array())
        .context("ngrok API response missing 'tunnels' array")?;

    if tunnels.is_empty() {
        bail!("ngrok API returned no tunnels — tunnel may still be initializing");
    }

    // Prefer HTTPS tunnel
    for tunnel in tunnels {
        let proto = tunnel.get("proto").and_then(|p| p.as_str()).unwrap_or("");
        if proto == "https"
            && let Some(url) = tunnel.get("public_url").and_then(|u| u.as_str())
        {
            return Ok(url.to_string());
        }
    }

    // Fallback: take the first tunnel's public_url
    let first_url = tunnels[0]
        .get("public_url")
        .and_then(|u| u.as_str())
        .context("ngrok tunnel entry missing 'public_url' field")?;

    Ok(first_url.to_string())
}

/// Parse Tailscale Funnel stdout output to extract the public URL.
///
/// Tailscale output typically contains a line like:
/// ```text
/// https://hostname.ts.net/
/// ```
/// or:
/// ```text
/// Available on the internet:
/// https://hostname.tail1234.ts.net:443/
/// ```
pub fn parse_tailscale_output(output: &str) -> anyhow::Result<String> {
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("https://") && trimmed.contains(".ts.net") {
            // Strip trailing slash and port for a clean URL
            let url = trimmed.trim_end_matches('/');
            // Remove :443 if present (it's the default for HTTPS)
            let url = url.replace(":443", "");
            return Ok(url);
        }
    }
    bail!(
        "Could not find HTTPS URL in Tailscale output. Output was:\n{}",
        output
    )
}

/// Async version: reads from tailscale stdout with a timeout
async fn parse_tailscale_output_async(
    stdout: tokio::process::ChildStdout,
) -> anyhow::Result<String> {
    use tokio::io::AsyncReadExt;

    let mut reader = stdout;
    let mut buf = vec![0u8; 4096];
    let mut collected = String::new();

    // Read with a timeout — tailscale should output the URL quickly
    let timeout = std::time::Duration::from_secs(10);
    let result = tokio::time::timeout(timeout, async {
        loop {
            let n = reader.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            collected.push_str(&String::from_utf8_lossy(&buf[..n]));

            // Try to parse after each read
            if let Ok(url) = parse_tailscale_output(&collected) {
                return Ok::<String, anyhow::Error>(url);
            }
        }
        parse_tailscale_output(&collected)
    })
    .await;

    match result {
        Ok(Ok(url)) => Ok(url),
        Ok(Err(e)) => Err(e),
        Err(_) => bail!(
            "Timed out waiting for Tailscale URL (10s). Collected output:\n{}",
            collected
        ),
    }
}

/// Integration helper: create a VoiceConfig with the tunnel's public URL
/// auto-populated as the webhook_base_url.
impl crate::VoiceConfig {
    /// Set the webhook base URL from a tunnel's public URL.
    pub fn with_tunnel_url(mut self, tunnel_url: &str) -> Self {
        self.webhook_base_url = tunnel_url.to_string();
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── TunnelProvider tests ──────────────────────────────────────────

    #[test]
    fn test_tunnel_provider_display() {
        assert_eq!(TunnelProvider::Ngrok.to_string(), "ngrok");
        assert_eq!(TunnelProvider::Tailscale.to_string(), "tailscale");
        assert_eq!(TunnelProvider::Custom.to_string(), "custom");
    }

    #[test]
    fn test_tunnel_provider_from_str() {
        assert_eq!(
            "ngrok"
                .parse::<TunnelProvider>()
                .expect("should parse successfully"),
            TunnelProvider::Ngrok
        );
        assert_eq!(
            "tailscale"
                .parse::<TunnelProvider>()
                .expect("should parse successfully"),
            TunnelProvider::Tailscale
        );
        assert_eq!(
            "custom"
                .parse::<TunnelProvider>()
                .expect("should parse successfully"),
            TunnelProvider::Custom
        );
        // Case insensitive
        assert_eq!(
            "NGROK"
                .parse::<TunnelProvider>()
                .expect("should parse successfully"),
            TunnelProvider::Ngrok
        );
        assert_eq!(
            "Tailscale"
                .parse::<TunnelProvider>()
                .expect("should parse successfully"),
            TunnelProvider::Tailscale
        );
    }

    #[test]
    fn test_tunnel_provider_from_str_invalid() {
        assert!("invalid".parse::<TunnelProvider>().is_err());
        assert!("".parse::<TunnelProvider>().is_err());
    }

    #[test]
    fn test_tunnel_provider_serialization() {
        let json = serde_json::to_string(&TunnelProvider::Ngrok).expect("should serialize to JSON");
        assert_eq!(json, r#""ngrok""#);

        let json =
            serde_json::to_string(&TunnelProvider::Tailscale).expect("should serialize to JSON");
        assert_eq!(json, r#""tailscale""#);

        let json =
            serde_json::to_string(&TunnelProvider::Custom).expect("should serialize to JSON");
        assert_eq!(json, r#""custom""#);
    }

    #[test]
    fn test_tunnel_provider_deserialization() {
        let p: TunnelProvider =
            serde_json::from_str(r#""ngrok""#).expect("should parse successfully");
        assert_eq!(p, TunnelProvider::Ngrok);

        let p: TunnelProvider =
            serde_json::from_str(r#""tailscale""#).expect("should parse successfully");
        assert_eq!(p, TunnelProvider::Tailscale);

        let p: TunnelProvider =
            serde_json::from_str(r#""custom""#).expect("should parse successfully");
        assert_eq!(p, TunnelProvider::Custom);
    }

    // ── TunnelConfig tests ───────────────────────────────────────────

    #[test]
    fn test_tunnel_config_defaults() {
        let config = TunnelConfig::default();
        assert_eq!(config.provider, TunnelProvider::Ngrok);
        assert_eq!(config.local_port, 8090);
        assert!(config.ngrok_auth_token.is_none());
        assert!(config.ngrok_domain.is_none());
        assert!(config.tailscale_hostname.is_none());
        assert!(!config.auto_start);
        assert!(config.custom_url.is_none());
    }

    #[test]
    fn test_tunnel_config_serialization_roundtrip() {
        let config = TunnelConfig {
            provider: TunnelProvider::Ngrok,
            local_port: 9090,
            ngrok_auth_token: Some("token123".to_string()),
            ngrok_domain: Some("my-domain.ngrok.io".to_string()),
            tailscale_hostname: None,
            auto_start: true,
            custom_url: None,
        };

        let json = serde_json::to_string(&config).expect("should serialize to JSON");
        let deserialized: TunnelConfig =
            serde_json::from_str(&json).expect("should parse successfully");

        assert_eq!(deserialized.provider, TunnelProvider::Ngrok);
        assert_eq!(deserialized.local_port, 9090);
        assert_eq!(deserialized.ngrok_auth_token.as_deref(), Some("token123"));
        assert_eq!(
            deserialized.ngrok_domain.as_deref(),
            Some("my-domain.ngrok.io")
        );
        assert!(deserialized.auto_start);
    }

    #[test]
    fn test_tunnel_config_deserialize_with_defaults() {
        let json = r#"{"provider":"tailscale"}"#;
        let config: TunnelConfig = serde_json::from_str(json).expect("should parse successfully");
        assert_eq!(config.provider, TunnelProvider::Tailscale);
        assert_eq!(config.local_port, 8090);
        assert!(!config.auto_start);
    }

    #[test]
    fn test_tunnel_config_env_overrides_ngrok_token() {
        temp_env::with_vars(
            [
                ("NGROK_AUTH_TOKEN", Some("env_ngrok_token")),
                ("TAILSCALE_HOSTNAME", None),
                ("ZEUS_TUNNEL_PROVIDER", None),
                ("ZEUS_TUNNEL_URL", None),
            ],
            || {
                let config = TunnelConfig::default().with_env_overrides();
                assert_eq!(config.ngrok_auth_token.as_deref(), Some("env_ngrok_token"));
            },
        );
    }

    #[test]
    fn test_tunnel_config_env_overrides_tailscale_hostname() {
        temp_env::with_vars(
            [
                ("NGROK_AUTH_TOKEN", None),
                ("TAILSCALE_HOSTNAME", Some("my-machine")),
                ("ZEUS_TUNNEL_PROVIDER", None),
                ("ZEUS_TUNNEL_URL", None),
            ],
            || {
                let config = TunnelConfig::default().with_env_overrides();
                assert_eq!(config.tailscale_hostname.as_deref(), Some("my-machine"));
            },
        );
    }

    #[test]
    fn test_tunnel_config_env_overrides_provider() {
        temp_env::with_vars(
            [
                ("NGROK_AUTH_TOKEN", None),
                ("TAILSCALE_HOSTNAME", None),
                ("ZEUS_TUNNEL_PROVIDER", Some("tailscale")),
                ("ZEUS_TUNNEL_URL", None),
            ],
            || {
                let config = TunnelConfig::default().with_env_overrides();
                assert_eq!(config.provider, TunnelProvider::Tailscale);
            },
        );
    }

    #[test]
    fn test_tunnel_config_env_overrides_custom_url() {
        temp_env::with_vars(
            [
                ("NGROK_AUTH_TOKEN", None),
                ("TAILSCALE_HOSTNAME", None),
                ("ZEUS_TUNNEL_PROVIDER", None),
                ("ZEUS_TUNNEL_URL", Some("https://my-custom-domain.com")),
            ],
            || {
                let config = TunnelConfig::default().with_env_overrides();
                assert_eq!(config.provider, TunnelProvider::Custom);
                assert_eq!(
                    config.custom_url.as_deref(),
                    Some("https://my-custom-domain.com")
                );
            },
        );
    }

    #[test]
    fn test_tunnel_config_env_invalid_provider_ignored() {
        temp_env::with_vars(
            [
                ("NGROK_AUTH_TOKEN", None),
                ("TAILSCALE_HOSTNAME", None),
                ("ZEUS_TUNNEL_PROVIDER", Some("invalid_provider")),
                ("ZEUS_TUNNEL_URL", None),
            ],
            || {
                let config = TunnelConfig::default().with_env_overrides();
                // Provider should remain the default (Ngrok) since the value was invalid
                assert_eq!(config.provider, TunnelProvider::Ngrok);
            },
        );
    }

    #[test]
    fn test_tunnel_config_from_env() {
        temp_env::with_vars(
            [
                ("NGROK_AUTH_TOKEN", None::<&str>),
                ("TAILSCALE_HOSTNAME", None),
                ("ZEUS_TUNNEL_PROVIDER", None),
                ("ZEUS_TUNNEL_URL", None),
            ],
            || {
                let config = TunnelConfig::from_env();
                assert_eq!(config.provider, TunnelProvider::Ngrok);
                assert_eq!(config.local_port, 8090);
            },
        );
    }

    // ── TunnelStatus tests ───────────────────────────────────────────

    #[test]
    fn test_tunnel_status_display() {
        assert_eq!(TunnelStatus::Stopped.to_string(), "stopped");
        assert_eq!(TunnelStatus::Starting.to_string(), "starting");
        assert_eq!(TunnelStatus::Active.to_string(), "active");
        assert_eq!(
            TunnelStatus::Error("process died".to_string()).to_string(),
            "error: process died"
        );
    }

    #[test]
    fn test_tunnel_status_serialization() {
        let json = serde_json::to_string(&TunnelStatus::Active).expect("should serialize to JSON");
        assert_eq!(json, r#""active""#);

        let json = serde_json::to_string(&TunnelStatus::Stopped).expect("should serialize to JSON");
        assert_eq!(json, r#""stopped""#);

        let json =
            serde_json::to_string(&TunnelStatus::Starting).expect("should serialize to JSON");
        assert_eq!(json, r#""starting""#);
    }

    #[test]
    fn test_tunnel_status_error_serialization() {
        let status = TunnelStatus::Error("tunnel crashed".to_string());
        let json = serde_json::to_string(&status).expect("should serialize to JSON");
        let deserialized: TunnelStatus =
            serde_json::from_str(&json).expect("should parse successfully");
        assert_eq!(
            deserialized,
            TunnelStatus::Error("tunnel crashed".to_string())
        );
    }

    #[test]
    fn test_tunnel_status_equality() {
        assert_eq!(TunnelStatus::Active, TunnelStatus::Active);
        assert_ne!(TunnelStatus::Active, TunnelStatus::Stopped);
        assert_ne!(
            TunnelStatus::Error("a".to_string()),
            TunnelStatus::Error("b".to_string())
        );
    }

    // ── TunnelManager tests ──────────────────────────────────────────

    #[test]
    fn test_tunnel_manager_new() {
        let config = TunnelConfig::default();
        let manager = TunnelManager::new(config);
        assert!(manager.public_url().is_none());
        assert!(!manager.is_running());
    }

    #[test]
    fn test_tunnel_manager_custom_no_subprocess() {
        let config = TunnelConfig {
            provider: TunnelProvider::Custom,
            custom_url: Some("https://my-server.com".to_string()),
            ..Default::default()
        };
        let manager = TunnelManager::new(config);
        assert!(!manager.is_running());
    }

    #[tokio::test]
    async fn test_tunnel_manager_start_custom() {
        let config = TunnelConfig {
            provider: TunnelProvider::Custom,
            custom_url: Some("https://my-tunnel.example.com".to_string()),
            ..Default::default()
        };
        let mut manager = TunnelManager::new(config);
        let url = manager
            .start()
            .await
            .expect("async operation should succeed");
        assert_eq!(url, "https://my-tunnel.example.com");
        assert_eq!(manager.public_url(), Some("https://my-tunnel.example.com"));
        assert!(manager.is_running());
    }

    #[tokio::test]
    async fn test_tunnel_manager_start_custom_missing_url() {
        let config = TunnelConfig {
            provider: TunnelProvider::Custom,
            custom_url: None,
            ..Default::default()
        };
        let mut manager = TunnelManager::new(config);
        let result = manager.start().await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("custom_url"));
    }

    #[tokio::test]
    async fn test_tunnel_manager_start_custom_empty_url() {
        let config = TunnelConfig {
            provider: TunnelProvider::Custom,
            custom_url: Some(String::new()),
            ..Default::default()
        };
        let mut manager = TunnelManager::new(config);
        let result = manager.start().await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty"));
    }

    #[tokio::test]
    async fn test_tunnel_manager_stop_when_not_running() {
        let config = TunnelConfig::default();
        let mut manager = TunnelManager::new(config);
        // Should not error even if nothing is running
        manager
            .stop()
            .await
            .expect("async operation should succeed");
        assert!(manager.public_url().is_none());
    }

    #[tokio::test]
    async fn test_tunnel_manager_health_check_stopped() {
        let config = TunnelConfig::default();
        let mut manager = TunnelManager::new(config);
        let status = manager
            .health_check()
            .await
            .expect("async operation should succeed");
        assert_eq!(status, TunnelStatus::Stopped);
    }

    #[tokio::test]
    async fn test_tunnel_manager_health_check_custom_active() {
        let config = TunnelConfig {
            provider: TunnelProvider::Custom,
            custom_url: Some("https://example.com".to_string()),
            ..Default::default()
        };
        let mut manager = TunnelManager::new(config);
        manager
            .start()
            .await
            .expect("async operation should succeed");
        let status = manager
            .health_check()
            .await
            .expect("async operation should succeed");
        assert_eq!(status, TunnelStatus::Active);
    }

    #[tokio::test]
    async fn test_tunnel_manager_health_check_custom_stopped() {
        let config = TunnelConfig {
            provider: TunnelProvider::Custom,
            custom_url: Some("https://example.com".to_string()),
            ..Default::default()
        };
        let mut manager = TunnelManager::new(config);
        // Not started yet
        let status = manager
            .health_check()
            .await
            .expect("async operation should succeed");
        assert_eq!(status, TunnelStatus::Stopped);
    }

    // ── ngrok API response parsing tests ─────────────────────────────

    #[test]
    fn test_parse_ngrok_api_response_https() {
        let response = r#"{
            "tunnels": [
                {
                    "name": "command_line",
                    "public_url": "https://abc123.ngrok.io",
                    "proto": "https",
                    "config": {"addr": "http://localhost:8090"}
                },
                {
                    "name": "command_line",
                    "public_url": "http://abc123.ngrok.io",
                    "proto": "http",
                    "config": {"addr": "http://localhost:8090"}
                }
            ]
        }"#;

        let url = parse_ngrok_api_response(response).expect("should parse successfully");
        assert_eq!(url, "https://abc123.ngrok.io");
    }

    #[test]
    fn test_parse_ngrok_api_response_https_only() {
        let response = r#"{
            "tunnels": [
                {
                    "public_url": "https://xyz789.ngrok-free.app",
                    "proto": "https"
                }
            ]
        }"#;

        let url = parse_ngrok_api_response(response).expect("should parse successfully");
        assert_eq!(url, "https://xyz789.ngrok-free.app");
    }

    #[test]
    fn test_parse_ngrok_api_response_http_fallback() {
        // Only HTTP tunnel (no HTTPS) — should still return it
        let response = r#"{
            "tunnels": [
                {
                    "public_url": "http://abc123.ngrok.io",
                    "proto": "http"
                }
            ]
        }"#;

        let url = parse_ngrok_api_response(response).expect("should parse successfully");
        assert_eq!(url, "http://abc123.ngrok.io");
    }

    #[test]
    fn test_parse_ngrok_api_response_empty_tunnels() {
        let response = r#"{"tunnels": []}"#;
        let result = parse_ngrok_api_response(response);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("no tunnels"));
    }

    #[test]
    fn test_parse_ngrok_api_response_missing_tunnels() {
        let response = r#"{"version": "2.0"}"#;
        let result = parse_ngrok_api_response(response);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("tunnels"));
    }

    #[test]
    fn test_parse_ngrok_api_response_invalid_json() {
        let result = parse_ngrok_api_response("not json");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_ngrok_api_response_ngrok_v3_format() {
        // ngrok v3 uses .ngrok-free.app domains
        let response = r#"{
            "tunnels": [
                {
                    "public_url": "https://1234-5678.ngrok-free.app",
                    "proto": "https",
                    "config": {"addr": "http://localhost:8090"}
                }
            ]
        }"#;

        let url = parse_ngrok_api_response(response).expect("should parse successfully");
        assert_eq!(url, "https://1234-5678.ngrok-free.app");
    }

    #[test]
    fn test_parse_ngrok_api_response_custom_domain() {
        let response = r#"{
            "tunnels": [
                {
                    "public_url": "https://my-custom-domain.ngrok.io",
                    "proto": "https"
                }
            ]
        }"#;

        let url = parse_ngrok_api_response(response).expect("should parse successfully");
        assert_eq!(url, "https://my-custom-domain.ngrok.io");
    }

    // ── Tailscale output parsing tests ───────────────────────────────

    #[test]
    fn test_parse_tailscale_output_simple() {
        let output = "https://my-machine.ts.net/\n";
        let url = parse_tailscale_output(output).expect("should parse successfully");
        assert_eq!(url, "https://my-machine.ts.net");
    }

    #[test]
    fn test_parse_tailscale_output_with_port() {
        let output = "Available on the internet:\nhttps://my-machine.tail1234.ts.net:443/\n";
        let url = parse_tailscale_output(output).expect("should parse successfully");
        assert_eq!(url, "https://my-machine.tail1234.ts.net");
    }

    #[test]
    fn test_parse_tailscale_output_with_prefix_text() {
        let output = "\
Funnel started.
Available on the internet:

https://hostname.ts.net/
  |-- / proxy http://127.0.0.1:8090
";
        let url = parse_tailscale_output(output).expect("should parse successfully");
        assert_eq!(url, "https://hostname.ts.net");
    }

    #[test]
    fn test_parse_tailscale_output_no_url() {
        let output = "Error: not connected to tailscale\n";
        let result = parse_tailscale_output(output);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Could not find"));
    }

    #[test]
    fn test_parse_tailscale_output_empty() {
        let result = parse_tailscale_output("");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_tailscale_output_indented() {
        let output = "  https://my-machine.ts.net/  \n";
        let url = parse_tailscale_output(output).expect("should parse successfully");
        assert_eq!(url, "https://my-machine.ts.net");
    }

    // ── VoiceConfig integration tests ────────────────────────────────

    #[test]
    fn test_voice_config_with_tunnel_url() {
        let config = crate::VoiceConfig::default().with_tunnel_url("https://abc123.ngrok.io");
        assert_eq!(config.webhook_base_url, "https://abc123.ngrok.io");
    }

    #[test]
    fn test_voice_config_with_tunnel_url_overwrites() {
        let config = crate::VoiceConfig {
            webhook_base_url: "https://old-url.com".to_string(),
            ..Default::default()
        }
        .with_tunnel_url("https://new-tunnel.ngrok-free.app");
        assert_eq!(config.webhook_base_url, "https://new-tunnel.ngrok-free.app");
    }
}
