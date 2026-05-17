//! Email channel adapter
//!
//! Provides email communication:
//! - SMTP for sending messages via lettre
//! - IMAP polling for receiving with seen message tracking

use crate::{ChannelAdapter, ChannelMessage, ChannelSource, ReceiveMode};
use crate::policy::ChannelPolicy;
use async_trait::async_trait;
use futures::StreamExt;
use lettre::{
    AsyncSmtpTransport, AsyncTransport, Message as LettreMessage, Tokio1Executor,
    message::header::ContentType, transport::smtp::authentication::Credentials,
};
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::{Notify, RwLock, mpsc};
use tokio_rustls::TlsConnector;
use tokio_rustls::rustls::ClientConfig;
use tokio_util::compat::TokioAsyncReadCompatExt;
use zeus_core::{Error, Result};

/// Email channel adapter
pub struct EmailAdapter {
    connected: Arc<AtomicBool>,
    config: EmailConfig,
    shutdown: Arc<Notify>,
    /// Seen message UIDs to avoid duplicates
    seen_uids: Arc<RwLock<HashSet<u32>>>,
    /// Handle to the receive task
    task_handle: RwLock<Option<tokio::task::JoinHandle<()>>>,
}

impl EmailAdapter {
    /// Create a new Email adapter
    pub async fn new(config: EmailConfig) -> Result<Self> {
        // Validate configuration
        if config.email.is_empty() {
            return Err(Error::Config("Email address is required".into()));
        }
        if config.password.is_empty() {
            return Err(Error::Config("Email password is required".into()));
        }
        if config.smtp_server.is_empty() {
            return Err(Error::Config("SMTP server is required".into()));
        }

        tracing::info!(email = %config.email, "Email adapter created");

        Ok(Self {
            connected: Arc::new(AtomicBool::new(false)),
            config,
            shutdown: Arc::new(Notify::new()),
            seen_uids: Arc::new(RwLock::new(HashSet::new())),
            task_handle: RwLock::new(None),
        })
    }

    /// Start IMAP polling for incoming emails
    async fn start_imap_polling(&self, tx: mpsc::Sender<ChannelMessage>) -> Result<()> {
        if self.config.imap_server.is_empty() {
            tracing::info!("IMAP not configured, running in send-only mode");
            return Ok(());
        }

        let config = self.config.clone();
        let shutdown = self.shutdown.clone();
        let connected = self.connected.clone();
        let seen_uids = self.seen_uids.clone();

        let handle = tokio::spawn(async move {
            let poll_interval = Duration::from_secs(config.poll_interval_secs);

            loop {
                tokio::select! {
                    _ = tokio::time::sleep(poll_interval) => {
                        if !connected.load(Ordering::SeqCst) {
                            break;
                        }

                        if let Err(e) = Self::poll_imap(&config, &tx, &seen_uids).await {
                            tracing::error!(error = %e, "IMAP poll error");
                        }
                    }
                    _ = shutdown.notified() => {
                        tracing::info!("Email IMAP polling shutdown");
                        break;
                    }
                }
            }
        });

        *self.task_handle.write().await = Some(handle);
        tracing::info!("Email IMAP polling started");
        Ok(())
    }

    /// Poll IMAP for new emails
    async fn poll_imap(
        config: &EmailConfig,
        tx: &mpsc::Sender<ChannelMessage>,
        seen_uids: &RwLock<HashSet<u32>>,
    ) -> Result<()> {
        // Connect to IMAP server
        let imap_addr = (config.imap_server.as_str(), config.imap_port);
        let tcp_stream = tokio::net::TcpStream::connect(imap_addr)
            .await
            .map_err(|e| Error::Channel(format!("Failed to connect to IMAP: {}", e)))?;

        // Set up TLS with tokio-rustls
        let root_store = tokio_rustls::rustls::RootCertStore::from_iter(
            webpki_roots::TLS_SERVER_ROOTS.iter().cloned()
        );
        let tls_config = ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();
        let tls_connector = TlsConnector::from(Arc::new(tls_config));
        let server_name = tokio_rustls::rustls::pki_types::ServerName::try_from(config.imap_server.clone())
            .map_err(|e| Error::Channel(format!("Invalid IMAP server name: {}", e)))?;
        let tls_stream = tls_connector
            .connect(server_name, tcp_stream)
            .await
            .map_err(|e| Error::Channel(format!("IMAP TLS error: {}", e)))?;

        // Wrap with compat layer for async-imap (uses futures traits)
        let compat_stream = tls_stream.compat();

        let client = async_imap::Client::new(compat_stream);
        let mut session = client
            .login(&config.email, &config.password)
            .await
            .map_err(|(e, _)| Error::Channel(format!("IMAP login failed: {}", e)))?;

        // Select INBOX
        let mailbox = session
            .select(&config.inbox_folder)
            .await
            .map_err(|e| Error::Channel(format!("Failed to select mailbox: {}", e)))?;

        tracing::debug!(
            exists = mailbox.exists,
            unseen = ?mailbox.unseen,
            "IMAP mailbox selected"
        );

        // Search for unseen messages
        let search_result = session
            .search("UNSEEN")
            .await
            .map_err(|e| Error::Channel(format!("IMAP search failed: {}", e)))?;

        if search_result.is_empty() {
            session.logout().await.ok();
            return Ok(());
        }

        // Fetch the messages
        let uids: Vec<String> = search_result.iter().map(|u| u.to_string()).collect();
        let uid_range = uids.join(",");

        let mut fetch_stream = session
            .fetch(&uid_range, "(UID ENVELOPE BODY[TEXT])")
            .await
            .map_err(|e| Error::Channel(format!("IMAP fetch failed: {}", e)))?;

        let policy = ChannelPolicy::new(config.policy.clone().unwrap_or_default());

        // AllowBotsMode: email has no native bot flag; all-or-nothing only
        // Note: allow_bots field does not exist in EmailConfig — email is implicitly DM-only
        let _ = config.email.as_str(); // suppress unused warning

        while let Some(result) = fetch_stream.next().await {
            let fetch = result.map_err(|e| Error::Channel(format!("IMAP fetch error: {}", e)))?;

            if let Some(uid) = fetch.uid {
                // Skip if we've already seen this message
                if seen_uids.read().await.contains(&uid) {
                    continue;
                }

                // Extract message details
                let (from_address, subject, body) = Self::parse_fetch(&fetch);

                if let Some(from) = from_address {
                    // Layer 1: policy — all emails are DMs
                    if policy.check_dm(&from).is_denied() {
                        tracing::debug!(from = %from, "Email message denied by policy");
                        continue;
                    }

                    let source = ChannelSource::new("email", &from);

                    // All emails are DMs — no group concept; from_address IS the addressing signal
                    let is_addressed = true;

                    let content = if let Some(ref subj) = subject {
                        format!("[{}] {}", subj, body.as_deref().unwrap_or(""))
                    } else {
                        body.unwrap_or_default()
                    };

                    let message = ChannelMessage::new(source, content).with_addressed(is_addressed);

                    if let Err(e) = tx.send(message).await {
                        tracing::error!(error = %e, "Failed to forward email");
                    } else {
                        tracing::info!(
                            from = %from,
                            subject = ?subject,
                            uid = uid,
                            "Received email"
                        );
                        // Mark as seen locally
                        seen_uids.write().await.insert(uid);
                    }
                }
            }
        }

        drop(fetch_stream);
        session.logout().await.ok();
        Ok(())
    }

    /// Parse a fetch result into message components
    fn parse_fetch(
        fetch: &async_imap::types::Fetch,
    ) -> (Option<String>, Option<String>, Option<String>) {
        let envelope = fetch.envelope();
        let body = fetch.text();

        let from_address = envelope.and_then(|env| {
            env.from.as_ref().and_then(|addrs| {
                addrs.first().and_then(|addr| {
                    let mailbox = addr
                        .mailbox
                        .as_ref()
                        .map(|s| String::from_utf8_lossy(s).to_string())?;
                    let host = addr
                        .host
                        .as_ref()
                        .map(|s| String::from_utf8_lossy(s).to_string())?;
                    Some(format!("{}@{}", mailbox, host))
                })
            })
        });

        let subject = envelope.and_then(|env| {
            env.subject
                .as_ref()
                .map(|s| String::from_utf8_lossy(s).to_string())
        });

        let body_text = body.map(|b| String::from_utf8_lossy(b).to_string());

        (from_address, subject, body_text)
    }

    /// Build SMTP transport
    fn build_smtp_transport(&self) -> Result<AsyncSmtpTransport<Tokio1Executor>> {
        let creds = Credentials::new(self.config.email.clone(), self.config.password.clone());

        let transport = if self.config.use_tls {
            AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&self.config.smtp_server)
                .map_err(|e| Error::Config(format!("Failed to create SMTP transport: {}", e)))?
                .port(self.config.smtp_port)
                .credentials(creds)
                .build()
        } else {
            AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&self.config.smtp_server)
                .port(self.config.smtp_port)
                .credentials(creds)
                .build()
        };

        Ok(transport)
    }

    /// Send an email
    pub async fn send_email(&self, to: &str, subject: &str, body: &str) -> Result<()> {
        let transport = self.build_smtp_transport()?;

        let email = LettreMessage::builder()
            .from(
                self.config
                    .email
                    .parse()
                    .map_err(|e| Error::Config(format!("Invalid from address: {}", e)))?,
            )
            .to(to
                .parse()
                .map_err(|e| Error::Config(format!("Invalid to address: {}", e)))?)
            .subject(subject)
            .header(ContentType::TEXT_PLAIN)
            .body(body.to_string())
            .map_err(|e| Error::Channel(format!("Failed to build email: {}", e)))?;

        transport
            .send(email)
            .await
            .map_err(|e| Error::Channel(format!("Failed to send email: {}", e)))?;

        tracing::info!(to = %to, subject = %subject, "Email sent");
        Ok(())
    }

    /// Test SMTP connection
    pub async fn test_connection(&self) -> Result<bool> {
        let transport = self.build_smtp_transport()?;
        transport
            .test_connection()
            .await
            .map_err(|e| Error::Channel(format!("SMTP connection test failed: {}", e)))
    }
}

#[async_trait]
impl ChannelAdapter for EmailAdapter {
    fn channel_type(&self) -> &'static str {
        "email"
    }

    fn receive_mode(&self) -> ReceiveMode {
        if !self.config.imap_server.is_empty() {
            ReceiveMode::Polling {
                interval_secs: self.config.poll_interval_secs,
            }
        } else {
            ReceiveMode::None
        }
    }

    async fn start(&self, tx: mpsc::Sender<ChannelMessage>) -> Result<()> {
        self.connected.store(true, Ordering::SeqCst);

        tracing::info!(
            email = %self.config.email,
            "Email adapter started (SMTP outbound ready)"
        );

        // Start IMAP polling if configured
        self.start_imap_polling(tx).await?;

        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        self.connected.store(false, Ordering::SeqCst);
        self.shutdown.notify_one();

        // Wait for the polling task to finish
        if let Some(handle) = self.task_handle.write().await.take() {
            let _ = handle.await;
        }

        tracing::info!("Email adapter stopped");
        Ok(())
    }

    async fn send(&self, to: &ChannelSource, content: &str) -> Result<()> {
        if to.channel_type() != "email" {
            return Err(Error::channel("Invalid channel source for Email"));
        }

        // user_id is the recipient email address
        let to_address = &to.user_id;
        let subject = to.chat_id.as_deref().unwrap_or("Message from Zeus");

        self.send_email(to_address, subject, content).await
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    fn supports_native_identity(&self) -> bool {
        false
    }
}

/// Email configuration
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct EmailConfig {
    /// SMTP server
    pub smtp_server: String,
    /// SMTP port
    #[serde(default = "default_smtp_port")]
    pub smtp_port: u16,
    /// IMAP server (optional, for receiving)
    #[serde(default)]
    pub imap_server: String,
    /// IMAP port
    #[serde(default = "default_imap_port")]
    pub imap_port: u16,
    /// IMAP inbox folder name
    #[serde(default = "default_inbox_folder")]
    pub inbox_folder: String,
    /// Email address
    pub email: String,
    /// Password (use app password for Gmail)
    #[serde(skip_serializing)]
    pub password: String,
    /// Use TLS
    #[serde(default = "default_use_tls")]
    pub use_tls: bool,
    /// Poll interval in seconds (for IMAP)
    #[serde(default = "default_poll_interval")]
    pub poll_interval_secs: u64,
    /// Access policy (DM access control)
    #[serde(default)]
    pub policy: Option<zeus_core::ChannelPolicyConfig>,
}

fn default_smtp_port() -> u16 {
    587
}

fn default_imap_port() -> u16 {
    993
}

fn default_inbox_folder() -> String {
    "INBOX".to_string()
}

fn default_use_tls() -> bool {
    true
}

fn default_poll_interval() -> u64 {
    60 // Check every minute
}

impl Default for EmailConfig {
    fn default() -> Self {
        Self {
            smtp_server: "smtp.gmail.com".to_string(),
            smtp_port: default_smtp_port(),
            imap_server: String::new(),
            imap_port: default_imap_port(),
            inbox_folder: default_inbox_folder(),
            email: String::new(),
            password: String::new(),
            use_tls: default_use_tls(),
            poll_interval_secs: default_poll_interval(),
            policy: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AgentSendIdentity;

    #[test]
    fn test_email_config_defaults() {
        let config = EmailConfig::default();
        assert_eq!(config.smtp_port, 587);
        assert_eq!(config.imap_port, 993);
        assert!(config.use_tls);
        assert_eq!(config.poll_interval_secs, 60);
    }

    #[tokio::test]
    async fn test_email_adapter_validation() {
        // Empty email should fail
        let config = EmailConfig {
            email: String::new(),
            password: "password".to_string(),
            ..Default::default()
        };
        assert!(EmailAdapter::new(config).await.is_err());

        // Empty password should fail
        let config = EmailConfig {
            email: "test@example.com".to_string(),
            password: String::new(),
            ..Default::default()
        };
        assert!(EmailAdapter::new(config).await.is_err());

        // Empty SMTP server should fail
        let config = EmailConfig {
            email: "test@example.com".to_string(),
            password: "password".to_string(),
            smtp_server: String::new(),
            ..Default::default()
        };
        assert!(EmailAdapter::new(config).await.is_err());
    }

    #[tokio::test]
    async fn test_email_adapter_lifecycle() {
        let config = EmailConfig {
            email: "test@example.com".to_string(),
            password: "password".to_string(),
            smtp_server: "smtp.example.com".to_string(),
            ..Default::default()
        };

        let adapter = EmailAdapter::new(config)
            .await
            .expect("EmailAdapter::new should succeed");
        assert!(!adapter.is_connected());
        assert_eq!(adapter.channel_type(), "email");
    }

    #[tokio::test]
    async fn test_email_adapter_start_stop() {
        let config = EmailConfig {
            email: "test@example.com".to_string(),
            password: "password".to_string(),
            smtp_server: "smtp.example.com".to_string(),
            ..Default::default()
        };

        let adapter = EmailAdapter::new(config)
            .await
            .expect("EmailAdapter::new should succeed");
        let (tx, _rx) = mpsc::channel(100);

        adapter
            .start(tx)
            .await
            .expect("async operation should succeed");
        assert!(adapter.is_connected());

        adapter
            .stop()
            .await
            .expect("async operation should succeed");
        assert!(!adapter.is_connected());
    }

    // ── S33 Track D: Tier 2 identity tests ──────────────────────────────────

    #[tokio::test]
    async fn test_email_supports_native_identity_false() {
        let config = EmailConfig {
            email: "test@example.com".to_string(),
            password: "password".to_string(),
            smtp_server: "smtp.example.com".to_string(),
            ..Default::default()
        };
        let adapter = EmailAdapter::new(config)
            .await
            .expect("EmailAdapter::new should succeed");
        assert!(!adapter.supports_native_identity());
    }

    #[test]
    fn test_email_send_as_text_prefix_format() {
        let identity = AgentSendIdentity::new("zeus_agent");
        let prefixed = identity.apply_prefix("Hello from Email");
        assert_eq!(prefixed, "[zeus_agent] Hello from Email");
    }
}
