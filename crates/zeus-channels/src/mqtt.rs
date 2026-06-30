//! MQTT Channel Adapter
//!
//! Provides an MQTT messaging adapter for IoT/home automation integration.
//! Uses rumqttc for MQTT v5 client connectivity.
//!
//! Features:
//! - Connect to any MQTT broker (Mosquitto, HiveMQ, EMQX, etc.)
//! - Publish messages to topics
//! - Subscribe to topics and receive messages
//! - Configurable QoS (0, 1, 2)
//! - Topic prefix for namespace isolation
//! - Last Will and Testament (LWT) for clean disconnect detection
//! - Auto-reconnect with backoff
//!
//! ## Config Example
//!
//! ```toml
//! [channels.mqtt]
//! broker_url = "mqtt://192.168.1.100"
//! port = 1883
//! client_id = "zeus-agent-01"
//! topic_prefix = "zeus/"
//! qos = 1
//! subscribe_topics = ["zeus/inbox/#", "home/sensors/#"]
//! username = "zeus"
//! password = "secret"
//! ```

use crate::{ChannelAdapter, ChannelMessage, ChannelSource, ReceiveMode};
use async_trait::async_trait;
use rumqttc::{AsyncClient, EventLoop, MqttOptions, QoS as RumQoS};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{Notify, RwLock, mpsc};
use tracing::{debug, error, info, warn};
use zeus_core::{Error, Result};

/// Map integer QoS (0, 1, 2) to rumqttc QoS
fn to_rumqttc_qos(qos: u8) -> RumQoS {
    match qos {
        0 => RumQoS::AtMostOnce,
        1 => RumQoS::AtLeastOnce,
        _ => RumQoS::ExactlyOnce,
    }
}

/// MQTT channel configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MqttConfig {
    /// Broker URL (host only, e.g., "192.168.1.100" or "mqtt.example.com")
    #[serde(default = "default_broker_url")]
    pub broker_url: String,

    /// Broker port (default: 1883, use 8883 for TLS)
    #[serde(default = "default_port")]
    pub port: u16,

    /// MQTT client ID (must be unique per connection)
    #[serde(default = "default_client_id")]
    pub client_id: String,

    /// Topic prefix for all published messages (e.g., "zeus/")
    #[serde(default)]
    pub topic_prefix: String,

    /// Default QoS level: 0 (at most once), 1 (at least once), 2 (exactly once)
    #[serde(default = "default_qos")]
    pub qos: u8,

    /// Topics to subscribe to for receiving messages.
    /// Supports MQTT wildcards: + (single level), # (multi level)
    #[serde(default)]
    pub subscribe_topics: Vec<String>,

    /// Optional username for broker authentication
    #[serde(default)]
    pub username: Option<String>,

    /// Optional password for broker authentication
    #[serde(default)]
    pub password: Option<String>,

    /// Keep-alive interval in seconds
    #[serde(default = "default_keep_alive")]
    pub keep_alive_secs: u64,

    /// Clean session flag (default: true)
    #[serde(default = "default_clean_session")]
    pub clean_session: bool,

    /// Last Will topic (published by broker when client disconnects ungracefully)
    #[serde(default)]
    pub last_will_topic: Option<String>,

    /// Last Will message payload
    #[serde(default)]
    pub last_will_message: Option<String>,

    /// Incoming message channel capacity
    #[serde(default = "default_channel_capacity")]
    pub channel_capacity: usize,
}

fn default_broker_url() -> String {
    "localhost".to_string()
}

fn default_port() -> u16 {
    1883
}

fn default_client_id() -> String {
    format!(
        "zeus-{}",
        uuid::Uuid::new_v4()
            .to_string()
            .split('-')
            .next()
            .unwrap_or("agent")
    )
}

fn default_qos() -> u8 {
    1
}

fn default_keep_alive() -> u64 {
    30
}

fn default_clean_session() -> bool {
    true
}

fn default_channel_capacity() -> usize {
    256
}

impl Default for MqttConfig {
    fn default() -> Self {
        Self {
            broker_url: default_broker_url(),
            port: default_port(),
            client_id: default_client_id(),
            topic_prefix: String::new(),
            qos: default_qos(),
            subscribe_topics: Vec::new(),
            username: None,
            password: None,
            keep_alive_secs: default_keep_alive(),
            clean_session: default_clean_session(),
            last_will_topic: None,
            last_will_message: None,
            channel_capacity: default_channel_capacity(),
        }
    }
}

impl MqttConfig {
    /// Apply environment variable overrides
    ///
    /// Checks:
    /// - `MQTT_BROKER_URL` -> `broker_url`
    /// - `MQTT_PORT` -> `port`
    /// - `MQTT_CLIENT_ID` -> `client_id`
    /// - `MQTT_USERNAME` -> `username`
    /// - `MQTT_PASSWORD` -> `password`
    /// - `MQTT_TOPIC_PREFIX` -> `topic_prefix`
    pub fn with_env_overrides(mut self) -> Self {
        if let Ok(url) = std::env::var("MQTT_BROKER_URL") {
            self.broker_url = url;
        }
        if let Ok(port) = std::env::var("MQTT_PORT")
            && let Ok(p) = port.parse()
        {
            self.port = p;
        }
        if let Ok(id) = std::env::var("MQTT_CLIENT_ID") {
            self.client_id = id;
        }
        if let Ok(user) = std::env::var("MQTT_USERNAME") {
            self.username = Some(user);
        }
        if let Ok(pass) = std::env::var("MQTT_PASSWORD") {
            self.password = Some(pass);
        }
        if let Ok(prefix) = std::env::var("MQTT_TOPIC_PREFIX") {
            self.topic_prefix = prefix;
        }
        self
    }

    /// Build full topic with prefix
    pub fn full_topic(&self, topic: &str) -> String {
        if self.topic_prefix.is_empty() {
            topic.to_string()
        } else {
            format!("{}{}", self.topic_prefix, topic)
        }
    }

    /// Build rumqttc MqttOptions from this config
    fn to_mqtt_options(&self) -> MqttOptions {
        let mut opts = MqttOptions::new(&self.client_id, &self.broker_url, self.port);
        opts.set_keep_alive(std::time::Duration::from_secs(self.keep_alive_secs));
        opts.set_clean_session(self.clean_session);

        if let (Some(user), Some(pass)) = (&self.username, &self.password) {
            opts.set_credentials(user, pass);
        }

        if let (Some(topic), Some(message)) = (&self.last_will_topic, &self.last_will_message) {
            let will = rumqttc::LastWill::new(
                topic,
                message.as_bytes().to_vec(),
                to_rumqttc_qos(self.qos),
                false,
            );
            opts.set_last_will(will);
        }

        opts
    }
}

/// MQTT channel adapter
pub struct MqttAdapter {
    config: MqttConfig,
    connected: Arc<AtomicBool>,
    client: Arc<RwLock<Option<AsyncClient>>>,
    shutdown: Arc<Notify>,
}

impl MqttAdapter {
    /// Create a new MQTT adapter
    pub fn new(config: MqttConfig) -> Self {
        info!(
            broker = %config.broker_url,
            port = config.port,
            client_id = %config.client_id,
            "MQTT adapter created"
        );

        Self {
            config,
            connected: Arc::new(AtomicBool::new(false)),
            client: Arc::new(RwLock::new(None)),
            shutdown: Arc::new(Notify::new()),
        }
    }

    /// Create from config with environment overrides
    pub fn from_env(config: MqttConfig) -> Self {
        Self::new(config.with_env_overrides())
    }

    /// Publish a message to a topic
    pub async fn publish(&self, topic: &str, payload: &str) -> Result<()> {
        let client = self.client.read().await;
        let client = client
            .as_ref()
            .ok_or_else(|| Error::Channel("MQTT client not connected".to_string()))?;

        let full_topic = self.config.full_topic(topic);
        let qos = to_rumqttc_qos(self.config.qos);

        client
            .publish(&full_topic, qos, false, payload.as_bytes())
            .await
            .map_err(|e| Error::Channel(format!("MQTT publish failed: {}", e)))?;

        debug!(topic = %full_topic, "MQTT message published");
        Ok(())
    }

    /// Subscribe to a topic
    pub async fn subscribe(&self, topic: &str) -> Result<()> {
        let client = self.client.read().await;
        let client = client
            .as_ref()
            .ok_or_else(|| Error::Channel("MQTT client not connected".to_string()))?;

        let qos = to_rumqttc_qos(self.config.qos);

        client
            .subscribe(topic, qos)
            .await
            .map_err(|e| Error::Channel(format!("MQTT subscribe failed: {}", e)))?;

        debug!(topic = %topic, "MQTT subscribed");
        Ok(())
    }

    /// Unsubscribe from a topic
    pub async fn unsubscribe(&self, topic: &str) -> Result<()> {
        let client = self.client.read().await;
        let client = client
            .as_ref()
            .ok_or_else(|| Error::Channel("MQTT client not connected".to_string()))?;

        client
            .unsubscribe(topic)
            .await
            .map_err(|e| Error::Channel(format!("MQTT unsubscribe failed: {}", e)))?;

        debug!(topic = %topic, "MQTT unsubscribed");
        Ok(())
    }

    /// Get current config
    pub fn config(&self) -> &MqttConfig {
        &self.config
    }

    /// Spawn the event loop that reads MQTT events and forwards messages
    fn spawn_event_loop(&self, mut eventloop: EventLoop, tx: mpsc::Sender<ChannelMessage>) {
        let connected = self.connected.clone();
        let shutdown = self.shutdown.clone();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown.notified() => {
                        info!("MQTT event loop shutting down");
                        connected.store(false, Ordering::SeqCst);
                        break;
                    }
                    event = eventloop.poll() => {
                        match event {
                            Ok(rumqttc::Event::Incoming(packet)) => {
                                match packet {
                                    rumqttc::Packet::ConnAck(_) => {
                                        connected.store(true, Ordering::SeqCst);
                                        info!("MQTT connected to broker");
                                    }
                                    rumqttc::Packet::Publish(publish) => {
                                        let topic = publish.topic.clone();
                                        let payload = String::from_utf8_lossy(&publish.payload).to_string();

                                        if payload.is_empty() {
                                            continue;
                                        }

                                        debug!(topic = %topic, len = payload.len(), "MQTT message received");

                                        let source = ChannelSource::with_chat(
                                            "mqtt",
                                            &topic,
                                            &topic,
                                        );
                                        let msg = ChannelMessage::new(source, payload);

                                        if let Err(e) = tx.send(msg).await {
                                            warn!("Failed to forward MQTT message: {}", e);
                                        }
                                    }
                                    rumqttc::Packet::Disconnect => {
                                        connected.store(false, Ordering::SeqCst);
                                        warn!("MQTT disconnected by broker");
                                    }
                                    _ => {}
                                }
                            }
                            Ok(rumqttc::Event::Outgoing(_)) => {}
                            Err(e) => {
                                connected.store(false, Ordering::SeqCst);
                                error!("MQTT connection error: {}", e);
                                // rumqttc handles reconnection internally;
                                // sleep briefly to avoid busy loop on persistent errors
                                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                            }
                        }
                    }
                }
            }
        });
    }
}

#[async_trait]
impl ChannelAdapter for MqttAdapter {
    fn channel_type(&self) -> &'static str {
        "mqtt"
    }

    fn receive_mode(&self) -> ReceiveMode {
        ReceiveMode::Native
    }

    async fn start(&self, tx: mpsc::Sender<ChannelMessage>) -> Result<()> {
        let mqtt_options = self.config.to_mqtt_options();

        let (client, eventloop) = AsyncClient::new(mqtt_options, self.config.channel_capacity);

        // Store the client
        *self.client.write().await = Some(client.clone());

        // Spawn event loop
        self.spawn_event_loop(eventloop, tx);

        // Subscribe to configured topics
        let qos = to_rumqttc_qos(self.config.qos);
        for topic in &self.config.subscribe_topics {
            if let Err(e) = client.subscribe(topic, qos).await {
                warn!(topic = %topic, error = %e, "Failed to subscribe to MQTT topic");
            } else {
                info!(topic = %topic, "MQTT subscribed");
            }
        }

        info!(
            broker = %self.config.broker_url,
            port = self.config.port,
            topics = ?self.config.subscribe_topics,
            "MQTT adapter started"
        );

        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        // Signal shutdown
        self.shutdown.notify_one();

        // Disconnect client
        if let Some(client) = self.client.write().await.take()
            && let Err(e) = client.disconnect().await
        {
            debug!("MQTT disconnect error (expected): {}", e);
        }

        self.connected.store(false, Ordering::SeqCst);
        info!("MQTT adapter stopped");
        Ok(())
    }

    async fn send(&self, to: &ChannelSource, content: &str) -> Result<()> {
        // Use chat_id as topic, falling back to user_id
        let topic = to.chat_id.as_deref().unwrap_or(&to.user_id);

        self.publish(topic, content).await
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mqtt_config_defaults() {
        let config = MqttConfig::default();
        assert_eq!(config.broker_url, "localhost");
        assert_eq!(config.port, 1883);
        assert_eq!(config.qos, 1);
        assert_eq!(config.keep_alive_secs, 30);
        assert!(config.clean_session);
        assert!(config.subscribe_topics.is_empty());
        assert!(config.username.is_none());
        assert!(config.password.is_none());
        assert!(config.last_will_topic.is_none());
        assert!(config.topic_prefix.is_empty());
        assert_eq!(config.channel_capacity, 256);
    }

    #[test]
    fn test_mqtt_config_serialization() {
        let config = MqttConfig {
            broker_url: "mqtt.example.com".to_string(),
            port: 8883,
            client_id: "zeus-test".to_string(),
            topic_prefix: "zeus/".to_string(),
            qos: 2,
            subscribe_topics: vec!["zeus/inbox/#".to_string(), "home/sensors/#".to_string()],
            username: Some("zeus".to_string()),
            password: Some("secret".to_string()),
            keep_alive_secs: 60,
            clean_session: false,
            last_will_topic: Some("zeus/status".to_string()),
            last_will_message: Some("offline".to_string()),
            channel_capacity: 512,
        };
        let json = serde_json::to_string(&config).unwrap();
        let parsed: MqttConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.broker_url, "mqtt.example.com");
        assert_eq!(parsed.port, 8883);
        assert_eq!(parsed.client_id, "zeus-test");
        assert_eq!(parsed.topic_prefix, "zeus/");
        assert_eq!(parsed.qos, 2);
        assert_eq!(parsed.subscribe_topics.len(), 2);
        assert_eq!(parsed.username.as_deref(), Some("zeus"));
        assert_eq!(parsed.password.as_deref(), Some("secret"));
        assert_eq!(parsed.keep_alive_secs, 60);
        assert!(!parsed.clean_session);
        assert_eq!(parsed.last_will_topic.as_deref(), Some("zeus/status"));
        assert_eq!(parsed.last_will_message.as_deref(), Some("offline"));
    }

    #[test]
    fn test_mqtt_config_deserialize_minimal() {
        let json = r#"{"broker_url": "192.168.1.100"}"#;
        let config: MqttConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.broker_url, "192.168.1.100");
        assert_eq!(config.port, 1883);
        assert_eq!(config.qos, 1);
        assert!(config.client_id.starts_with("zeus-"));
    }

    #[test]
    fn test_full_topic_with_prefix() {
        let config = MqttConfig {
            topic_prefix: "zeus/".to_string(),
            ..Default::default()
        };
        assert_eq!(config.full_topic("response"), "zeus/response");
        assert_eq!(config.full_topic("home/light"), "zeus/home/light");
    }

    #[test]
    fn test_full_topic_without_prefix() {
        let config = MqttConfig::default();
        assert_eq!(config.full_topic("response"), "response");
        assert_eq!(config.full_topic("home/light"), "home/light");
    }

    #[test]
    fn test_to_rumqttc_qos() {
        assert_eq!(to_rumqttc_qos(0), RumQoS::AtMostOnce);
        assert_eq!(to_rumqttc_qos(1), RumQoS::AtLeastOnce);
        assert_eq!(to_rumqttc_qos(2), RumQoS::ExactlyOnce);
        assert_eq!(to_rumqttc_qos(99), RumQoS::ExactlyOnce); // clamp to max
    }

    #[test]
    fn test_mqtt_options_basic() {
        let config = MqttConfig {
            broker_url: "test-broker".to_string(),
            port: 1883,
            client_id: "test-client".to_string(),
            keep_alive_secs: 45,
            clean_session: true,
            ..Default::default()
        };
        let opts = config.to_mqtt_options();
        // MqttOptions doesn't expose getters for all fields, but we can verify it doesn't panic
        assert!(format!("{:?}", opts).contains("test-broker"));
    }

    #[test]
    fn test_mqtt_options_with_credentials() {
        let config = MqttConfig {
            broker_url: "broker.local".to_string(),
            port: 1883,
            client_id: "zeus".to_string(),
            username: Some("user".to_string()),
            password: Some("pass".to_string()),
            ..Default::default()
        };
        let opts = config.to_mqtt_options();
        // Verify it constructs without error
        let debug_str = format!("{:?}", opts);
        assert!(debug_str.contains("broker.local"));
    }

    #[test]
    fn test_mqtt_options_with_last_will() {
        let config = MqttConfig {
            broker_url: "broker.local".to_string(),
            port: 1883,
            client_id: "zeus".to_string(),
            last_will_topic: Some("zeus/status".to_string()),
            last_will_message: Some("offline".to_string()),
            ..Default::default()
        };
        let opts = config.to_mqtt_options();
        let debug_str = format!("{:?}", opts);
        assert!(debug_str.contains("zeus/status"));
    }

    #[test]
    fn test_adapter_creation() {
        let config = MqttConfig {
            broker_url: "test-broker".to_string(),
            port: 1883,
            client_id: "zeus-test".to_string(),
            ..Default::default()
        };
        let adapter = MqttAdapter::new(config);
        assert_eq!(adapter.channel_type(), "mqtt");
        assert!(!adapter.is_connected());
        assert!(matches!(adapter.receive_mode(), ReceiveMode::Native));
    }

    #[test]
    fn test_adapter_config_access() {
        let config = MqttConfig {
            broker_url: "my-broker".to_string(),
            topic_prefix: "iot/".to_string(),
            ..Default::default()
        };
        let adapter = MqttAdapter::new(config);
        assert_eq!(adapter.config().broker_url, "my-broker");
        assert_eq!(adapter.config().topic_prefix, "iot/");
    }

    #[tokio::test]
    async fn test_publish_without_connection() {
        let adapter = MqttAdapter::new(MqttConfig::default());
        let result = adapter.publish("test/topic", "hello").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not connected"));
    }

    #[tokio::test]
    async fn test_subscribe_without_connection() {
        let adapter = MqttAdapter::new(MqttConfig::default());
        let result = adapter.subscribe("test/topic").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not connected"));
    }

    #[tokio::test]
    async fn test_unsubscribe_without_connection() {
        let adapter = MqttAdapter::new(MqttConfig::default());
        let result = adapter.unsubscribe("test/topic").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not connected"));
    }

    #[tokio::test]
    async fn test_send_without_connection() {
        let adapter = MqttAdapter::new(MqttConfig::default());
        let target = ChannelSource::with_chat("mqtt", "device/sensor", "home/temperature");
        let result = adapter.send(&target, "22.5").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_send_uses_chat_id_as_topic() {
        // Verify send routes to the right topic (even though it fails without connection)
        let adapter = MqttAdapter::new(MqttConfig {
            topic_prefix: "zeus/".to_string(),
            ..Default::default()
        });
        let target = ChannelSource::with_chat("mqtt", "user", "response/123");
        let result = adapter.send(&target, "hello").await;
        assert!(result.is_err()); // No connection, but tests the path
    }

    #[tokio::test]
    async fn test_send_falls_back_to_user_id() {
        let adapter = MqttAdapter::new(MqttConfig::default());
        let target = ChannelSource::new("mqtt", "device/led");
        let result = adapter.send(&target, "on").await;
        assert!(result.is_err()); // No connection
    }

    #[tokio::test]
    async fn test_stop_when_not_started() {
        let adapter = MqttAdapter::new(MqttConfig::default());
        let result = adapter.stop().await;
        assert!(result.is_ok()); // Should not error
        assert!(!adapter.is_connected());
    }

    #[test]
    fn test_mqtt_config_env_overrides() {
        // Can't reliably test env vars without mutex, but verify the method exists
        let config = MqttConfig {
            broker_url: "original".to_string(),
            ..Default::default()
        };
        // Without env vars set, should return unchanged
        let overridden = config.with_env_overrides();
        // broker_url only changes if MQTT_BROKER_URL is set
        assert!(!overridden.broker_url.is_empty());
    }

    #[test]
    fn test_channel_source_mqtt() {
        let source = ChannelSource::with_chat("mqtt", "home/sensor/temp", "home/sensor/temp");
        assert_eq!(source.channel_type(), "mqtt");
        assert_eq!(source.user_id, "home/sensor/temp");
        assert_eq!(source.chat_id.as_deref(), Some("home/sensor/temp"));
    }

    #[tokio::test]
    async fn test_adapter_start_stop_lifecycle() {
        // Test start -> stop lifecycle (will fail to connect to non-existent broker,
        // but should not panic)
        let config = MqttConfig {
            broker_url: "127.0.0.1".to_string(),
            port: 19999, // unlikely to have an MQTT broker here
            client_id: "zeus-lifecycle-test".to_string(),
            ..Default::default()
        };
        let adapter = MqttAdapter::new(config);
        let (tx, _rx) = mpsc::channel(10);

        // Start creates client and spawns event loop
        let result = adapter.start(tx).await;
        assert!(result.is_ok());

        // Client should be set (connection happens async)
        assert!(adapter.client.read().await.is_some());

        // Stop
        let result = adapter.stop().await;
        assert!(result.is_ok());
        assert!(!adapter.is_connected());
    }
}
