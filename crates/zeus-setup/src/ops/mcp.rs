//! MCP configuration — Claude Code and Claude Desktop

use crate::config;
use anyhow::{Context, Result};

/// Configure Zeus MCP for Claude Code (~/.claude.json)
pub async fn configure_code() -> Result<()> {
    let zeus_bin = config::zeus_bin();
    if !zeus_bin.exists() {
        anyhow::bail!(
            "Zeus binary not found at {} — install first",
            zeus_bin.display()
        );
    }

    let config_path = dirs::home_dir()
        .context("Could not determine home directory")?
        .join(".claude.json");

    let mut data: serde_json::Value = if config_path.exists() {
        let content = std::fs::read_to_string(&config_path)?;
        serde_json::from_str(&content).unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    // Ensure mcpServers object exists
    if data.get("mcpServers").is_none() {
        data["mcpServers"] = serde_json::json!({});
    }

    // Add zeus MCP server with HOME env so it can find ~/.zeus/.env
    let home_dir = dirs::home_dir().context("Could not determine home directory")?;
    data["mcpServers"]["zeus"] = serde_json::json!({
        "command": zeus_bin.to_string_lossy(),
        "args": ["mcp"],
        "type": "stdio",
        "env": {
            "HOME": home_dir.to_string_lossy(),
            "ZEUS_HOME": home_dir.join(".zeus").to_string_lossy().to_string()
        }
    });

    std::fs::write(&config_path, serde_json::to_string_pretty(&data)?)?;

    Ok(())
}

/// Configure Zeus MCP for Claude Desktop
pub async fn configure_desktop() -> Result<()> {
    let zeus_bin = config::zeus_bin();
    if !zeus_bin.exists() {
        anyhow::bail!(
            "Zeus binary not found at {} — install first",
            zeus_bin.display()
        );
    }

    let home_dir = dirs::home_dir().context("Could not determine home directory")?;

    let config_path = if cfg!(target_os = "macos") {
        home_dir.join("Library/Application Support/Claude/claude_desktop_config.json")
    } else {
        home_dir.join(".config/Claude/claude_desktop_config.json")
    };

    let mut data: serde_json::Value = if config_path.exists() {
        let content = std::fs::read_to_string(&config_path)?;
        serde_json::from_str(&content).unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    if data.get("mcpServers").is_none() {
        data["mcpServers"] = serde_json::json!({});
    }

    data["mcpServers"]["zeus"] = serde_json::json!({
        "command": zeus_bin.to_string_lossy(),
        "args": ["mcp"],
        "type": "stdio",
        "env": {
            "HOME": home_dir.to_string_lossy(),
            "ZEUS_HOME": home_dir.join(".zeus").to_string_lossy().to_string()
        }
    });

    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    std::fs::write(&config_path, serde_json::to_string_pretty(&data)?)?;

    Ok(())
}

/// Remove Zeus MCP from all configs
pub async fn remove() -> Result<()> {
    // Claude Code
    let claude_json = dirs::home_dir()
        .context("Could not determine home directory")?
        .join(".claude.json");

    if claude_json.exists() {
        let content = std::fs::read_to_string(&claude_json)?;
        if let Ok(mut data) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(servers) = data.get_mut("mcpServers")
                && let Some(obj) = servers.as_object_mut()
            {
                obj.remove("zeus");
            }
            std::fs::write(&claude_json, serde_json::to_string_pretty(&data)?)?;
        }
    }

    // Claude Desktop
    let desktop_path = if cfg!(target_os = "macos") {
        dirs::home_dir()
            .map(|h| h.join("Library/Application Support/Claude/claude_desktop_config.json"))
    } else {
        dirs::home_dir().map(|h| h.join(".config/Claude/claude_desktop_config.json"))
    };

    if let Some(path) = desktop_path
        && path.exists()
    {
        let content = std::fs::read_to_string(&path)?;
        if let Ok(mut data) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(servers) = data.get_mut("mcpServers")
                && let Some(obj) = servers.as_object_mut()
            {
                obj.remove("zeus");
            }
            std::fs::write(&path, serde_json::to_string_pretty(&data)?)?;
        }
    }

    Ok(())
}

/// Show current MCP configuration
pub async fn show() -> Result<String> {
    let mut output = String::new();

    // Claude Code
    let claude_json = dirs::home_dir()
        .context("Could not determine home directory")?
        .join(".claude.json");

    if claude_json.exists() {
        let content = std::fs::read_to_string(&claude_json)?;
        if let Ok(data) = serde_json::from_str::<serde_json::Value>(&content)
            && let Some(servers) = data.get("mcpServers")
        {
            if let Some(zeus) = servers.get("zeus") {
                output.push_str(&format!(
                    "Claude Code: {}\n",
                    serde_json::to_string_pretty(zeus)?
                ));
            } else {
                output.push_str("Claude Code: Not configured\n");
            }
        }
    } else {
        output.push_str("Claude Code: ~/.claude.json not found\n");
    }

    // Claude Desktop
    let desktop_path = if cfg!(target_os = "macos") {
        dirs::home_dir()
            .map(|h| h.join("Library/Application Support/Claude/claude_desktop_config.json"))
    } else {
        dirs::home_dir().map(|h| h.join(".config/Claude/claude_desktop_config.json"))
    };

    if let Some(path) = desktop_path {
        if path.exists() {
            let content = std::fs::read_to_string(&path)?;
            if let Ok(data) = serde_json::from_str::<serde_json::Value>(&content)
                && let Some(servers) = data.get("mcpServers")
            {
                if let Some(zeus) = servers.get("zeus") {
                    output.push_str(&format!(
                        "Claude Desktop: {}\n",
                        serde_json::to_string_pretty(zeus)?
                    ));
                } else {
                    output.push_str("Claude Desktop: Not configured\n");
                }
            }
        } else {
            output.push_str("Claude Desktop: Config file not found\n");
        }
    }

    Ok(output)
}
