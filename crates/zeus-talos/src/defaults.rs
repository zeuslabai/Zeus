//! macOS defaults (system preferences) tools

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::Value;
use zeus_core::{Error, Result, ToolSchema};

/// Read a defaults value
pub struct DefaultsReadTool;

#[async_trait]
impl TalosTool for DefaultsReadTool {
    fn name(&self) -> &'static str {
        "config_read"
    }
    fn description(&self) -> &'static str {
        "Read a macOS defaults value for a domain and key"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "domain",
                "string",
                "Defaults domain (e.g. com.apple.finder)",
                true,
            )
            .with_param("key", "string", "Key to read", true)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let domain = args
            .get("domain")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing domain".to_string()))?;

        let key = args
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing key".to_string()))?;

        #[cfg(target_os = "macos")]
        {
            // Args passed directly to Command (not through a shell), so no
            // shell-escaping is needed — each arg is a separate OS-level
            // argument and cannot be interpreted as shell metacharacters.
            let output = tokio::process::Command::new("defaults")
                .arg("read")
                .arg(domain)
                .arg(key)
                .output()
                .await
                .map_err(|e| Error::Tool(format!("Failed to read defaults: {}", e)))?;

            if output.status.success() {
                Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
            } else {
                Err(Error::Tool(format!(
                    "defaults read failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                )))
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = (domain, key);
            Ok("config_read only available on macOS".to_string())
        }
    }
}

/// Write a boolean defaults value
pub struct DefaultsWriteBoolTool;

#[async_trait]
impl TalosTool for DefaultsWriteBoolTool {
    fn name(&self) -> &'static str {
        "config_write_bool"
    }
    fn description(&self) -> &'static str {
        "Write a boolean value to macOS defaults"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "domain",
                "string",
                "Defaults domain (e.g. com.apple.finder)",
                true,
            )
            .with_param("key", "string", "Key to write", true)
            .with_param("value", "boolean", "Boolean value to set", true)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let domain = args
            .get("domain")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing domain".to_string()))?;

        let key = args
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing key".to_string()))?;

        let value = args
            .get("value")
            .and_then(|v| v.as_bool())
            .ok_or_else(|| Error::Tool("Missing value (must be boolean)".to_string()))?;

        #[cfg(target_os = "macos")]
        {
            let bool_str = if value { "YES" } else { "NO" };

            let output = tokio::process::Command::new("defaults")
                .arg("write")
                .arg(domain)
                .arg(key)
                .arg("-bool")
                .arg(bool_str)
                .output()
                .await
                .map_err(|e| Error::Tool(format!("Failed to write defaults: {}", e)))?;

            if output.status.success() {
                Ok(format!("Set {} {} to {}", domain, key, value))
            } else {
                Err(Error::Tool(format!(
                    "defaults write failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                )))
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = (domain, key, value);
            Ok("config_write_bool only available on macOS".to_string())
        }
    }
}

/// Write an integer defaults value
pub struct DefaultsWriteIntTool;

#[async_trait]
impl TalosTool for DefaultsWriteIntTool {
    fn name(&self) -> &'static str {
        "config_write_int"
    }
    fn description(&self) -> &'static str {
        "Write an integer value to macOS defaults"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "domain",
                "string",
                "Defaults domain (e.g. com.apple.finder)",
                true,
            )
            .with_param("key", "string", "Key to write", true)
            .with_param("value", "integer", "Integer value to set", true)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let domain = args
            .get("domain")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing domain".to_string()))?;

        let key = args
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing key".to_string()))?;

        let value = args
            .get("value")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| Error::Tool("Missing value (must be integer)".to_string()))?;

        #[cfg(target_os = "macos")]
        {
            let value_str = value.to_string();

            let output = tokio::process::Command::new("defaults")
                .arg("write")
                .arg(domain)
                .arg(key)
                .arg("-int")
                .arg(&value_str)
                .output()
                .await
                .map_err(|e| Error::Tool(format!("Failed to write defaults: {}", e)))?;

            if output.status.success() {
                Ok(format!("Set {} {} to {}", domain, key, value))
            } else {
                Err(Error::Tool(format!(
                    "defaults write failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                )))
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = (domain, key, value);
            Ok("config_write_int only available on macOS".to_string())
        }
    }
}

/// Write a string defaults value
pub struct DefaultsWriteStringTool;

#[async_trait]
impl TalosTool for DefaultsWriteStringTool {
    fn name(&self) -> &'static str {
        "config_write_string"
    }
    fn description(&self) -> &'static str {
        "Write a string value to macOS defaults"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "domain",
                "string",
                "Defaults domain (e.g. com.apple.finder)",
                true,
            )
            .with_param("key", "string", "Key to write", true)
            .with_param("value", "string", "String value to set", true)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let domain = args
            .get("domain")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing domain".to_string()))?;

        let key = args
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing key".to_string()))?;

        let value = args
            .get("value")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing value (must be string)".to_string()))?;

        #[cfg(target_os = "macos")]
        {
            let output = tokio::process::Command::new("defaults")
                .arg("write")
                .arg(domain)
                .arg(key)
                .arg("-string")
                .arg(value)
                .output()
                .await
                .map_err(|e| Error::Tool(format!("Failed to write defaults: {}", e)))?;

            if output.status.success() {
                Ok(format!("Set {} {} to {}", domain, key, value))
            } else {
                Err(Error::Tool(format!(
                    "defaults write failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                )))
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = (domain, key, value);
            Ok("config_write_string only available on macOS".to_string())
        }
    }
}

/// List all keys in a defaults domain
pub struct DefaultsListDomainTool;

#[async_trait]
impl TalosTool for DefaultsListDomainTool {
    fn name(&self) -> &'static str {
        "config_list_domain"
    }
    fn description(&self) -> &'static str {
        "List all keys and values in a macOS defaults domain"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "domain",
            "string",
            "Defaults domain to list (e.g. com.apple.finder)",
            true,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let domain = args
            .get("domain")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing domain".to_string()))?;

        #[cfg(target_os = "macos")]
        {
            let output = tokio::process::Command::new("defaults")
                .arg("read")
                .arg(domain)
                .output()
                .await
                .map_err(|e| Error::Tool(format!("Failed to list defaults domain: {}", e)))?;

            if output.status.success() {
                Ok(String::from_utf8_lossy(&output.stdout).to_string())
            } else {
                Err(Error::Tool(format!(
                    "defaults read domain failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                )))
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = domain;
            Ok("config_list_domain only available on macOS".to_string())
        }
    }
}

/// List all defaults domains
pub struct DefaultsListDomainsTool;

#[async_trait]
impl TalosTool for DefaultsListDomainsTool {
    fn name(&self) -> &'static str {
        "config_list_domains"
    }
    fn description(&self) -> &'static str {
        "List all macOS defaults domains"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let output = tokio::process::Command::new("defaults")
                .arg("domains")
                .output()
                .await
                .map_err(|e| Error::Tool(format!("Failed to list defaults domains: {}", e)))?;

            if output.status.success() {
                Ok(String::from_utf8_lossy(&output.stdout).to_string())
            } else {
                Err(Error::Tool(format!(
                    "defaults domains failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                )))
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            Ok("config_list_domains only available on macOS".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_defaults_read_schema() {
        let tool = DefaultsReadTool;
        let schema = tool.schema();
        assert_eq!(schema.name, "config_read");
        let required = schema
            .parameters
            .get("required")
            .and_then(|r| r.as_array())
            .expect("should have required array");
        assert!(required.iter().any(|v| v.as_str() == Some("domain")));
        assert!(required.iter().any(|v| v.as_str() == Some("key")));
    }

    #[test]
    fn test_defaults_write_bool_schema() {
        let tool = DefaultsWriteBoolTool;
        let schema = tool.schema();
        assert_eq!(schema.name, "config_write_bool");
        let required = schema
            .parameters
            .get("required")
            .and_then(|r| r.as_array())
            .expect("should have required array");
        assert!(required.iter().any(|v| v.as_str() == Some("domain")));
        assert!(required.iter().any(|v| v.as_str() == Some("key")));
        assert!(required.iter().any(|v| v.as_str() == Some("value")));
    }

    #[test]
    fn test_defaults_write_int_schema() {
        let tool = DefaultsWriteIntTool;
        let schema = tool.schema();
        assert_eq!(schema.name, "config_write_int");
        let required = schema
            .parameters
            .get("required")
            .and_then(|r| r.as_array())
            .expect("should have required array");
        assert!(required.iter().any(|v| v.as_str() == Some("domain")));
        assert!(required.iter().any(|v| v.as_str() == Some("key")));
        assert!(required.iter().any(|v| v.as_str() == Some("value")));
    }

    #[test]
    fn test_defaults_write_string_schema() {
        let tool = DefaultsWriteStringTool;
        let schema = tool.schema();
        assert_eq!(schema.name, "config_write_string");
        let required = schema
            .parameters
            .get("required")
            .and_then(|r| r.as_array())
            .expect("should have required array");
        assert!(required.iter().any(|v| v.as_str() == Some("domain")));
        assert!(required.iter().any(|v| v.as_str() == Some("key")));
        assert!(required.iter().any(|v| v.as_str() == Some("value")));
    }

    #[test]
    fn test_defaults_list_domain_schema() {
        let tool = DefaultsListDomainTool;
        let schema = tool.schema();
        assert_eq!(schema.name, "config_list_domain");
        let required = schema
            .parameters
            .get("required")
            .and_then(|r| r.as_array())
            .expect("should have required array");
        assert!(required.iter().any(|v| v.as_str() == Some("domain")));
    }

    #[test]
    fn test_defaults_list_domains_schema() {
        let tool = DefaultsListDomainsTool;
        let schema = tool.schema();
        assert_eq!(schema.name, "config_list_domains");
    }
}
