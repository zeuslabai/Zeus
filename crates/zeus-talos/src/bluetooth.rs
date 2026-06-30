//! Bluetooth tools using blueutil and system_profiler

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::Value;
use zeus_core::{Error, Result, ToolSchema};

/// List paired/connected Bluetooth devices
pub struct BluetoothListDevicesTool;

#[async_trait]
impl TalosTool for BluetoothListDevicesTool {
    fn name(&self) -> &'static str {
        "bluetooth_list"
    }
    fn description(&self) -> &'static str {
        "List paired and connected Bluetooth devices"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let output = tokio::process::Command::new("system_profiler")
                .args(["SPBluetoothDataType", "-json"])
                .output()
                .await
                .map_err(|e| Error::Tool(format!("Failed to list Bluetooth devices: {}", e)))?;

            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout).to_string();
                if let Ok(parsed) = serde_json::from_str::<Value>(&text) {
                    Ok(serde_json::to_string_pretty(&parsed)?)
                } else {
                    Ok(text)
                }
            } else {
                Err(Error::Tool(format!(
                    "Bluetooth list failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                )))
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            Ok("bluetooth_list only available on macOS".to_string())
        }
    }
}

/// Connect to a Bluetooth device by address
pub struct BluetoothConnectTool;

#[async_trait]
impl TalosTool for BluetoothConnectTool {
    fn name(&self) -> &'static str {
        "bluetooth_connect"
    }
    fn description(&self) -> &'static str {
        "Connect to a Bluetooth device by MAC address"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "address",
            "string",
            "Bluetooth device MAC address",
            true,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let address = args
            .get("address")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing address".to_string()))?;

        #[cfg(target_os = "macos")]
        {
            let sanitized = crate::sanitize_shell_arg(address);
            let output = tokio::process::Command::new("blueutil")
                .args(["--connect", &sanitized])
                .output()
                .await
                .map_err(|e| {
                    Error::Tool(format!(
                        "Failed to connect Bluetooth device (is blueutil installed?): {}",
                        e
                    ))
                })?;

            if output.status.success() {
                Ok(format!("Connected to Bluetooth device {}", address))
            } else {
                Err(Error::Tool(format!(
                    "Bluetooth connect failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                )))
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = address;
            Ok("bluetooth_connect only available on macOS".to_string())
        }
    }
}

/// Disconnect a Bluetooth device by address
pub struct BluetoothDisconnectTool;

#[async_trait]
impl TalosTool for BluetoothDisconnectTool {
    fn name(&self) -> &'static str {
        "bluetooth_disconnect"
    }
    fn description(&self) -> &'static str {
        "Disconnect a Bluetooth device by MAC address"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "address",
            "string",
            "Bluetooth device MAC address",
            true,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let address = args
            .get("address")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing address".to_string()))?;

        #[cfg(target_os = "macos")]
        {
            let sanitized = crate::sanitize_shell_arg(address);
            let output = tokio::process::Command::new("blueutil")
                .args(["--disconnect", &sanitized])
                .output()
                .await
                .map_err(|e| {
                    Error::Tool(format!(
                        "Failed to disconnect Bluetooth device (is blueutil installed?): {}",
                        e
                    ))
                })?;

            if output.status.success() {
                Ok(format!("Disconnected Bluetooth device {}", address))
            } else {
                Err(Error::Tool(format!(
                    "Bluetooth disconnect failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                )))
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = address;
            Ok("bluetooth_disconnect only available on macOS".to_string())
        }
    }
}

/// Pair with a Bluetooth device by address
pub struct BluetoothPairTool;

#[async_trait]
impl TalosTool for BluetoothPairTool {
    fn name(&self) -> &'static str {
        "bluetooth_pair"
    }
    fn description(&self) -> &'static str {
        "Pair with a Bluetooth device by MAC address"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "address",
            "string",
            "Bluetooth device MAC address",
            true,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let address = args
            .get("address")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing address".to_string()))?;

        #[cfg(target_os = "macos")]
        {
            let sanitized = crate::sanitize_shell_arg(address);
            let output = tokio::process::Command::new("blueutil")
                .args(["--pair", &sanitized])
                .output()
                .await
                .map_err(|e| {
                    Error::Tool(format!(
                        "Failed to pair Bluetooth device (is blueutil installed?): {}",
                        e
                    ))
                })?;

            if output.status.success() {
                Ok(format!("Paired with Bluetooth device {}", address))
            } else {
                Err(Error::Tool(format!(
                    "Bluetooth pair failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                )))
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = address;
            Ok("bluetooth_pair only available on macOS".to_string())
        }
    }
}

/// Unpair a Bluetooth device by address
pub struct BluetoothUnpairTool;

#[async_trait]
impl TalosTool for BluetoothUnpairTool {
    fn name(&self) -> &'static str {
        "bluetooth_unpair"
    }
    fn description(&self) -> &'static str {
        "Unpair a Bluetooth device by MAC address"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "address",
            "string",
            "Bluetooth device MAC address",
            true,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let address = args
            .get("address")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing address".to_string()))?;

        #[cfg(target_os = "macos")]
        {
            let sanitized = crate::sanitize_shell_arg(address);
            let output = tokio::process::Command::new("blueutil")
                .args(["--unpair", &sanitized])
                .output()
                .await
                .map_err(|e| {
                    Error::Tool(format!(
                        "Failed to unpair Bluetooth device (is blueutil installed?): {}",
                        e
                    ))
                })?;

            if output.status.success() {
                Ok(format!("Unpaired Bluetooth device {}", address))
            } else {
                Err(Error::Tool(format!(
                    "Bluetooth unpair failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                )))
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = address;
            Ok("bluetooth_unpair only available on macOS".to_string())
        }
    }
}

/// Get or set Bluetooth power state
pub struct BluetoothPowerTool;

#[async_trait]
impl TalosTool for BluetoothPowerTool {
    fn name(&self) -> &'static str {
        "bluetooth_power"
    }
    fn description(&self) -> &'static str {
        "Get or set Bluetooth power state"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "enabled",
            "boolean",
            "true to enable, false to disable (omit to get current state)",
            false,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            if let Some(enabled) = args.get("enabled").and_then(|v| v.as_bool()) {
                let power_val = if enabled { "1" } else { "0" };
                let output = tokio::process::Command::new("blueutil")
                    .args(["--power", power_val])
                    .output()
                    .await
                    .map_err(|e| {
                        Error::Tool(format!(
                            "Failed to set Bluetooth power (is blueutil installed?): {}",
                            e
                        ))
                    })?;

                if output.status.success() {
                    let state = if enabled { "on" } else { "off" };
                    Ok(format!("Bluetooth turned {}", state))
                } else {
                    Err(Error::Tool(format!(
                        "Bluetooth power set failed: {}",
                        String::from_utf8_lossy(&output.stderr)
                    )))
                }
            } else {
                let output = tokio::process::Command::new("blueutil")
                    .arg("--power")
                    .output()
                    .await
                    .map_err(|e| {
                        Error::Tool(format!(
                            "Failed to get Bluetooth power (is blueutil installed?): {}",
                            e
                        ))
                    })?;

                if output.status.success() {
                    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    let is_on = text == "1";
                    let state = if is_on { "on" } else { "off" };
                    Ok(format!("Bluetooth is {}", state))
                } else {
                    Err(Error::Tool(format!(
                        "Bluetooth power get failed: {}",
                        String::from_utf8_lossy(&output.stderr)
                    )))
                }
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("bluetooth_power only available on macOS".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bluetooth_list_devices_schema() {
        let tool = BluetoothListDevicesTool;
        let schema = tool.schema();
        assert_eq!(schema.name, "bluetooth_list");
    }

    #[test]
    fn test_bluetooth_connect_schema() {
        let tool = BluetoothConnectTool;
        let schema = tool.schema();
        assert_eq!(schema.name, "bluetooth_connect");
        assert!(
            schema
                .parameters
                .get("properties")
                .and_then(|p| p.get("address"))
                .is_some()
        );
        assert!(
            schema
                .parameters
                .get("required")
                .and_then(|r| r.as_array())
                .map(|arr| arr.iter().any(|v| v.as_str() == Some("address")))
                .unwrap_or(false)
        );
    }

    #[test]
    fn test_bluetooth_disconnect_schema() {
        let tool = BluetoothDisconnectTool;
        let schema = tool.schema();
        assert_eq!(schema.name, "bluetooth_disconnect");
        assert!(
            schema
                .parameters
                .get("properties")
                .and_then(|p| p.get("address"))
                .is_some()
        );
        assert!(
            schema
                .parameters
                .get("required")
                .and_then(|r| r.as_array())
                .map(|arr| arr.iter().any(|v| v.as_str() == Some("address")))
                .unwrap_or(false)
        );
    }

    #[test]
    fn test_bluetooth_pair_schema() {
        let tool = BluetoothPairTool;
        let schema = tool.schema();
        assert_eq!(schema.name, "bluetooth_pair");
        assert!(
            schema
                .parameters
                .get("properties")
                .and_then(|p| p.get("address"))
                .is_some()
        );
        assert!(
            schema
                .parameters
                .get("required")
                .and_then(|r| r.as_array())
                .map(|arr| arr.iter().any(|v| v.as_str() == Some("address")))
                .unwrap_or(false)
        );
    }

    #[test]
    fn test_bluetooth_unpair_schema() {
        let tool = BluetoothUnpairTool;
        let schema = tool.schema();
        assert_eq!(schema.name, "bluetooth_unpair");
        assert!(
            schema
                .parameters
                .get("properties")
                .and_then(|p| p.get("address"))
                .is_some()
        );
        assert!(
            schema
                .parameters
                .get("required")
                .and_then(|r| r.as_array())
                .map(|arr| arr.iter().any(|v| v.as_str() == Some("address")))
                .unwrap_or(false)
        );
    }

    #[test]
    fn test_bluetooth_power_schema() {
        let tool = BluetoothPowerTool;
        let schema = tool.schema();
        assert_eq!(schema.name, "bluetooth_power");
        // enabled is optional, so it should not be in required
        let required = schema
            .parameters
            .get("required")
            .and_then(|r| r.as_array())
            .cloned()
            .unwrap_or_default();
        assert!(!required.iter().any(|v| v.as_str() == Some("enabled")));
    }
}
