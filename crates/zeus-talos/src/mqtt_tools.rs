//! MQTT Protocol tools
//!
//! Provides tools for publishing and subscribing to MQTT topics via the
//! `mosquitto_pub` and `mosquitto_sub` CLI tools.
//! Each tool accepts optional connection parameters, falling back to
//! `MQTT_BROKER_HOST`, `MQTT_BROKER_PORT`, `MQTT_USERNAME`, and
//! `MQTT_PASSWORD` environment variables.

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::Value;
use zeus_core::{Error, Result, ToolSchema};

const DEFAULT_HOST: &str = "localhost";
const DEFAULT_PORT: &str = "1883";

/// Get broker host from args or environment
fn get_host(args: &Value) -> String {
    if let Some(host) = args.get("host").and_then(|v| v.as_str()) {
        return host.to_string();
    }
    std::env::var("MQTT_BROKER_HOST").unwrap_or_else(|_| DEFAULT_HOST.to_string())
}

/// Get broker port from args or environment
fn get_port(args: &Value) -> String {
    if let Some(port) = args.get("port").and_then(|v| v.as_str()) {
        return port.to_string();
    }
    if let Some(port) = args.get("port").and_then(|v| v.as_u64()) {
        return port.to_string();
    }
    std::env::var("MQTT_BROKER_PORT").unwrap_or_else(|_| DEFAULT_PORT.to_string())
}

/// Get optional username from args or environment
fn get_username(args: &Value) -> Option<String> {
    if let Some(user) = args.get("username").and_then(|v| v.as_str()) {
        return Some(user.to_string());
    }
    std::env::var("MQTT_USERNAME").ok()
}

/// Get optional password from args or environment
fn get_password(args: &Value) -> Option<String> {
    if let Some(pass) = args.get("password").and_then(|v| v.as_str()) {
        return Some(pass.to_string());
    }
    std::env::var("MQTT_PASSWORD").ok()
}

/// Build common mosquitto CLI arguments for host, port, and auth
fn build_connection_args(args: &Value) -> Vec<String> {
    let mut cmd_args = vec![
        "-h".to_string(),
        get_host(args),
        "-p".to_string(),
        get_port(args),
    ];

    if let Some(user) = get_username(args) {
        cmd_args.push("-u".to_string());
        cmd_args.push(user);
    }
    if let Some(pass) = get_password(args) {
        cmd_args.push("-P".to_string());
        cmd_args.push(pass);
    }

    cmd_args
}

// ---------------------------------------------------------------------------
// 1. MqttPublishTool
// ---------------------------------------------------------------------------

/// Publish a message to an MQTT topic
pub struct MqttPublishTool;

#[async_trait]
impl TalosTool for MqttPublishTool {
    fn name(&self) -> &'static str {
        "mqtt_publish"
    }
    fn description(&self) -> &'static str {
        "Publish a message to an MQTT topic via mosquitto_pub"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("topic", "string", "MQTT topic to publish to", true)
            .with_param("message", "string", "Message payload to publish", true)
            .with_param(
                "host",
                "string",
                "Broker hostname (or set MQTT_BROKER_HOST, default: localhost)",
                false,
            )
            .with_param(
                "port",
                "string",
                "Broker port (or set MQTT_BROKER_PORT, default: 1883)",
                false,
            )
            .with_param(
                "username",
                "string",
                "Username (or set MQTT_USERNAME env var)",
                false,
            )
            .with_param(
                "password",
                "string",
                "Password (or set MQTT_PASSWORD env var)",
                false,
            )
            .with_param(
                "qos",
                "integer",
                "QoS level: 0, 1, or 2 (default: 0)",
                false,
            )
            .with_param(
                "retain",
                "boolean",
                "Set retain flag on the message (default: false)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let topic = args
            .get("topic")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'topic'".to_string()))?;
        let message = args
            .get("message")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'message'".to_string()))?;

        let mut cmd_args = build_connection_args(&args);
        cmd_args.push("-t".to_string());
        cmd_args.push(topic.to_string());
        cmd_args.push("-m".to_string());
        cmd_args.push(message.to_string());

        let qos = args.get("qos").and_then(|v| v.as_u64()).unwrap_or(0).min(2);
        cmd_args.push("-q".to_string());
        cmd_args.push(qos.to_string());

        if args
            .get("retain")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            cmd_args.push("-r".to_string());
        }

        let output = tokio::process::Command::new("mosquitto_pub")
            .args(&cmd_args)
            .output()
            .await
            .map_err(|e| {
                Error::Tool(format!(
                    "Failed to execute mosquitto_pub (is it installed?): {}",
                    e
                ))
            })?;

        if output.status.success() {
            Ok(format!(
                "Message published to topic '{}' on {}:{} (QoS {})",
                topic,
                get_host(&args),
                get_port(&args),
                qos
            ))
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            Err(Error::Tool(format!("mosquitto_pub failed: {}", stderr)))
        }
    }
}

// ---------------------------------------------------------------------------
// 2. MqttSubscribeTool
// ---------------------------------------------------------------------------

/// Subscribe to an MQTT topic and receive messages
pub struct MqttSubscribeTool;

#[async_trait]
impl TalosTool for MqttSubscribeTool {
    fn name(&self) -> &'static str {
        "mqtt_subscribe"
    }
    fn description(&self) -> &'static str {
        "Subscribe to an MQTT topic and receive messages via mosquitto_sub"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "topic",
                "string",
                "MQTT topic to subscribe to (supports wildcards: +, #)",
                true,
            )
            .with_param(
                "count",
                "integer",
                "Number of messages to receive before exiting (default: 1)",
                false,
            )
            .with_param(
                "timeout",
                "integer",
                "Timeout in seconds (default: 10)",
                false,
            )
            .with_param(
                "host",
                "string",
                "Broker hostname (or set MQTT_BROKER_HOST, default: localhost)",
                false,
            )
            .with_param(
                "port",
                "string",
                "Broker port (or set MQTT_BROKER_PORT, default: 1883)",
                false,
            )
            .with_param(
                "username",
                "string",
                "Username (or set MQTT_USERNAME env var)",
                false,
            )
            .with_param(
                "password",
                "string",
                "Password (or set MQTT_PASSWORD env var)",
                false,
            )
            .with_param(
                "verbose",
                "boolean",
                "Show topic names with messages (default: false)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let topic = args
            .get("topic")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing 'topic'".to_string()))?;

        let count = args
            .get("count")
            .and_then(|v| v.as_u64())
            .unwrap_or(1)
            .max(1);
        let timeout_secs = args
            .get("timeout")
            .and_then(|v| v.as_u64())
            .unwrap_or(10)
            .clamp(1, 300);

        let mut cmd_args = build_connection_args(&args);
        cmd_args.push("-t".to_string());
        cmd_args.push(topic.to_string());
        cmd_args.push("-C".to_string());
        cmd_args.push(count.to_string());
        cmd_args.push("-W".to_string());
        cmd_args.push(timeout_secs.to_string());

        if args
            .get("verbose")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            cmd_args.push("-v".to_string());
        }

        let output = tokio::process::Command::new("mosquitto_sub")
            .args(&cmd_args)
            .output()
            .await
            .map_err(|e| {
                Error::Tool(format!(
                    "Failed to execute mosquitto_sub (is it installed?): {}",
                    e
                ))
            })?;

        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();

        if output.status.success() || !stdout.is_empty() {
            if stdout.is_empty() {
                Ok(format!(
                    "No messages received on topic '{}' within {} seconds.",
                    topic, timeout_secs
                ))
            } else {
                let lines: Vec<&str> = stdout.lines().collect();
                Ok(format!(
                    "{} message(s) from topic '{}':\n{}",
                    lines.len(),
                    topic,
                    stdout
                ))
            }
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            if stderr.contains("timed out") || stderr.contains("timeout") {
                Ok(format!(
                    "No messages received on topic '{}' within {} seconds (timed out).",
                    topic, timeout_secs
                ))
            } else {
                Err(Error::Tool(format!("mosquitto_sub failed: {}", stderr)))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_publish_schema() {
        let tool = MqttPublishTool;
        assert_eq!(tool.name(), "mqtt_publish");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("params");
        let required = params["required"].as_array().expect("required");
        let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(names.contains(&"topic"));
        assert!(names.contains(&"message"));
    }

    #[test]
    fn test_subscribe_schema() {
        let tool = MqttSubscribeTool;
        assert_eq!(tool.name(), "mqtt_subscribe");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("params");
        let required = params["required"].as_array().expect("required");
        let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(names.contains(&"topic"));
    }

    #[test]
    fn test_get_host_from_args() {
        let args = json!({"host": "mqtt.example.com"});
        let host = get_host(&args);
        assert_eq!(host, "mqtt.example.com");
    }

    #[test]
    fn test_get_host_default() {
        let args = json!({});
        let host = get_host(&args);
        // Will be env var or default "localhost"
        assert!(!host.is_empty());
    }

    #[test]
    fn test_get_port_from_args_string() {
        let args = json!({"port": "8883"});
        let port = get_port(&args);
        assert_eq!(port, "8883");
    }

    #[test]
    fn test_get_port_from_args_integer() {
        let args = json!({"port": 8883});
        let port = get_port(&args);
        assert_eq!(port, "8883");
    }

    #[test]
    fn test_build_connection_args_basic() {
        let args = json!({"host": "broker.local", "port": "1884"});
        let cmd_args = build_connection_args(&args);
        assert!(cmd_args.contains(&"-h".to_string()));
        assert!(cmd_args.contains(&"broker.local".to_string()));
        assert!(cmd_args.contains(&"-p".to_string()));
        assert!(cmd_args.contains(&"1884".to_string()));
    }

    #[test]
    fn test_build_connection_args_with_auth() {
        let args = json!({"host": "broker.local", "username": "user1", "password": "pass1"});
        let cmd_args = build_connection_args(&args);
        assert!(cmd_args.contains(&"-u".to_string()));
        assert!(cmd_args.contains(&"user1".to_string()));
        assert!(cmd_args.contains(&"-P".to_string()));
        assert!(cmd_args.contains(&"pass1".to_string()));
    }
}
