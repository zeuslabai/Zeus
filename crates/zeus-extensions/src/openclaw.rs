//! OpenClaw extension compatibility layer.
//!
//! Parses `openclaw.plugin.json` manifests, discovers extensions from a
//! directory, and bridges OpenClaw ChannelPlugin adapters to Zeus channel
//! events via JSON-RPC over the Deno subprocess.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::{Extension, ExtensionError, ExtensionPermissions, ExtensionRegistry, ExtensionSource};

// ---------------------------------------------------------------------------
// OpenClaw manifest types
// ---------------------------------------------------------------------------

/// Parsed `openclaw.plugin.json` manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenClawManifest {
    /// Plugin identifier (e.g. "telegram", "discord").
    pub id: String,

    /// Channel types this plugin provides.
    #[serde(default)]
    pub channels: Vec<String>,

    /// JSON Schema for plugin configuration.
    #[serde(rename = "configSchema", default)]
    pub config_schema: serde_json::Value,

    /// Human-readable name (optional, defaults to id).
    #[serde(default)]
    pub name: Option<String>,

    /// Plugin description.
    #[serde(default)]
    pub description: Option<String>,

    /// Version string.
    #[serde(default)]
    pub version: Option<String>,
}

impl OpenClawManifest {
    /// Load a manifest from a JSON file.
    pub fn from_file(path: &Path) -> Result<Self, ExtensionError> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            ExtensionError::ImportFailed(format!("cannot read {}: {}", path.display(), e))
        })?;
        serde_json::from_str(&content).map_err(|e| {
            ExtensionError::InvalidExtension(format!("invalid manifest {}: {}", path.display(), e))
        })
    }

    /// Display name — uses `name` field if set, otherwise capitalizes `id`.
    pub fn display_name(&self) -> String {
        self.name.clone().unwrap_or_else(|| capitalize(&self.id))
    }
}

/// Capitalize first letter.
fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().to_string() + chars.as_str(),
    }
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

/// Result of scanning an OpenClaw extensions directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredExtension {
    /// Parsed manifest.
    pub manifest: OpenClawManifest,
    /// Absolute path to the extension directory.
    pub path: PathBuf,
    /// Whether `index.ts` exists.
    pub has_entry_point: bool,
    /// Whether `src/` directory exists.
    pub has_src: bool,
}

/// Scan an OpenClaw extensions directory and return discovered extensions.
pub fn discover_openclaw_extensions(
    base_dir: &Path,
) -> Result<Vec<DiscoveredExtension>, ExtensionError> {
    if !base_dir.exists() {
        return Err(ExtensionError::ImportFailed(format!(
            "OpenClaw extensions directory not found: {}",
            base_dir.display()
        )));
    }

    let mut results = Vec::new();
    let entries = std::fs::read_dir(base_dir).map_err(|e| {
        ExtensionError::ImportFailed(format!("cannot read {}: {}", base_dir.display(), e))
    })?;

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let manifest_path = path.join("openclaw.plugin.json");
        if !manifest_path.exists() {
            continue;
        }

        match OpenClawManifest::from_file(&manifest_path) {
            Ok(manifest) => {
                let has_entry_point = path.join("index.ts").exists();
                let has_src = path.join("src").is_dir();

                if !has_entry_point {
                    warn!(
                        "OpenClaw extension '{}' has manifest but no index.ts",
                        manifest.id
                    );
                }

                results.push(DiscoveredExtension {
                    manifest,
                    path,
                    has_entry_point,
                    has_src,
                });
            }
            Err(e) => {
                warn!("Skipping {}: {}", path.display(), e);
            }
        }
    }

    results.sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
    info!(
        "Discovered {} OpenClaw extensions in {}",
        results.len(),
        base_dir.display()
    );
    Ok(results)
}

// ---------------------------------------------------------------------------
// Permissions mapping
// ---------------------------------------------------------------------------

/// Map an OpenClaw channel type to reasonable default Deno permissions.
pub fn permissions_for_channel(channel: &str) -> ExtensionPermissions {
    match channel {
        "telegram" => ExtensionPermissions {
            allow_net: vec!["api.telegram.org".to_string()],
            allow_read: vec!["/tmp".to_string()],
            allow_write: vec!["/tmp".to_string()],
            allow_env: vec![
                "TELEGRAM_BOT_TOKEN".to_string(),
                "TELEGRAM_API_ID".to_string(),
                "TELEGRAM_API_HASH".to_string(),
            ],
        },
        "discord" => ExtensionPermissions {
            allow_net: vec!["discord.com".to_string(), "gateway.discord.gg".to_string()],
            allow_read: vec!["/tmp".to_string()],
            allow_write: vec!["/tmp".to_string()],
            allow_env: vec!["DISCORD_TOKEN".to_string(), "DISCORD_BOT_TOKEN".to_string()],
        },
        "slack" => ExtensionPermissions {
            allow_net: vec![
                "slack.com".to_string(),
                "api.slack.com".to_string(),
                "wss-primary.slack.com".to_string(),
            ],
            allow_read: vec!["/tmp".to_string()],
            allow_write: vec!["/tmp".to_string()],
            allow_env: vec!["SLACK_BOT_TOKEN".to_string(), "SLACK_APP_TOKEN".to_string()],
        },
        "whatsapp" => ExtensionPermissions {
            allow_net: vec!["graph.facebook.com".to_string()],
            allow_read: vec!["/tmp".to_string()],
            allow_write: vec!["/tmp".to_string()],
            allow_env: vec![
                "WHATSAPP_TOKEN".to_string(),
                "WHATSAPP_PHONE_ID".to_string(),
            ],
        },
        "signal" => ExtensionPermissions {
            allow_net: vec!["localhost".to_string(), "127.0.0.1".to_string()],
            allow_read: vec!["/tmp".to_string()],
            allow_write: vec!["/tmp".to_string()],
            allow_env: vec!["SIGNAL_CLI_PATH".to_string()],
        },
        "imessage" | "bluebubbles" => ExtensionPermissions {
            allow_net: vec!["localhost".to_string()],
            allow_read: vec!["/tmp".to_string()],
            allow_write: vec!["/tmp".to_string()],
            allow_env: vec![
                "BLUEBUBBLES_URL".to_string(),
                "BLUEBUBBLES_PASSWORD".to_string(),
            ],
        },
        "matrix" => ExtensionPermissions {
            allow_net: vec!["*".to_string()], // Matrix homeservers vary
            allow_read: vec!["/tmp".to_string()],
            allow_write: vec!["/tmp".to_string()],
            allow_env: vec![
                "MATRIX_HOMESERVER".to_string(),
                "MATRIX_USER".to_string(),
                "MATRIX_PASSWORD".to_string(),
            ],
        },
        // Default: conservative permissions
        _ => ExtensionPermissions {
            allow_net: vec![],
            allow_read: vec!["/tmp".to_string()],
            allow_write: vec!["/tmp".to_string()],
            allow_env: vec![],
        },
    }
}

// ---------------------------------------------------------------------------
// Import helpers
// ---------------------------------------------------------------------------

/// Channel message that can be sent/received through the bridge.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeMessage {
    /// Channel type (e.g. "telegram").
    pub channel: String,
    /// Target identifier (chat ID, user ID, etc.).
    pub target: String,
    /// Message text.
    pub text: String,
    /// Optional media URLs.
    #[serde(default)]
    pub media: Vec<String>,
    /// Optional reply-to message ID.
    #[serde(default)]
    pub reply_to: Option<String>,
    /// Optional thread ID.
    #[serde(default)]
    pub thread_id: Option<String>,
}

/// Channel event received from the bridge.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum BridgeEvent {
    /// Incoming message from a channel.
    #[serde(rename = "message")]
    Message {
        channel: String,
        from: String,
        text: String,
        #[serde(default)]
        media: Vec<String>,
        #[serde(default)]
        message_id: Option<String>,
        #[serde(default)]
        thread_id: Option<String>,
    },
    /// Extension status change.
    #[serde(rename = "status")]
    Status {
        channel: String,
        connected: bool,
        #[serde(default)]
        error: Option<String>,
    },
    /// Extension log message.
    #[serde(rename = "log")]
    Log { level: String, message: String },
}

// ---------------------------------------------------------------------------
// Registry integration
// ---------------------------------------------------------------------------

impl ExtensionRegistry {
    /// Discover and import all valid OpenClaw extensions from the configured base.
    pub async fn import_all_openclaw(&self) -> Result<Vec<Extension>, ExtensionError> {
        let discovered = discover_openclaw_extensions(&self.openclaw_base)?;
        let mut imported = Vec::new();

        for disc in discovered {
            if !disc.has_entry_point {
                warn!("Skipping '{}': no index.ts entry point", disc.manifest.id);
                continue;
            }

            // Determine permissions from channel types
            let permissions = if let Some(ch) = disc.manifest.channels.first() {
                permissions_for_channel(ch)
            } else {
                ExtensionPermissions::default()
            };

            let mut ext = Extension::new(
                disc.manifest.display_name(),
                ExtensionSource::OpenClaw(disc.manifest.id.clone()),
            );
            ext.permissions = permissions;
            if let Some(v) = &disc.manifest.version {
                ext.version = v.clone();
            }

            match self.register(ext).await {
                Ok(registered) => {
                    info!(
                        "Imported OpenClaw extension '{}' (channels: {:?})",
                        disc.manifest.id, disc.manifest.channels
                    );
                    imported.push(registered);
                }
                Err(ExtensionError::AlreadyExists(_)) => {
                    // Already registered, skip silently
                }
                Err(e) => {
                    warn!("Failed to import '{}': {}", disc.manifest.id, e);
                }
            }
        }

        info!("Imported {} OpenClaw extensions", imported.len());
        Ok(imported)
    }

    /// Send a message through an OpenClaw extension's outbound adapter.
    pub async fn send_channel_message(
        &self,
        extension_id: &str,
        message: BridgeMessage,
    ) -> Result<serde_json::Value, ExtensionError> {
        self.send_rpc(
            extension_id,
            "channel.send",
            serde_json::to_value(&message)
                .map_err(|e| ExtensionError::JsonRpcError(e.to_string()))?,
        )
        .await
    }

    /// Poll for incoming messages from an OpenClaw extension.
    pub async fn poll_channel_messages(
        &self,
        extension_id: &str,
    ) -> Result<Vec<BridgeEvent>, ExtensionError> {
        let result = self
            .send_rpc(extension_id, "channel.poll", serde_json::json!({}))
            .await?;

        serde_json::from_value(result)
            .map_err(|e| ExtensionError::JsonRpcError(format!("invalid poll response: {}", e)))
    }

    /// Get channel capabilities from an OpenClaw extension.
    pub async fn channel_capabilities(
        &self,
        extension_id: &str,
    ) -> Result<serde_json::Value, ExtensionError> {
        self.send_rpc(extension_id, "channel.capabilities", serde_json::json!({}))
            .await
    }

    /// Get channel status from an OpenClaw extension.
    pub async fn channel_status(
        &self,
        extension_id: &str,
    ) -> Result<serde_json::Value, ExtensionError> {
        self.send_rpc(extension_id, "channel.status", serde_json::json!({}))
            .await
    }
}

// ---------------------------------------------------------------------------
// Channel Extension Adapter
// ---------------------------------------------------------------------------

/// Wraps an OpenClaw channel extension into a high-level send/receive interface.
///
/// Manages the extension lifecycle: start the Deno bridge, send messages
/// via JSON-RPC `channel.send`, poll for inbound via `channel.poll`.
pub struct ChannelExtensionAdapter {
    extension_id: String,
    channel_type: String,
}

impl ChannelExtensionAdapter {
    /// Create a new adapter for the given extension.
    pub fn new(extension_id: impl Into<String>, channel_type: impl Into<String>) -> Self {
        Self {
            extension_id: extension_id.into(),
            channel_type: channel_type.into(),
        }
    }

    /// Extension ID.
    pub fn extension_id(&self) -> &str {
        &self.extension_id
    }

    /// Channel type (e.g. "whatsapp", "signal", "imessage").
    pub fn channel_type(&self) -> &str {
        &self.channel_type
    }

    /// Send a message through the extension bridge.
    pub async fn send(
        &self,
        registry: &ExtensionRegistry,
        target: &str,
        text: &str,
    ) -> Result<serde_json::Value, ExtensionError> {
        let msg = BridgeMessage {
            channel: self.channel_type.clone(),
            target: target.to_string(),
            text: text.to_string(),
            media: vec![],
            reply_to: None,
            thread_id: None,
        };
        registry.send_channel_message(&self.extension_id, msg).await
    }

    /// Send a message with media attachments.
    pub async fn send_with_media(
        &self,
        registry: &ExtensionRegistry,
        target: &str,
        text: &str,
        media: Vec<String>,
    ) -> Result<serde_json::Value, ExtensionError> {
        let msg = BridgeMessage {
            channel: self.channel_type.clone(),
            target: target.to_string(),
            text: text.to_string(),
            media,
            reply_to: None,
            thread_id: None,
        };
        registry.send_channel_message(&self.extension_id, msg).await
    }

    /// Reply to a specific message.
    pub async fn reply(
        &self,
        registry: &ExtensionRegistry,
        target: &str,
        text: &str,
        reply_to: &str,
    ) -> Result<serde_json::Value, ExtensionError> {
        let msg = BridgeMessage {
            channel: self.channel_type.clone(),
            target: target.to_string(),
            text: text.to_string(),
            media: vec![],
            reply_to: Some(reply_to.to_string()),
            thread_id: None,
        };
        registry.send_channel_message(&self.extension_id, msg).await
    }

    /// Poll for incoming messages.
    pub async fn poll(
        &self,
        registry: &ExtensionRegistry,
    ) -> Result<Vec<BridgeEvent>, ExtensionError> {
        registry.poll_channel_messages(&self.extension_id).await
    }

    /// Check channel connectivity status.
    pub async fn status(
        &self,
        registry: &ExtensionRegistry,
    ) -> Result<serde_json::Value, ExtensionError> {
        registry.channel_status(&self.extension_id).await
    }

    /// Get channel capabilities.
    pub async fn capabilities(
        &self,
        registry: &ExtensionRegistry,
    ) -> Result<serde_json::Value, ExtensionError> {
        registry.channel_capabilities(&self.extension_id).await
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn make_temp_dir() -> TempDir {
        tempfile::tempdir().expect("should create temp dir")
    }

    // -- Manifest tests -----------------------------------------------------

    #[test]
    fn test_manifest_parse_minimal() {
        let json = r#"{"id": "test", "channels": ["test"]}"#;
        let manifest: OpenClawManifest =
            serde_json::from_str(json).expect("should parse successfully");
        assert_eq!(manifest.id, "test");
        assert_eq!(manifest.channels, vec!["test"]);
        assert!(manifest.config_schema.is_null());
    }

    #[test]
    fn test_manifest_parse_full() {
        let json = r#"{
            "id": "telegram",
            "name": "Telegram Plugin",
            "description": "Telegram channel adapter",
            "version": "1.2.0",
            "channels": ["telegram"],
            "configSchema": {"type": "object", "properties": {}}
        }"#;
        let manifest: OpenClawManifest =
            serde_json::from_str(json).expect("should parse successfully");
        assert_eq!(manifest.id, "telegram");
        assert_eq!(manifest.name.as_deref(), Some("Telegram Plugin"));
        assert_eq!(manifest.version.as_deref(), Some("1.2.0"));
        assert_eq!(manifest.display_name(), "Telegram Plugin");
    }

    #[test]
    fn test_manifest_display_name_fallback() {
        let json = r#"{"id": "discord", "channels": ["discord"]}"#;
        let manifest: OpenClawManifest =
            serde_json::from_str(json).expect("should parse successfully");
        assert_eq!(manifest.display_name(), "Discord");
    }

    #[test]
    fn test_manifest_from_file() {
        let dir = make_temp_dir();
        let path = dir.path().join("openclaw.plugin.json");
        let mut f = std::fs::File::create(&path).expect("should create file");
        write!(f, r#"{{"id":"test","channels":["test"]}}"#).expect("operation should succeed");

        let manifest = OpenClawManifest::from_file(&path).expect("operation should succeed");
        assert_eq!(manifest.id, "test");
    }

    #[test]
    fn test_manifest_from_file_missing() {
        let err = OpenClawManifest::from_file(Path::new("/nonexistent/manifest.json")).unwrap_err();
        assert!(matches!(err, ExtensionError::ImportFailed(_)));
    }

    #[test]
    fn test_manifest_from_file_invalid_json() {
        let dir = make_temp_dir();
        let path = dir.path().join("bad.json");
        std::fs::write(&path, "not json {").expect("should write file");

        let err = OpenClawManifest::from_file(&path).unwrap_err();
        assert!(matches!(err, ExtensionError::InvalidExtension(_)));
    }

    // -- Discovery tests ----------------------------------------------------

    #[test]
    fn test_discover_empty_dir() {
        let dir = make_temp_dir();
        let results = discover_openclaw_extensions(dir.path()).expect("operation should succeed");
        assert!(results.is_empty());
    }

    #[test]
    fn test_discover_missing_dir() {
        let err = discover_openclaw_extensions(Path::new("/nonexistent/path")).unwrap_err();
        assert!(matches!(err, ExtensionError::ImportFailed(_)));
    }

    #[test]
    fn test_discover_with_extensions() {
        let dir = make_temp_dir();

        // Create a valid extension
        let ext_dir = dir.path().join("telegram");
        std::fs::create_dir(&ext_dir).expect("should create directory");
        std::fs::write(
            ext_dir.join("openclaw.plugin.json"),
            r#"{"id":"telegram","channels":["telegram"]}"#,
        )
        .expect("operation should succeed");
        std::fs::write(ext_dir.join("index.ts"), "export default {};").expect("should write file");
        std::fs::create_dir(ext_dir.join("src")).expect("should create directory");

        // Create extension without index.ts
        let ext_dir2 = dir.path().join("broken");
        std::fs::create_dir(&ext_dir2).expect("should create directory");
        std::fs::write(
            ext_dir2.join("openclaw.plugin.json"),
            r#"{"id":"broken","channels":["test"]}"#,
        )
        .expect("operation should succeed");

        // Create a non-extension directory
        std::fs::create_dir(dir.path().join("not-an-ext")).expect("should create directory");

        let results = discover_openclaw_extensions(dir.path()).expect("operation should succeed");
        assert_eq!(results.len(), 2);

        let telegram = results
            .iter()
            .find(|r| r.manifest.id == "telegram")
            .expect("find should succeed");
        assert!(telegram.has_entry_point);
        assert!(telegram.has_src);

        let broken = results
            .iter()
            .find(|r| r.manifest.id == "broken")
            .expect("find should succeed");
        assert!(!broken.has_entry_point);
        assert!(!broken.has_src);
    }

    #[test]
    fn test_discover_sorted_by_id() {
        let dir = make_temp_dir();
        for name in &["zebra", "alpha", "middle"] {
            let ext_dir = dir.path().join(name);
            std::fs::create_dir(&ext_dir).expect("should create directory");
            std::fs::write(
                ext_dir.join("openclaw.plugin.json"),
                format!(r#"{{"id":"{}","channels":[]}}"#, name),
            )
            .expect("operation should succeed");
            std::fs::write(ext_dir.join("index.ts"), "").expect("should write file");
        }

        let results = discover_openclaw_extensions(dir.path()).expect("operation should succeed");
        let ids: Vec<&str> = results.iter().map(|r| r.manifest.id.as_str()).collect();
        assert_eq!(ids, vec!["alpha", "middle", "zebra"]);
    }

    // -- Permissions tests --------------------------------------------------

    #[test]
    fn test_permissions_telegram() {
        let perms = permissions_for_channel("telegram");
        assert!(perms.allow_net.contains(&"api.telegram.org".to_string()));
        assert!(perms.allow_env.contains(&"TELEGRAM_BOT_TOKEN".to_string()));
    }

    #[test]
    fn test_permissions_discord() {
        let perms = permissions_for_channel("discord");
        assert!(perms.allow_net.contains(&"discord.com".to_string()));
        assert!(perms.allow_env.contains(&"DISCORD_TOKEN".to_string()));
    }

    #[test]
    fn test_permissions_slack() {
        let perms = permissions_for_channel("slack");
        assert!(perms.allow_net.contains(&"slack.com".to_string()));
    }

    #[test]
    fn test_permissions_whatsapp() {
        let perms = permissions_for_channel("whatsapp");
        assert!(perms.allow_net.contains(&"graph.facebook.com".to_string()));
    }

    #[test]
    fn test_permissions_signal() {
        let perms = permissions_for_channel("signal");
        assert!(perms.allow_net.contains(&"localhost".to_string()));
    }

    #[test]
    fn test_permissions_matrix() {
        let perms = permissions_for_channel("matrix");
        assert!(perms.allow_net.contains(&"*".to_string()));
    }

    #[test]
    fn test_permissions_unknown() {
        let perms = permissions_for_channel("unknown_channel");
        assert!(perms.allow_net.is_empty());
        assert!(perms.allow_env.is_empty());
    }

    // -- BridgeMessage tests ------------------------------------------------

    #[test]
    fn test_bridge_message_serialize() {
        let msg = BridgeMessage {
            channel: "telegram".to_string(),
            target: "12345".to_string(),
            text: "Hello!".to_string(),
            media: vec![],
            reply_to: None,
            thread_id: None,
        };
        let json = serde_json::to_string(&msg).expect("should serialize to JSON");
        assert!(json.contains("telegram"));
        assert!(json.contains("Hello!"));
    }

    #[test]
    fn test_bridge_message_with_media() {
        let msg = BridgeMessage {
            channel: "discord".to_string(),
            target: "#general".to_string(),
            text: "Check this out".to_string(),
            media: vec!["https://example.com/image.png".to_string()],
            reply_to: Some("msg-123".to_string()),
            thread_id: Some("thread-456".to_string()),
        };
        let json = serde_json::to_value(&msg).expect("should serialize to JSON");
        assert_eq!(
            json["media"].as_array().expect("should be an array").len(),
            1
        );
        assert_eq!(json["reply_to"], "msg-123");
    }

    // -- BridgeEvent tests --------------------------------------------------

    #[test]
    fn test_bridge_event_message() {
        let json = r#"{
            "type": "message",
            "channel": "telegram",
            "from": "user123",
            "text": "Hello Zeus"
        }"#;
        let event: BridgeEvent = serde_json::from_str(json).expect("should parse successfully");
        match event {
            BridgeEvent::Message {
                channel,
                from,
                text,
                ..
            } => {
                assert_eq!(channel, "telegram");
                assert_eq!(from, "user123");
                assert_eq!(text, "Hello Zeus");
            }
            _ => panic!("expected Message event"),
        }
    }

    #[test]
    fn test_bridge_event_status() {
        let json = r#"{
            "type": "status",
            "channel": "discord",
            "connected": true
        }"#;
        let event: BridgeEvent = serde_json::from_str(json).expect("should parse successfully");
        match event {
            BridgeEvent::Status {
                channel,
                connected,
                error,
            } => {
                assert_eq!(channel, "discord");
                assert!(connected);
                assert!(error.is_none());
            }
            _ => panic!("expected Status event"),
        }
    }

    #[test]
    fn test_bridge_event_log() {
        let json = r#"{"type": "log", "level": "info", "message": "connected"}"#;
        let event: BridgeEvent = serde_json::from_str(json).expect("should parse successfully");
        match event {
            BridgeEvent::Log { level, message } => {
                assert_eq!(level, "info");
                assert_eq!(message, "connected");
            }
            _ => panic!("expected Log event"),
        }
    }

    // -- capitalize helper --------------------------------------------------

    // -- ChannelExtensionAdapter tests ----------------------------------------

    #[test]
    fn test_channel_adapter_new() {
        let adapter = ChannelExtensionAdapter::new("ext-123", "whatsapp");
        assert_eq!(adapter.extension_id(), "ext-123");
        assert_eq!(adapter.channel_type(), "whatsapp");
    }

    #[test]
    fn test_channel_adapter_signal() {
        let adapter = ChannelExtensionAdapter::new("ext-456", "signal");
        assert_eq!(adapter.channel_type(), "signal");
    }

    #[test]
    fn test_channel_adapter_imessage() {
        let adapter = ChannelExtensionAdapter::new("ext-789", "imessage");
        assert_eq!(adapter.channel_type(), "imessage");
    }

    // -- capitalize helper --------------------------------------------------

    #[test]
    fn test_capitalize() {
        assert_eq!(capitalize("telegram"), "Telegram");
        assert_eq!(capitalize("discord"), "Discord");
        assert_eq!(capitalize(""), "");
        assert_eq!(capitalize("a"), "A");
    }

    // -- Registry integration (async) ---------------------------------------

    #[tokio::test]
    async fn test_import_all_openclaw_empty() {
        let dir = make_temp_dir();
        let reg = ExtensionRegistry::new().with_openclaw_base(dir.path().to_path_buf());
        let imported = reg
            .import_all_openclaw()
            .await
            .expect("async operation should succeed");
        assert!(imported.is_empty());
    }

    #[tokio::test]
    async fn test_import_all_openclaw_with_extensions() {
        let dir = make_temp_dir();

        // Create telegram extension
        let tg = dir.path().join("telegram");
        std::fs::create_dir(&tg).expect("should create directory");
        std::fs::write(
            tg.join("openclaw.plugin.json"),
            r#"{"id":"telegram","channels":["telegram"]}"#,
        )
        .expect("operation should succeed");
        std::fs::write(tg.join("index.ts"), "export default {};").expect("should write file");

        // Create discord extension
        let dc = dir.path().join("discord");
        std::fs::create_dir(&dc).expect("should create directory");
        std::fs::write(
            dc.join("openclaw.plugin.json"),
            r#"{"id":"discord","channels":["discord"]}"#,
        )
        .expect("operation should succeed");
        std::fs::write(dc.join("index.ts"), "export default {};").expect("should write file");

        let reg = ExtensionRegistry::new().with_openclaw_base(dir.path().to_path_buf());
        let imported = reg
            .import_all_openclaw()
            .await
            .expect("async operation should succeed");
        assert_eq!(imported.len(), 2);
        assert_eq!(reg.count().await, 2);
    }

    #[tokio::test]
    async fn test_import_all_openclaw_skips_no_entry() {
        let dir = make_temp_dir();

        let ext = dir.path().join("broken");
        std::fs::create_dir(&ext).expect("should create directory");
        std::fs::write(
            ext.join("openclaw.plugin.json"),
            r#"{"id":"broken","channels":["test"]}"#,
        )
        .expect("operation should succeed");
        // No index.ts

        let reg = ExtensionRegistry::new().with_openclaw_base(dir.path().to_path_buf());
        let imported = reg
            .import_all_openclaw()
            .await
            .expect("async operation should succeed");
        assert!(imported.is_empty());
    }

    #[test]
    fn test_permissions_imessage_bluebubbles() {
        let perms = permissions_for_channel("imessage");
        assert!(perms.allow_net.contains(&"localhost".to_string()));
        assert!(perms.allow_env.contains(&"BLUEBUBBLES_URL".to_string()));

        // "bluebubbles" alias should map the same way
        let perms2 = permissions_for_channel("bluebubbles");
        assert_eq!(perms.allow_net, perms2.allow_net);
    }

    #[tokio::test]
    async fn test_import_whatsapp_signal_imessage() {
        let dir = make_temp_dir();

        // Create WhatsApp extension
        let wa = dir.path().join("whatsapp");
        std::fs::create_dir(&wa).expect("should create directory");
        std::fs::write(
            wa.join("openclaw.plugin.json"),
            r#"{"id":"whatsapp","name":"WhatsApp Cloud API","channels":["whatsapp"],"version":"0.1.0"}"#,
        )
        .expect("operation should succeed");
        std::fs::write(wa.join("index.ts"), "export default { register(api: any) { api.registerChannel({ plugin: { id: 'whatsapp' } }); } };").expect("should write file");

        // Create Signal extension
        let sig = dir.path().join("signal");
        std::fs::create_dir(&sig).expect("should create directory");
        std::fs::write(
            sig.join("openclaw.plugin.json"),
            r#"{"id":"signal","name":"Signal Messenger","channels":["signal"],"version":"0.1.0"}"#,
        )
        .expect("operation should succeed");
        std::fs::write(sig.join("index.ts"), "export default { register(api: any) { api.registerChannel({ plugin: { id: 'signal' } }); } };").expect("should write file");

        // Create iMessage extension
        let im = dir.path().join("imessage");
        std::fs::create_dir(&im).expect("should create directory");
        std::fs::write(
            im.join("openclaw.plugin.json"),
            r#"{"id":"imessage","name":"iMessage (BlueBubbles)","channels":["imessage"],"version":"0.1.0"}"#,
        )
        .expect("operation should succeed");
        std::fs::write(im.join("index.ts"), "export default { register(api: any) { api.registerChannel({ plugin: { id: 'imessage' } }); } };").expect("should write file");

        let reg = ExtensionRegistry::new().with_openclaw_base(dir.path().to_path_buf());

        // Discover
        let discovered =
            discover_openclaw_extensions(dir.path()).expect("operation should succeed");
        assert_eq!(discovered.len(), 3);
        let ids: Vec<&str> = discovered.iter().map(|d| d.manifest.id.as_str()).collect();
        assert!(ids.contains(&"whatsapp"));
        assert!(ids.contains(&"signal"));
        assert!(ids.contains(&"imessage"));

        // Import all
        let imported = reg
            .import_all_openclaw()
            .await
            .expect("async operation should succeed");
        assert_eq!(imported.len(), 3);
        assert_eq!(reg.count().await, 3);

        // Verify permissions were applied
        let list = reg.list().await;
        let wa_ext = list
            .iter()
            .find(|e| e.name == "WhatsApp Cloud API")
            .expect("find should succeed");
        assert!(
            wa_ext
                .permissions
                .allow_net
                .contains(&"graph.facebook.com".to_string())
        );

        let sig_ext = list
            .iter()
            .find(|e| e.name == "Signal Messenger")
            .expect("find should succeed");
        assert!(
            sig_ext
                .permissions
                .allow_net
                .contains(&"localhost".to_string())
        );

        let im_ext = list
            .iter()
            .find(|e| e.name == "iMessage (BlueBubbles)")
            .expect("operation should succeed");
        assert!(
            im_ext
                .permissions
                .allow_net
                .contains(&"localhost".to_string())
        );
    }

    #[test]
    fn test_bridge_message_whatsapp() {
        let msg = BridgeMessage {
            channel: "whatsapp".to_string(),
            target: "+1234567890".to_string(),
            text: "Hello from Zeus!".to_string(),
            media: vec![],
            reply_to: None,
            thread_id: None,
        };
        let json = serde_json::to_value(&msg).expect("should serialize to JSON");
        assert_eq!(json["channel"], "whatsapp");
        assert_eq!(json["target"], "+1234567890");
    }

    #[test]
    fn test_bridge_message_signal_with_reply() {
        let msg = BridgeMessage {
            channel: "signal".to_string(),
            target: "+9876543210".to_string(),
            text: "Replying via Signal".to_string(),
            media: vec![],
            reply_to: Some("1707123456789".to_string()),
            thread_id: None,
        };
        let json = serde_json::to_value(&msg).expect("should serialize to JSON");
        assert_eq!(json["reply_to"], "1707123456789");
    }

    #[test]
    fn test_bridge_message_imessage_with_media() {
        let msg = BridgeMessage {
            channel: "imessage".to_string(),
            target: "iMessage;-;+1234567890".to_string(),
            text: "Check this out".to_string(),
            media: vec!["https://example.com/photo.jpg".to_string()],
            reply_to: None,
            thread_id: Some("chat-guid-abc".to_string()),
        };
        let json = serde_json::to_value(&msg).expect("should serialize to JSON");
        assert_eq!(json["thread_id"], "chat-guid-abc");
        assert_eq!(
            json["media"].as_array().expect("should be an array").len(),
            1
        );
    }

    #[test]
    fn test_bridge_event_whatsapp_inbound() {
        let json = r#"{
            "type": "message",
            "channel": "whatsapp",
            "from": "+1234567890",
            "text": "Hello Zeus",
            "message_id": "wamid.abc123"
        }"#;
        let event: BridgeEvent = serde_json::from_str(json).expect("should parse successfully");
        match event {
            BridgeEvent::Message {
                channel,
                from,
                text,
                message_id,
                ..
            } => {
                assert_eq!(channel, "whatsapp");
                assert_eq!(from, "+1234567890");
                assert_eq!(text, "Hello Zeus");
                assert_eq!(message_id.as_deref(), Some("wamid.abc123"));
            }
            _ => panic!("expected Message event"),
        }
    }

    #[test]
    fn test_bridge_event_signal_status() {
        let json = r#"{
            "type": "status",
            "channel": "signal",
            "connected": false,
            "error": "signal-cli not running"
        }"#;
        let event: BridgeEvent = serde_json::from_str(json).expect("should parse successfully");
        match event {
            BridgeEvent::Status {
                channel,
                connected,
                error,
            } => {
                assert_eq!(channel, "signal");
                assert!(!connected);
                assert_eq!(error.as_deref(), Some("signal-cli not running"));
            }
            _ => panic!("expected Status event"),
        }
    }

    #[tokio::test]
    async fn test_import_all_openclaw_no_duplicates() {
        let dir = make_temp_dir();

        let tg = dir.path().join("telegram");
        std::fs::create_dir(&tg).expect("should create directory");
        std::fs::write(
            tg.join("openclaw.plugin.json"),
            r#"{"id":"telegram","channels":["telegram"]}"#,
        )
        .expect("operation should succeed");
        std::fs::write(tg.join("index.ts"), "").expect("should write file");

        let reg = ExtensionRegistry::new().with_openclaw_base(dir.path().to_path_buf());

        // Import twice
        let first = reg
            .import_all_openclaw()
            .await
            .expect("async operation should succeed");
        let second = reg
            .import_all_openclaw()
            .await
            .expect("async operation should succeed");

        assert_eq!(first.len(), 1);
        assert!(second.is_empty()); // No new imports
        assert_eq!(reg.count().await, 1);
    }
}
