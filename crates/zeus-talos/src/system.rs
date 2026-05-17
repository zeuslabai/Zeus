//! System information and process management tools

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::{Value, json};
use sysinfo::{Disks, Pid, System};
use zeus_core::{Error, Result, ToolSchema};

/// Get system information
pub struct SystemInfoTool;

#[async_trait]
impl TalosTool for SystemInfoTool {
    fn name(&self) -> &'static str {
        "system_info"
    }
    fn description(&self) -> &'static str {
        "Get system information (OS, CPU, memory, uptime)"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        let mut sys = System::new_all();
        sys.refresh_all();

        let info = json!({
            "os": System::name().unwrap_or_default(),
            "os_version": System::os_version().unwrap_or_default(),
            "kernel_version": System::kernel_version().unwrap_or_default(),
            "host_name": System::host_name().unwrap_or_default(),
            "cpu_count": sys.cpus().len(),
            "total_memory_gb": sys.total_memory() as f64 / 1024.0 / 1024.0 / 1024.0,
            "used_memory_gb": sys.used_memory() as f64 / 1024.0 / 1024.0 / 1024.0,
            "uptime_hours": System::uptime() / 3600,
        });

        Ok(serde_json::to_string_pretty(&info)?)
    }
}

/// List running processes
pub struct ProcessListTool;

#[async_trait]
impl TalosTool for ProcessListTool {
    fn name(&self) -> &'static str {
        "process_list"
    }
    fn description(&self) -> &'static str {
        "List running processes with CPU and memory usage"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "filter",
                "string",
                "Filter processes by name (optional)",
                false,
            )
            .with_param(
                "limit",
                "integer",
                "Max number of processes to return (default 20)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let filter = args.get("filter").and_then(|v| v.as_str());
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;

        let mut sys = System::new_all();
        sys.refresh_all();

        let mut processes: Vec<Value> = sys
            .processes()
            .values()
            .filter(|p| {
                if let Some(f) = filter {
                    p.name()
                        .to_string_lossy()
                        .to_lowercase()
                        .contains(&f.to_lowercase())
                } else {
                    true
                }
            })
            .map(|p| {
                json!({
                    "pid": p.pid().to_string(),
                    "name": p.name().to_string_lossy(),
                    "cpu_usage": format!("{:.1}%", p.cpu_usage()),
                    "memory_mb": p.memory() / 1024 / 1024,
                })
            })
            .collect();

        // Sort by memory usage descending BEFORE taking top N
        processes.sort_by(|a, b| {
            let ma = a["memory_mb"].as_u64().unwrap_or(0);
            let mb = b["memory_mb"].as_u64().unwrap_or(0);
            mb.cmp(&ma)
        });
        processes.truncate(limit);

        Ok(serde_json::to_string_pretty(&processes)?)
    }
}

/// Kill a process by PID
pub struct KillProcessTool;

#[async_trait]
impl TalosTool for KillProcessTool {
    fn name(&self) -> &'static str {
        "kill_process"
    }
    fn description(&self) -> &'static str {
        "Kill a process by PID"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "pid",
            "integer",
            "Process ID to kill",
            true,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let pid = args
            .get("pid")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| Error::Tool("Missing pid".to_string()))?;

        let mut sys = System::new_all();
        sys.refresh_all();

        if let Some(process) = sys.process(Pid::from(pid as usize)) {
            if process.kill() {
                Ok(format!("Process {} killed", pid))
            } else {
                Err(Error::Tool(format!("Failed to kill process {}", pid)))
            }
        } else {
            Err(Error::Tool(format!("Process {} not found", pid)))
        }
    }
}

/// Get disk usage
pub struct DiskUsageTool;

#[async_trait]
impl TalosTool for DiskUsageTool {
    fn name(&self) -> &'static str {
        "disk_usage"
    }
    fn description(&self) -> &'static str {
        "Get disk usage for all mounted volumes"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        let disks = Disks::new_with_refreshed_list();

        let disk_info: Vec<Value> = disks
            .iter()
            .map(|d| {
                let total_gb = d.total_space() as f64 / 1024.0 / 1024.0 / 1024.0;
                let available_gb = d.available_space() as f64 / 1024.0 / 1024.0 / 1024.0;
                let used_gb = total_gb - available_gb;
                let used_percent = if total_gb > 0.0 {
                    (used_gb / total_gb) * 100.0
                } else {
                    0.0
                };

                json!({
                    "mount_point": d.mount_point().to_string_lossy(),
                    "total_gb": format!("{:.1}", total_gb),
                    "used_gb": format!("{:.1}", used_gb),
                    "available_gb": format!("{:.1}", available_gb),
                    "used_percent": format!("{:.1}%", used_percent),
                })
            })
            .collect();

        Ok(serde_json::to_string_pretty(&disk_info)?)
    }
}

/// Send a macOS notification
pub struct SystemNotifyTool;

#[async_trait]
impl TalosTool for SystemNotifyTool {
    fn name(&self) -> &'static str {
        "system_notify"
    }
    fn description(&self) -> &'static str {
        "Send a macOS notification via Notification Center"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("title", "string", "Notification title", true)
            .with_param("message", "string", "Notification message", true)
            .with_param("sound", "boolean", "Play sound (default true)", false)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let title = args
            .get("title")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing title".to_string()))?;

        let message = args
            .get("message")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing message".to_string()))?;

        let sound = args.get("sound").and_then(|v| v.as_bool()).unwrap_or(true);

        #[cfg(target_os = "macos")]
        {
            let sound_str = if sound {
                r#" sound name "default""#
            } else {
                ""
            };
            let script = format!(
                r#"display notification "{}" with title "{}"{}"#,
                crate::sanitize_applescript(message),
                crate::sanitize_applescript(title),
                sound_str
            );
            crate::run_applescript(&script)?;
            Ok(format!("Notification sent: {}", title))
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = sound;
            Ok(format!("Notification (console): {} - {}", title, message))
        }
    }
}

/// Read from clipboard
pub struct ClipboardReadTool;

#[async_trait]
impl TalosTool for ClipboardReadTool {
    fn name(&self) -> &'static str {
        "clipboard_read"
    }
    fn description(&self) -> &'static str {
        "Read the current clipboard contents"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            crate::run_applescript("the clipboard")
        }

        #[cfg(not(target_os = "macos"))]
        {
            Ok("Clipboard read not available on this platform".to_string())
        }
    }
}

/// Write to clipboard
pub struct ClipboardWriteTool;

#[async_trait]
impl TalosTool for ClipboardWriteTool {
    fn name(&self) -> &'static str {
        "clipboard_write"
    }
    fn description(&self) -> &'static str {
        "Write text to the clipboard"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "text",
            "string",
            "Text to copy to clipboard",
            true,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let text = args
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing text".to_string()))?;

        #[cfg(target_os = "macos")]
        {
            let script = format!(
                r#"set the clipboard to "{}""#,
                crate::sanitize_applescript(text)
            );
            crate::run_applescript(&script)?;
            Ok(format!("Copied {} chars to clipboard", text.len()))
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = text;
            Ok("Clipboard write not available on this platform".to_string())
        }
    }
}

/// Open an application
pub struct OpenAppTool;

#[async_trait]
impl TalosTool for OpenAppTool {
    fn name(&self) -> &'static str {
        "open_app"
    }
    fn description(&self) -> &'static str {
        "Open/launch a macOS application"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "name",
            "string",
            "Application name (e.g. 'Safari', 'Terminal')",
            true,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing name".to_string()))?;

        #[cfg(target_os = "macos")]
        {
            let script = format!(
                r#"tell application "{}" to activate"#,
                crate::sanitize_applescript(name)
            );
            crate::run_applescript(&script)?;
            Ok(format!("Opened {}", name))
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = name;
            Ok("open_app only available on macOS".to_string())
        }
    }
}

/// Quit an application
pub struct QuitAppTool;

#[async_trait]
impl TalosTool for QuitAppTool {
    fn name(&self) -> &'static str {
        "quit_app"
    }
    fn description(&self) -> &'static str {
        "Quit a macOS application"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "name",
            "string",
            "Application name to quit",
            true,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing name".to_string()))?;

        #[cfg(target_os = "macos")]
        {
            let script = format!(
                r#"tell application "{}" to quit"#,
                crate::sanitize_applescript(name)
            );
            crate::run_applescript(&script)?;
            Ok(format!("Quit {}", name))
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = name;
            Ok("quit_app only available on macOS".to_string())
        }
    }
}

/// Get system volume
pub struct VolumeGetTool;

#[async_trait]
impl TalosTool for VolumeGetTool {
    fn name(&self) -> &'static str {
        "volume_get"
    }
    fn description(&self) -> &'static str {
        "Get the current system audio volume (0-100)"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            crate::run_applescript("output volume of (get volume settings)")
        }

        #[cfg(not(target_os = "macos"))]
        {
            Ok("Volume control only available on macOS".to_string())
        }
    }
}

/// Set system volume
pub struct VolumeSetTool;

#[async_trait]
impl TalosTool for VolumeSetTool {
    fn name(&self) -> &'static str {
        "volume_set"
    }
    fn description(&self) -> &'static str {
        "Set the system audio volume (0-100)"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "level",
            "integer",
            "Volume level 0-100",
            true,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let level = args
            .get("level")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| Error::Tool("Missing level".to_string()))?;

        if level > 100 {
            return Err(Error::Tool("Volume must be 0-100".to_string()));
        }

        #[cfg(target_os = "macos")]
        {
            let script = format!("set volume output volume {}", level);
            crate::run_applescript(&script)?;
            Ok(format!("Volume set to {}", level))
        }

        #[cfg(not(target_os = "macos"))]
        {
            Ok(format!("Volume set (simulated): {}", level))
        }
    }
}

/// Capture a screenshot
pub struct ScreenshotTool;

#[async_trait]
impl TalosTool for ScreenshotTool {
    fn name(&self) -> &'static str {
        "screenshot"
    }
    fn description(&self) -> &'static str {
        "Capture a screenshot and save to file"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "path",
                "string",
                "Output file path (default: ~/Desktop/screenshot.png)",
                false,
            )
            .with_param(
                "type",
                "string",
                "Capture type: 'screen', 'window', or 'selection' (default: screen)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let default_path = dirs::desktop_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
            .join(format!(
                "screenshot-{}.png",
                chrono::Utc::now().format("%Y%m%d-%H%M%S")
            ));

        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| default_path.to_str().unwrap_or("/tmp/screenshot.png"));

        let capture_type = args
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("screen");

        let flag = match capture_type {
            "window" => "-w",
            "selection" => "-s",
            _ => "",
        };

        let output = tokio::process::Command::new("screencapture")
            .arg(flag)
            .arg(path)
            .output()
            .await
            .map_err(|e| Error::Tool(format!("Screenshot failed: {}", e)))?;

        if output.status.success() {
            Ok(format!("Screenshot saved to {}", path))
        } else {
            Err(Error::Tool(format!(
                "Screenshot failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )))
        }
    }
}

/// Reveal file in Finder
pub struct FinderRevealTool;

#[async_trait]
impl TalosTool for FinderRevealTool {
    fn name(&self) -> &'static str {
        "finder_reveal"
    }
    fn description(&self) -> &'static str {
        "Reveal a file or folder in Finder"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "path",
            "string",
            "File or folder path to reveal",
            true,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing path".to_string()))?;

        #[cfg(target_os = "macos")]
        {
            let script = format!(
                r#"tell application "Finder" to reveal POSIX file "{}""#,
                crate::sanitize_applescript(path)
            );
            crate::run_applescript(&script)?;
            crate::run_applescript(r#"tell application "Finder" to activate"#)?;
            Ok(format!("Revealed {} in Finder", path))
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = path;
            Ok("Finder reveal only available on macOS".to_string())
        }
    }
}

/// Spotlight search
pub struct SpotlightSearchTool;

#[async_trait]
impl TalosTool for SpotlightSearchTool {
    fn name(&self) -> &'static str {
        "spotlight_search"
    }
    fn description(&self) -> &'static str {
        "Search for files using Spotlight (mdfind)"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("query", "string", "Search query", true)
            .with_param("limit", "integer", "Max results (default 20)", false)
            .with_param(
                "directory",
                "string",
                "Limit search to directory (optional)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing query".to_string()))?;

        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20);

        let mut cmd = tokio::process::Command::new("mdfind");
        cmd.arg(query);

        if let Some(dir) = args.get("directory").and_then(|v| v.as_str()) {
            cmd.arg("-onlyin").arg(dir);
        }

        let output = cmd
            .output()
            .await
            .map_err(|e| Error::Tool(format!("Spotlight search failed: {}", e)))?;

        if output.status.success() {
            let results = String::from_utf8_lossy(&output.stdout);
            let limited: String = results
                .lines()
                .take(limit as usize)
                .collect::<Vec<_>>()
                .join("\n");
            Ok(limited)
        } else {
            Err(Error::Tool(format!(
                "Spotlight search failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )))
        }
    }
}

/// Get/set dark mode
pub struct DarkModeTool;

#[async_trait]
impl TalosTool for DarkModeTool {
    fn name(&self) -> &'static str {
        "system_appearance"
    }
    fn description(&self) -> &'static str {
        "Get or set macOS dark mode appearance"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "mode",
            "string",
            "Set mode: 'dark', 'light', or 'toggle'. Omit to get current.",
            false,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            if let Some(mode) = args.get("mode").and_then(|v| v.as_str()) {
                let dark = match mode {
                    "dark" => "true",
                    "light" => "false",
                    "toggle" => "not dark mode",
                    _ => {
                        return Err(Error::Tool(
                            "mode must be 'dark', 'light', or 'toggle'".to_string(),
                        ));
                    }
                };
                let script = format!(
                    r#"tell application "System Events" to tell appearance preferences to set dark mode to {}"#,
                    dark
                );
                crate::run_applescript(&script)?;
                Ok(format!("Appearance set to {}", mode))
            } else {
                let result = crate::run_applescript(
                    r#"tell application "System Events" to tell appearance preferences to return dark mode"#,
                )?;
                let mode = if result.trim() == "true" {
                    "dark"
                } else {
                    "light"
                };
                Ok(format!("Current appearance: {}", mode))
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Appearance control only available on macOS".to_string())
        }
    }
}

/// Get memory information
pub struct MemoryInfoTool;

#[async_trait]
impl TalosTool for MemoryInfoTool {
    fn name(&self) -> &'static str {
        "memory_info"
    }
    fn description(&self) -> &'static str {
        "Get detailed memory information"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        let mut sys = System::new_all();
        sys.refresh_all();

        let total = sys.total_memory() as f64 / 1024.0 / 1024.0 / 1024.0;
        let used = sys.used_memory() as f64 / 1024.0 / 1024.0 / 1024.0;
        let available = total - used;
        let used_percent = if total > 0.0 {
            (used / total) * 100.0
        } else {
            0.0
        };

        let total_swap = sys.total_swap() as f64 / 1024.0 / 1024.0 / 1024.0;
        let used_swap = sys.used_swap() as f64 / 1024.0 / 1024.0 / 1024.0;

        let info = json!({
            "total_gb": format!("{:.2}", total),
            "used_gb": format!("{:.2}", used),
            "available_gb": format!("{:.2}", available),
            "used_percent": format!("{:.1}%", used_percent),
            "swap_total_gb": format!("{:.2}", total_swap),
            "swap_used_gb": format!("{:.2}", used_swap),
        });

        Ok(serde_json::to_string_pretty(&info)?)
    }
}

/// Get battery percentage and charging state
pub struct BatteryInfoTool;

#[async_trait]
impl TalosTool for BatteryInfoTool {
    fn name(&self) -> &'static str {
        "battery_info"
    }
    fn description(&self) -> &'static str {
        "Get battery percentage and charging state"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let script = r#"do shell script "pmset -g batt""#;
            let output = crate::run_applescript(script)?;
            let mut percentage = String::from("unknown");
            let mut charging_state = String::from("unknown");

            for line in output.lines() {
                let trimmed = line.trim();
                if let Some(pct_idx) = trimmed.find('%') {
                    // Walk backwards from '%' to find the start of the number
                    let before = &trimmed[..pct_idx];
                    if let Some(num_start) = before.rfind(|c: char| !c.is_ascii_digit()) {
                        percentage = before[num_start + 1..].to_string();
                    } else {
                        percentage = before.to_string();
                    }
                    // Charging state follows the percentage, e.g. "50%; charging;"
                    if trimmed.contains("charging")
                        && !trimmed.contains("discharging")
                        && !trimmed.contains("not charging")
                    {
                        charging_state = "charging".to_string();
                    } else if trimmed.contains("discharging") {
                        charging_state = "discharging".to_string();
                    } else if trimmed.contains("charged") || trimmed.contains("finishing charge") {
                        charging_state = "charged".to_string();
                    } else if trimmed.contains("AC attached") || trimmed.contains("not charging") {
                        charging_state = "not charging".to_string();
                    }
                }
            }

            let info = json!({
                "percentage": percentage,
                "state": charging_state,
                "raw": output,
            });
            Ok(serde_json::to_string_pretty(&info)?)
        }

        #[cfg(not(target_os = "macos"))]
        {
            Ok("battery_info only available on macOS".to_string())
        }
    }
}

/// Get current WiFi network name
pub struct WifiCurrentTool;

#[async_trait]
impl TalosTool for WifiCurrentTool {
    fn name(&self) -> &'static str {
        "wifi_current"
    }
    fn description(&self) -> &'static str {
        "Get the current WiFi network name"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let output = tokio::process::Command::new("networksetup")
                .args(["-getairportnetwork", "en0"])
                .output()
                .await
                .map_err(|e| Error::Tool(format!("Failed to get WiFi info: {}", e)))?;

            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
                // Output is like "Current Wi-Fi Network: MyNetwork"
                let network = text.split(": ").nth(1).unwrap_or("Not connected");
                let info = json!({
                    "network": network,
                    "raw": text,
                });
                Ok(serde_json::to_string_pretty(&info)?)
            } else {
                Err(Error::Tool(format!(
                    "Failed to get WiFi network: {}",
                    String::from_utf8_lossy(&output.stderr)
                )))
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            Ok("wifi_current only available on macOS".to_string())
        }
    }
}

/// List available WiFi networks
pub struct WifiListTool;

#[async_trait]
impl TalosTool for WifiListTool {
    fn name(&self) -> &'static str {
        "wifi_list"
    }
    fn description(&self) -> &'static str {
        "List available WiFi networks"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let output = tokio::process::Command::new(
                "/System/Library/PrivateFrameworks/Apple80211.framework/Resources/airport",
            )
            .arg("-s")
            .output()
            .await
            .map_err(|e| Error::Tool(format!("Failed to scan WiFi: {}", e)))?;

            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout).to_string();
                Ok(text)
            } else {
                Err(Error::Tool(format!(
                    "WiFi scan failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                )))
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            Ok("wifi_list only available on macOS".to_string())
        }
    }
}

/// Connect to a WiFi network
pub struct WifiConnectTool;

#[async_trait]
impl TalosTool for WifiConnectTool {
    fn name(&self) -> &'static str {
        "wifi_connect"
    }
    fn description(&self) -> &'static str {
        "Connect to a WiFi network"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("network", "string", "WiFi network name (SSID)", true)
            .with_param(
                "password",
                "string",
                "WiFi password (optional for open networks)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let network = args
            .get("network")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing network".to_string()))?;

        #[cfg(target_os = "macos")]
        {
            let mut cmd = tokio::process::Command::new("networksetup");
            cmd.arg("-setairportnetwork").arg("en0").arg(network);

            if let Some(password) = args.get("password").and_then(|v| v.as_str()) {
                cmd.arg(password);
            }

            let output = cmd
                .output()
                .await
                .map_err(|e| Error::Tool(format!("Failed to connect to WiFi: {}", e)))?;

            if output.status.success() {
                Ok(format!("Connected to WiFi network: {}", network))
            } else {
                Err(Error::Tool(format!(
                    "Failed to connect to {}: {}",
                    network,
                    String::from_utf8_lossy(&output.stderr)
                )))
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = network;
            Ok("wifi_connect only available on macOS".to_string())
        }
    }
}

/// List paired Bluetooth devices
pub struct BluetoothListTool;

#[async_trait]
impl TalosTool for BluetoothListTool {
    fn name(&self) -> &'static str {
        "bluetooth_list"
    }
    fn description(&self) -> &'static str {
        "List paired Bluetooth devices"
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
                // Parse the JSON to extract device names and connection status
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

/// Toggle Bluetooth on or off
pub struct BluetoothToggleTool;

#[async_trait]
impl TalosTool for BluetoothToggleTool {
    fn name(&self) -> &'static str {
        "bluetooth_toggle"
    }
    fn description(&self) -> &'static str {
        "Turn Bluetooth on or off"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "enabled",
            "boolean",
            "true to enable, false to disable",
            true,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let enabled = args
            .get("enabled")
            .and_then(|v| v.as_bool())
            .ok_or_else(|| Error::Tool("Missing enabled".to_string()))?;

        #[cfg(target_os = "macos")]
        {
            let power_val = if enabled { "1" } else { "0" };
            let output = tokio::process::Command::new("blueutil")
                .args(["--power", power_val])
                .output()
                .await
                .map_err(|e| {
                    Error::Tool(format!(
                        "Failed to toggle Bluetooth (is blueutil installed?): {}",
                        e
                    ))
                })?;

            if output.status.success() {
                let state = if enabled { "on" } else { "off" };
                Ok(format!("Bluetooth turned {}", state))
            } else {
                Err(Error::Tool(format!(
                    "Bluetooth toggle failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                )))
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = enabled;
            Ok("bluetooth_toggle only available on macOS".to_string())
        }
    }
}

/// Set desktop wallpaper
pub struct SetWallpaperTool;

#[async_trait]
impl TalosTool for SetWallpaperTool {
    fn name(&self) -> &'static str {
        "set_wallpaper"
    }
    fn description(&self) -> &'static str {
        "Set the desktop wallpaper image"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "path",
            "string",
            "Absolute path to the wallpaper image file",
            true,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing path".to_string()))?;

        #[cfg(target_os = "macos")]
        {
            let script = format!(
                r#"tell application "System Events" to tell every desktop to set picture to POSIX file "{}""#,
                crate::sanitize_applescript(path)
            );
            crate::run_applescript(&script)?;
            Ok(format!("Wallpaper set to {}", path))
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = path;
            Ok("set_wallpaper only available on macOS".to_string())
        }
    }
}

/// Lock the screen immediately
pub struct ScreenLockTool;

#[async_trait]
impl TalosTool for ScreenLockTool {
    fn name(&self) -> &'static str {
        "screen_lock"
    }
    fn description(&self) -> &'static str {
        "Lock the screen immediately"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let output = tokio::process::Command::new("pmset")
                .arg("displaysleepnow")
                .output()
                .await
                .map_err(|e| Error::Tool(format!("Failed to lock screen: {}", e)))?;

            if output.status.success() {
                Ok("Screen locked".to_string())
            } else {
                Err(Error::Tool(format!(
                    "Screen lock failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                )))
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            Ok("screen_lock only available on macOS".to_string())
        }
    }
}

/// Get or set screen brightness
pub struct ScreenBrightnessTool;

#[async_trait]
impl TalosTool for ScreenBrightnessTool {
    fn name(&self) -> &'static str {
        "screen_brightness"
    }
    fn description(&self) -> &'static str {
        "Get or set screen brightness (0.0 to 1.0)"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "level",
            "number",
            "Brightness level 0.0-1.0 (omit to get current)",
            false,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            if let Some(level) = args.get("level").and_then(|v| v.as_f64()) {
                if !(0.0..=1.0).contains(&level) {
                    return Err(Error::Tool(
                        "Brightness level must be between 0.0 and 1.0".to_string(),
                    ));
                }
                let output = tokio::process::Command::new("brightness")
                    .arg(format!("{:.2}", level))
                    .output()
                    .await
                    .map_err(|e| {
                        Error::Tool(format!(
                            "Failed to set brightness (is `brightness` CLI installed?): {}",
                            e
                        ))
                    })?;

                if output.status.success() {
                    Ok(format!("Brightness set to {:.0}%", level * 100.0))
                } else {
                    Err(Error::Tool(format!(
                        "Failed to set brightness: {}",
                        String::from_utf8_lossy(&output.stderr)
                    )))
                }
            } else {
                let output = tokio::process::Command::new("brightness")
                    .arg("-l")
                    .output()
                    .await
                    .map_err(|e| {
                        Error::Tool(format!(
                            "Failed to get brightness (is `brightness` CLI installed?): {}",
                            e
                        ))
                    })?;

                if output.status.success() {
                    let text = String::from_utf8_lossy(&output.stdout).to_string();
                    // Parse brightness value from output
                    let mut current = String::from("unknown");
                    for line in text.lines() {
                        if line.contains("brightness")
                            && let Some(val) = line.split_whitespace().last()
                        {
                            current = val.to_string();
                        }
                    }
                    let info = json!({
                        "brightness": current,
                        "raw": text.trim(),
                    });
                    Ok(serde_json::to_string_pretty(&info)?)
                } else {
                    Err(Error::Tool(format!(
                        "Failed to get brightness: {}",
                        String::from_utf8_lossy(&output.stderr)
                    )))
                }
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("screen_brightness only available on macOS".to_string())
        }
    }
}

/// Get or set Do Not Disturb / Focus mode
pub struct DoNotDisturbTool;

#[async_trait]
impl TalosTool for DoNotDisturbTool {
    fn name(&self) -> &'static str {
        "do_not_disturb"
    }
    fn description(&self) -> &'static str {
        "Get or set Do Not Disturb / Focus mode"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "enabled",
            "boolean",
            "true to enable DND, false to disable (omit to get current state)",
            false,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            if let Some(enabled) = args.get("enabled").and_then(|v| v.as_bool()) {
                // Use defaults to toggle Focus/DND via the notification center plist
                let value = if enabled { "true" } else { "false" };
                let output = tokio::process::Command::new("defaults")
                    .args([
                        "write",
                        "com.apple.controlcenter",
                        "NSStatusItem Visible FocusModes",
                        "-bool",
                        value,
                    ])
                    .output()
                    .await
                    .map_err(|e| Error::Tool(format!("Failed to set DND: {}", e)))?;

                if output.status.success() {
                    let state = if enabled { "enabled" } else { "disabled" };
                    Ok(format!("Do Not Disturb {}", state))
                } else {
                    Err(Error::Tool(format!(
                        "Failed to set DND: {}",
                        String::from_utf8_lossy(&output.stderr)
                    )))
                }
            } else {
                // Read current state
                let output = tokio::process::Command::new("defaults")
                    .args([
                        "read",
                        "com.apple.controlcenter",
                        "NSStatusItem Visible FocusModes",
                    ])
                    .output()
                    .await
                    .map_err(|e| Error::Tool(format!("Failed to read DND state: {}", e)))?;

                let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let is_enabled = text == "1" || text.to_lowercase() == "true";
                let info = json!({
                    "enabled": is_enabled,
                    "raw": text,
                });
                Ok(serde_json::to_string_pretty(&info)?)
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("do_not_disturb only available on macOS".to_string())
        }
    }
}

/// List all installed applications
pub struct AppListTool;

#[async_trait]
impl TalosTool for AppListTool {
    fn name(&self) -> &'static str {
        "app_list"
    }
    fn description(&self) -> &'static str {
        "List all installed applications"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let mut apps = Vec::new();

            // List /Applications
            let output = tokio::process::Command::new("ls")
                .arg("/Applications")
                .output()
                .await
                .map_err(|e| Error::Tool(format!("Failed to list /Applications: {}", e)))?;

            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout);
                for line in text.lines() {
                    let name = line.trim();
                    if !name.is_empty() {
                        apps.push(json!({
                            "name": name.strip_suffix(".app").unwrap_or(name),
                            "path": format!("/Applications/{}", name),
                        }));
                    }
                }
            }

            // List ~/Applications
            if let Some(home) = dirs::home_dir() {
                let user_apps = home.join("Applications");
                if user_apps.exists() {
                    let output = tokio::process::Command::new("ls")
                        .arg(user_apps.to_str().unwrap_or("~/Applications"))
                        .output()
                        .await
                        .map_err(|e| {
                            Error::Tool(format!("Failed to list ~/Applications: {}", e))
                        })?;

                    if output.status.success() {
                        let text = String::from_utf8_lossy(&output.stdout);
                        for line in text.lines() {
                            let name = line.trim();
                            if !name.is_empty() {
                                apps.push(json!({
                                    "name": name.strip_suffix(".app").unwrap_or(name),
                                    "path": format!("{}/{}", user_apps.display(), name),
                                }));
                            }
                        }
                    }
                }
            }

            let info = json!({
                "count": apps.len(),
                "applications": apps,
            });
            Ok(serde_json::to_string_pretty(&info)?)
        }

        #[cfg(not(target_os = "macos"))]
        {
            Ok("app_list only available on macOS".to_string())
        }
    }
}

/// Get the frontmost application name
pub struct FrontAppTool;

#[async_trait]
impl TalosTool for FrontAppTool {
    fn name(&self) -> &'static str {
        "front_app"
    }
    fn description(&self) -> &'static str {
        "Get the name of the frontmost application"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let result = crate::run_applescript(
                r#"tell application "System Events" to get name of first application process whose frontmost is true"#,
            )?;
            let info = json!({
                "frontmost_app": result.trim(),
            });
            Ok(serde_json::to_string_pretty(&info)?)
        }

        #[cfg(not(target_os = "macos"))]
        {
            Ok("front_app only available on macOS".to_string())
        }
    }
}

/// Hide an application
pub struct HideAppTool;

#[async_trait]
impl TalosTool for HideAppTool {
    fn name(&self) -> &'static str {
        "hide_app"
    }
    fn description(&self) -> &'static str {
        "Hide an application by name"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "name",
            "string",
            "Application name to hide",
            true,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing name".to_string()))?;

        #[cfg(target_os = "macos")]
        {
            let script = format!(
                r#"tell application "System Events" to set visible of process "{}" to false"#,
                crate::sanitize_applescript(name)
            );
            crate::run_applescript(&script)?;
            Ok(format!("Hidden {}", name))
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = name;
            Ok("hide_app only available on macOS".to_string())
        }
    }
}

/// Mute or unmute system audio
pub struct MuteToggleTool;

#[async_trait]
impl TalosTool for MuteToggleTool {
    fn name(&self) -> &'static str {
        "mute_toggle"
    }
    fn description(&self) -> &'static str {
        "Mute, unmute, or toggle system audio"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "muted",
            "boolean",
            "true to mute, false to unmute (omit to toggle)",
            false,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            if let Some(muted) = args.get("muted").and_then(|v| v.as_bool()) {
                let script = if muted {
                    "set volume with output muted"
                } else {
                    "set volume without output muted"
                };
                crate::run_applescript(script)?;
                let state = if muted { "muted" } else { "unmuted" };
                Ok(format!("Audio {}", state))
            } else {
                // Toggle: read current state and flip it
                let current = crate::run_applescript("output muted of (get volume settings)")?;
                let is_muted = current.trim() == "true";
                let script = if is_muted {
                    "set volume without output muted"
                } else {
                    "set volume with output muted"
                };
                crate::run_applescript(script)?;
                let state = if is_muted { "unmuted" } else { "muted" };
                Ok(format!("Audio toggled to {}", state))
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("mute_toggle only available on macOS".to_string())
        }
    }
}

// === SYSTEM ADDITIONS ===

/// List or get environment variables
pub struct EnvVarsTool;

#[async_trait]
impl TalosTool for EnvVarsTool {
    fn name(&self) -> &'static str {
        "env_vars"
    }
    fn description(&self) -> &'static str {
        "List or get environment variables"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "name",
            "string",
            "Specific variable name to get (omit to list all)",
            false,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        if let Some(name) = args.get("name").and_then(|v| v.as_str()) {
            match std::env::var(name) {
                Ok(value) => {
                    let info = json!({ name: value });
                    Ok(serde_json::to_string_pretty(&info)?)
                }
                Err(_) => Err(Error::Tool(format!(
                    "Environment variable not found: {}",
                    name
                ))),
            }
        } else {
            let vars: serde_json::Map<String, Value> = std::env::vars()
                .map(|(k, v)| (k, Value::String(v)))
                .collect();
            Ok(serde_json::to_string_pretty(&vars)?)
        }
    }
}

/// Check file permissions
pub struct CheckPermissionsTool;

#[async_trait]
impl TalosTool for CheckPermissionsTool {
    fn name(&self) -> &'static str {
        "check_permissions"
    }
    fn description(&self) -> &'static str {
        "Check file permissions (readable, writable, executable)"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "path",
            "string",
            "Path to file or directory",
            true,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing path".to_string()))?;

        let p = std::path::Path::new(path);
        if !p.exists() {
            return Err(Error::Tool(format!("Path not found: {}", path)));
        }

        let readable = std::fs::OpenOptions::new().read(true).open(p).is_ok();
        let writable = std::fs::OpenOptions::new().write(true).open(p).is_ok();

        // Check executable via metadata permissions on unix
        #[cfg(unix)]
        let executable = {
            use std::os::unix::fs::PermissionsExt;
            std::fs::metadata(p)
                .map(|m| m.permissions().mode() & 0o111 != 0)
                .unwrap_or(false)
        };
        #[cfg(not(unix))]
        let executable = false;

        let info = json!({
            "path": path,
            "readable": readable,
            "writable": writable,
            "executable": executable,
        });

        Ok(serde_json::to_string_pretty(&info)?)
    }
}

/// List macOS Shortcuts
pub struct ListShortcutsTool;

#[async_trait]
impl TalosTool for ListShortcutsTool {
    fn name(&self) -> &'static str {
        "list_shortcuts"
    }
    fn description(&self) -> &'static str {
        "List available macOS Shortcuts"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let output = tokio::process::Command::new("shortcuts")
                .arg("list")
                .output()
                .await
                .map_err(|e| Error::Tool(format!("Failed to list shortcuts: {}", e)))?;

            if output.status.success() {
                let result = String::from_utf8_lossy(&output.stdout).trim().to_string();
                Ok(result)
            } else {
                Err(Error::Tool(format!(
                    "shortcuts list failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                )))
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            Ok("list_shortcuts only available on macOS".to_string())
        }
    }
}

/// Run a macOS Shortcut
pub struct RunShortcutTool;

#[async_trait]
impl TalosTool for RunShortcutTool {
    fn name(&self) -> &'static str {
        "run_shortcut"
    }
    fn description(&self) -> &'static str {
        "Run a macOS Shortcut by name"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("name", "string", "Name of the shortcut to run", true)
            .with_param(
                "input",
                "string",
                "Input to pass to the shortcut (optional)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing name".to_string()))?;

        #[cfg(target_os = "macos")]
        {
            let mut cmd = tokio::process::Command::new("shortcuts");
            cmd.arg("run");
            cmd.arg(name);

            if let Some(input) = args.get("input").and_then(|v| v.as_str()) {
                cmd.arg("--input-path").arg(input);
            }

            let output = cmd
                .output()
                .await
                .map_err(|e| Error::Tool(format!("Failed to run shortcut: {}", e)))?;

            if output.status.success() {
                let result = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if result.is_empty() {
                    Ok(format!("Shortcut '{}' executed successfully", name))
                } else {
                    Ok(result)
                }
            } else {
                Err(Error::Tool(format!(
                    "Shortcut '{}' failed: {}",
                    name,
                    String::from_utf8_lossy(&output.stderr)
                )))
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = name;
            Ok("run_shortcut only available on macOS".to_string())
        }
    }
}

/// Screenshot a specific screen region
pub struct ScreenshotRegionTool;

#[async_trait]
impl TalosTool for ScreenshotRegionTool {
    fn name(&self) -> &'static str {
        "screenshot_region"
    }
    fn description(&self) -> &'static str {
        "Capture a screenshot of a specific screen region"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("x", "integer", "X coordinate of the region", true)
            .with_param("y", "integer", "Y coordinate of the region", true)
            .with_param("width", "integer", "Width of the region", true)
            .with_param("height", "integer", "Height of the region", true)
            .with_param(
                "output",
                "string",
                "Output file path (optional, defaults to temp file)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let x = args
            .get("x")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| Error::Tool("Missing x".to_string()))?;
        let y = args
            .get("y")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| Error::Tool("Missing y".to_string()))?;
        let width = args
            .get("width")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| Error::Tool("Missing width".to_string()))?;
        let height = args
            .get("height")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| Error::Tool("Missing height".to_string()))?;

        let default_output = format!(
            "/tmp/screenshot-region-{}.png",
            chrono::Utc::now().format("%Y%m%d-%H%M%S")
        );
        let output = args
            .get("output")
            .and_then(|v| v.as_str())
            .unwrap_or(&default_output);

        #[cfg(target_os = "macos")]
        {
            let region = format!("{},{},{},{}", x, y, width, height);
            let cmd_output = tokio::process::Command::new("screencapture")
                .arg("-R")
                .arg(&region)
                .arg(output)
                .output()
                .await
                .map_err(|e| Error::Tool(format!("Screenshot failed: {}", e)))?;

            if cmd_output.status.success() {
                Ok(format!("Screenshot saved to {}", output))
            } else {
                Err(Error::Tool(format!(
                    "Screenshot failed: {}",
                    String::from_utf8_lossy(&cmd_output.stderr)
                )))
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = (x, y, width, height, output);
            Ok("screenshot_region only available on macOS".to_string())
        }
    }
}

/// Execute arbitrary AppleScript
pub struct ExecuteApplescriptTool;

/// Validate an AppleScript for dangerous patterns before execution.
///
/// Blocks scripts that attempt to:
/// - Execute shell commands via `do shell script` with destructive operations
/// - Access sensitive files through shell escapes
fn validate_applescript(script: &str) -> Result<()> {
    if script.contains('\0') {
        return Err(Error::Tool("AppleScript contains null bytes".to_string()));
    }

    if script.len() > 50_000 {
        return Err(Error::Tool(
            "AppleScript too long (max 50,000 characters)".to_string(),
        ));
    }

    // Detect `do shell script` with destructive payloads
    let lower = script.to_lowercase();
    if lower.contains("do shell script") {
        let dangerous_patterns = [
            "rm -rf /",
            "/etc/shadow",
            "/etc/master.passwd",
            "mkfs",
            "dd if=",
            "> /dev/",
            "curl.*|.*sh",
            "wget.*|.*sh",
        ];
        for pattern in &dangerous_patterns {
            if lower.contains(pattern) {
                return Err(Error::Tool(format!(
                    "AppleScript blocked: dangerous shell pattern detected ({})",
                    pattern
                )));
            }
        }
    }

    Ok(())
}

#[async_trait]
impl TalosTool for ExecuteApplescriptTool {
    fn name(&self) -> &'static str {
        "execute_applescript"
    }
    fn description(&self) -> &'static str {
        "Execute arbitrary AppleScript code"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "script",
            "string",
            "AppleScript code to execute",
            true,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let script = args
            .get("script")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing script".to_string()))?;

        validate_applescript(script)?;

        #[cfg(target_os = "macos")]
        {
            crate::run_applescript(script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = script;
            Ok("execute_applescript only available on macOS".to_string())
        }
    }
}

/// Run a shell command with retries
pub struct RetryCommandTool;

#[async_trait]
impl TalosTool for RetryCommandTool {
    fn name(&self) -> &'static str {
        "retry_command"
    }
    fn description(&self) -> &'static str {
        "Run a shell command with automatic retries on failure"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("command", "string", "Shell command to execute", true)
            .with_param(
                "retries",
                "integer",
                "Number of retry attempts (default 3)",
                false,
            )
            .with_param(
                "delay_ms",
                "integer",
                "Delay between retries in milliseconds (default 1000)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing command".to_string()))?;

        let retries = args.get("retries").and_then(|v| v.as_u64()).unwrap_or(3) as usize;

        let delay_ms = args
            .get("delay_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(1000);

        let mut last_error = String::new();

        for attempt in 0..=retries {
            // Safe: parse command into argv, no shell invocation
            let mut rc_argv: Vec<&str> = command.split_whitespace().collect();
            if rc_argv.is_empty() {
                return Err(Error::Tool("Empty command".to_string()));
            }
            let rc_prog = rc_argv.remove(0);
            let output = tokio::process::Command::new(rc_prog)
                .args(&rc_argv)
                .output()
                .await
                .map_err(|e| Error::Tool(format!("Failed to execute command: {}", e)))?;

            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                return Ok(json!({
                    "success": true,
                    "attempt": attempt + 1,
                    "output": stdout,
                })
                .to_string());
            }

            last_error = String::from_utf8_lossy(&output.stderr).trim().to_string();

            if attempt < retries {
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            }
        }

        Err(Error::Tool(format!(
            "Command failed after {} attempts. Last error: {}",
            retries + 1,
            last_error
        )))
    }
}

/// Check if a macOS service/daemon is running
pub struct ServiceStatusTool;

#[async_trait]
impl TalosTool for ServiceStatusTool {
    fn name(&self) -> &'static str {
        "service_status"
    }
    fn description(&self) -> &'static str {
        "Check if a macOS service or daemon is running"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "service",
            "string",
            "Service name to check",
            true,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let service = args
            .get("service")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing service".to_string()))?;

        #[cfg(target_os = "macos")]
        {
            // Safe: run launchctl list directly, filter in Rust — no sh -c needed
            let lc_output = tokio::process::Command::new("launchctl")
                .arg("list")
                .output()
                .await
                .map_err(|e| Error::Tool(format!("Failed to check service: {}", e)))?;
            let stdout: String = String::from_utf8_lossy(&lc_output.stdout)
                .lines()
                .filter(|line| line.contains(service))
                .collect::<Vec<_>>()
                .join("\n");
            if stdout.is_empty() {
                Ok(json!({
                    "service": service,
                    "running": false,
                    "details": "Service not found in launchctl list",
                })
                .to_string())
            } else {
                Ok(json!({
                    "service": service,
                    "running": true,
                    "details": stdout,
                })
                .to_string())
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = service;
            Ok("service_status only available on macOS".to_string())
        }
    }
}

/// Text-to-speech
pub struct SpeakTextTool;

#[async_trait]
impl TalosTool for SpeakTextTool {
    fn name(&self) -> &'static str {
        "speak_text"
    }
    fn description(&self) -> &'static str {
        "Convert text to speech using system TTS"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("text", "string", "Text to speak", true)
            .with_param(
                "voice",
                "string",
                "Voice name (optional, e.g. 'Samantha')",
                false,
            )
            .with_param(
                "rate",
                "integer",
                "Speech rate in words per minute (optional)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let text = args
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing text".to_string()))?;

        #[cfg(target_os = "macos")]
        {
            let mut cmd = tokio::process::Command::new("say");
            cmd.arg(text);

            if let Some(voice) = args.get("voice").and_then(|v| v.as_str()) {
                cmd.arg("-v").arg(voice);
            }

            if let Some(rate) = args.get("rate").and_then(|v| v.as_u64()) {
                cmd.arg("-r").arg(rate.to_string());
            }

            let output = cmd
                .output()
                .await
                .map_err(|e| Error::Tool(format!("Failed to speak text: {}", e)))?;

            if output.status.success() {
                Ok(format!("Spoke: {}", text))
            } else {
                Err(Error::Tool(format!(
                    "Speech failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                )))
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            // Try espeak as a fallback on Linux
            let mut cmd = tokio::process::Command::new("espeak");
            cmd.arg(text);

            if let Some(rate) = args.get("rate").and_then(|v| v.as_u64()) {
                cmd.arg("-s").arg(rate.to_string());
            }

            match cmd.output().await {
                Ok(output) if output.status.success() => Ok(format!("Spoke: {}", text)),
                _ => Ok(format!(
                    "Speech not available on this platform. Text was: {}",
                    text
                )),
            }
        }
    }
}

/// Send OS notification with more control
pub struct SendNotificationTool;

#[async_trait]
impl TalosTool for SendNotificationTool {
    fn name(&self) -> &'static str {
        "send_notification"
    }
    fn description(&self) -> &'static str {
        "Send an OS notification with title, message, subtitle, and sound"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("title", "string", "Notification title", true)
            .with_param("message", "string", "Notification message", true)
            .with_param(
                "subtitle",
                "string",
                "Notification subtitle (optional)",
                false,
            )
            .with_param("sound", "string", "Sound name (default: 'default')", false)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let title = args
            .get("title")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing title".to_string()))?;

        let message = args
            .get("message")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("Missing message".to_string()))?;

        let subtitle = args.get("subtitle").and_then(|v| v.as_str());
        let sound = args
            .get("sound")
            .and_then(|v| v.as_str())
            .unwrap_or("default");

        #[cfg(target_os = "macos")]
        {
            let subtitle_part = if let Some(sub) = subtitle {
                format!(r#" subtitle "{}""#, crate::sanitize_applescript(sub))
            } else {
                String::new()
            };

            let script = format!(
                r#"display notification "{}" with title "{}"{} sound name "{}""#,
                crate::sanitize_applescript(message),
                crate::sanitize_applescript(title),
                subtitle_part,
                crate::sanitize_applescript(sound),
            );
            crate::run_applescript(&script)?;
            Ok(format!("Notification sent: {}", title))
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = (subtitle, sound);
            Ok(format!("Notification (console): {} - {}", title, message))
        }
    }
}

/// Explicit mute/unmute (not toggle)
pub struct SetMuteTool;

#[async_trait]
impl TalosTool for SetMuteTool {
    fn name(&self) -> &'static str {
        "set_mute"
    }
    fn description(&self) -> &'static str {
        "Explicitly mute or unmute system audio"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "muted",
            "boolean",
            "true to mute, false to unmute",
            true,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let muted = args
            .get("muted")
            .and_then(|v| v.as_bool())
            .ok_or_else(|| Error::Tool("Missing muted".to_string()))?;

        #[cfg(target_os = "macos")]
        {
            let script = if muted {
                "set volume with output muted"
            } else {
                "set volume without output muted"
            };
            crate::run_applescript(script)?;
            let state = if muted { "muted" } else { "unmuted" };
            Ok(format!("Audio {}", state))
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = muted;
            Ok("set_mute only available on macOS".to_string())
        }
    }
}

/// Enable a Focus mode (e.g. Do Not Disturb)
pub struct EnableFocusTool;

#[async_trait]
impl TalosTool for EnableFocusTool {
    fn name(&self) -> &'static str {
        "enable_focus"
    }
    fn description(&self) -> &'static str {
        "Enable a Focus mode (e.g. Do Not Disturb) on macOS"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "mode",
            "string",
            "Focus mode name (default: \"Do Not Disturb\")",
            false,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let mode = args
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("Do Not Disturb");

        #[cfg(target_os = "macos")]
        {
            // Try using the Shortcuts app first (most reliable on modern macOS)
            let shortcut_name = format!("Turn On {}", mode);
            let output = tokio::process::Command::new("shortcuts")
                .arg("run")
                .arg(&shortcut_name)
                .output()
                .await;

            match output {
                Ok(o) if o.status.success() => {
                    return Ok(format!("Focus mode '{}' enabled", mode));
                }
                _ => {
                    // Fallback: use defaults to enable DND via notification center
                    let output = tokio::process::Command::new("defaults")
                        .args([
                            "-currentHost",
                            "write",
                            "com.apple.notificationcenterui",
                            "doNotDisturb",
                            "-boolean",
                            "true",
                        ])
                        .output()
                        .await
                        .map_err(|e| Error::Tool(format!("Failed to enable focus mode: {}", e)))?;

                    if output.status.success() {
                        // Restart NotificationCenter to pick up the change
                        let _ = tokio::process::Command::new("killall")
                            .arg("NotificationCenter")
                            .output()
                            .await;
                        Ok(format!("Focus mode '{}' enabled (via defaults)", mode))
                    } else {
                        Err(Error::Tool(format!(
                            "Failed to enable focus mode: {}",
                            String::from_utf8_lossy(&output.stderr)
                        )))
                    }
                }
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = mode;
            Ok("enable_focus only available on macOS".to_string())
        }
    }
}

/// Disable a Focus mode (e.g. Do Not Disturb)
pub struct DisableFocusTool;

#[async_trait]
impl TalosTool for DisableFocusTool {
    fn name(&self) -> &'static str {
        "disable_focus"
    }
    fn description(&self) -> &'static str {
        "Disable a Focus mode (e.g. Do Not Disturb) on macOS"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "mode",
            "string",
            "Focus mode name (default: \"Do Not Disturb\")",
            false,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let mode = args
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("Do Not Disturb");

        #[cfg(target_os = "macos")]
        {
            // Try using the Shortcuts app first (most reliable on modern macOS)
            let shortcut_name = format!("Turn Off {}", mode);
            let output = tokio::process::Command::new("shortcuts")
                .arg("run")
                .arg(&shortcut_name)
                .output()
                .await;

            match output {
                Ok(o) if o.status.success() => {
                    return Ok(format!("Focus mode '{}' disabled", mode));
                }
                _ => {
                    // Fallback: use defaults to disable DND via notification center
                    let output = tokio::process::Command::new("defaults")
                        .args([
                            "-currentHost",
                            "write",
                            "com.apple.notificationcenterui",
                            "doNotDisturb",
                            "-boolean",
                            "false",
                        ])
                        .output()
                        .await
                        .map_err(|e| Error::Tool(format!("Failed to disable focus mode: {}", e)))?;

                    if output.status.success() {
                        // Restart NotificationCenter to pick up the change
                        let _ = tokio::process::Command::new("killall")
                            .arg("NotificationCenter")
                            .output()
                            .await;
                        Ok(format!("Focus mode '{}' disabled (via defaults)", mode))
                    } else {
                        Err(Error::Tool(format!(
                            "Failed to disable focus mode: {}",
                            String::from_utf8_lossy(&output.stderr)
                        )))
                    }
                }
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = mode;
            Ok("disable_focus only available on macOS".to_string())
        }
    }
}

// ── Extended system tools ────────────────────────────────────────────────

/// Get detailed CPU information
pub struct CpuInfoTool;

#[async_trait]
impl TalosTool for CpuInfoTool {
    fn name(&self) -> &'static str {
        "cpu_info"
    }
    fn description(&self) -> &'static str {
        "Get detailed CPU information (brand, cores, frequency, usage per core)"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        let mut sys = System::new_all();
        sys.refresh_all();
        // Wait briefly for accurate CPU usage
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        sys.refresh_cpu_all();

        let cpus: Vec<Value> = sys.cpus().iter().enumerate().map(|(i, cpu)| {
            json!({ "core": i, "brand": cpu.brand(), "frequency_mhz": cpu.frequency(), "usage_percent": cpu.cpu_usage() })
        }).collect();

        let info = json!({
            "brand": sys.cpus().first().map(|c| c.brand()).unwrap_or("unknown"),
            "physical_cores": sys.physical_core_count().unwrap_or(0),
            "logical_cores": sys.cpus().len(),
            "global_usage_percent": sys.global_cpu_usage(),
            "cores": cpus,
        });
        Ok(serde_json::to_string_pretty(&info)?)
    }
}

/// List connected displays/monitors
pub struct DisplayListTool;

#[async_trait]
impl TalosTool for DisplayListTool {
    fn name(&self) -> &'static str {
        "display_list"
    }
    fn description(&self) -> &'static str {
        "List connected displays/monitors with resolution and properties"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let output = tokio::process::Command::new("system_profiler")
                .args(["SPDisplaysDataType", "-json"])
                .output()
                .await
                .map_err(|e| Error::Tool(format!("Failed to get display info: {}", e)))?;
            if output.status.success() {
                Ok(String::from_utf8_lossy(&output.stdout).to_string())
            } else {
                Err(Error::Tool("Failed to get display info".to_string()))
            }
        }
        #[cfg(not(target_os = "macos"))]
        Ok("display_list only available on macOS".to_string())
    }
}

/// Get info for a specific process by PID or name
pub struct GetProcessInfoTool;

#[async_trait]
impl TalosTool for GetProcessInfoTool {
    fn name(&self) -> &'static str {
        "get_process_info"
    }
    fn description(&self) -> &'static str {
        "Get detailed info for a specific process by PID or name"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("pid", "integer", "Process ID to look up", false)
            .with_param("name", "string", "Process name to search for", false)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let mut sys = System::new_all();
        sys.refresh_all();

        let pid = args.get("pid").and_then(|v| v.as_u64());
        let name = args.get("name").and_then(|v| v.as_str());

        if pid.is_none() && name.is_none() {
            return Err(Error::Tool("Provide either 'pid' or 'name'".to_string()));
        }

        let mut results: Vec<Value> = Vec::new();
        for (p, proc_info) in sys.processes() {
            let matches = pid.map(|id| p.as_u32() == id as u32).unwrap_or(false)
                || name
                    .map(|n| {
                        proc_info
                            .name()
                            .to_string_lossy()
                            .to_lowercase()
                            .contains(&n.to_lowercase())
                    })
                    .unwrap_or(false);
            if matches {
                results.push(json!({
                    "pid": p.as_u32(),
                    "name": proc_info.name().to_string_lossy(),
                    "cpu_usage": proc_info.cpu_usage(),
                    "memory_bytes": proc_info.memory(),
                    "status": format!("{:?}", proc_info.status()),
                    "run_time_secs": proc_info.run_time(),
                    "exe": proc_info.exe().map(|e| e.to_string_lossy().to_string()).unwrap_or_default(),
                }));
            }
        }
        Ok(serde_json::to_string_pretty(
            &json!({ "processes": results, "count": results.len() }),
        )?)
    }
}

/// Check if a process is currently running
pub struct IsProcessRunningTool;

#[async_trait]
impl TalosTool for IsProcessRunningTool {
    fn name(&self) -> &'static str {
        "is_process_running"
    }
    fn description(&self) -> &'static str {
        "Check if a process is currently running by name"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "name",
            "string",
            "Process name to check",
            true,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("'name' is required".to_string()))?;

        let mut sys = System::new_all();
        sys.refresh_all();

        let matches: Vec<Value> = sys
            .processes()
            .iter()
            .filter(|(_, p)| {
                p.name()
                    .to_string_lossy()
                    .to_lowercase()
                    .contains(&name.to_lowercase())
            })
            .map(|(pid, p)| json!({ "pid": pid.as_u32(), "name": p.name().to_string_lossy() }))
            .collect();

        Ok(serde_json::to_string_pretty(&json!({
            "running": !matches.is_empty(),
            "count": matches.len(),
            "matches": matches,
        }))?)
    }
}

/// Get the title of the frontmost window
pub struct GetWindowTitleTool;

#[async_trait]
impl TalosTool for GetWindowTitleTool {
    fn name(&self) -> &'static str {
        "get_window_title"
    }
    fn description(&self) -> &'static str {
        "Get the title of the frontmost window"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let script = r#"
                tell application "System Events"
                    set frontApp to first application process whose frontmost is true
                    set appName to name of frontApp
                    try
                        set winTitle to name of front window of frontApp
                    on error
                        set winTitle to "(no window)"
                    end try
                end tell
                return appName & ": " & winTitle
            "#;
            let output = tokio::process::Command::new("osascript")
                .arg("-e")
                .arg(script)
                .output()
                .await
                .map_err(|e| Error::Tool(format!("Failed: {}", e)))?;
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
        }
        #[cfg(not(target_os = "macos"))]
        Ok("get_window_title only available on macOS".to_string())
    }
}

/// Maximize a window (zoom to fill screen)
pub struct MaximizeWindowTool;

#[async_trait]
impl TalosTool for MaximizeWindowTool {
    fn name(&self) -> &'static str {
        "maximize_window"
    }
    fn description(&self) -> &'static str {
        "Maximize/zoom the frontmost window to fill the screen"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "app",
            "string",
            "Application name (default: frontmost app)",
            false,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let app_clause = if let Some(app) = args.get("app").and_then(|v| v.as_str()) {
                format!("application process \"{}\"", app)
            } else {
                "first application process whose frontmost is true".to_string()
            };
            let script = format!(
                r#"tell application "System Events"
                    set targetApp to {}
                    tell targetApp
                        try
                            click (first button of front window whose subrole is "AXZoomButton")
                        on error
                            set position of front window to {{0, 25}}
                            set size of front window to {{1920, 1055}}
                        end try
                    end tell
                end tell"#,
                app_clause
            );
            let output = tokio::process::Command::new("osascript")
                .arg("-e")
                .arg(&script)
                .output()
                .await
                .map_err(|e| Error::Tool(format!("Failed: {}", e)))?;
            if output.status.success() {
                Ok("Window maximized".to_string())
            } else {
                Err(Error::Tool(
                    String::from_utf8_lossy(&output.stderr).to_string(),
                ))
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("maximize_window only available on macOS".to_string())
        }
    }
}

/// Switch to a specific macOS Mission Control space
pub struct SwitchSpaceTool;

#[async_trait]
impl TalosTool for SwitchSpaceTool {
    fn name(&self) -> &'static str {
        "switch_space"
    }
    fn description(&self) -> &'static str {
        "Switch to a specific Mission Control desktop space by number"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "space",
            "integer",
            "Space number to switch to (1-based)",
            true,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let space =
            args.get("space")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| Error::Tool("'space' is required".to_string()))? as u32;

        #[cfg(target_os = "macos")]
        {
            // Use Ctrl+<number> keyboard shortcut (requires Mission Control keyboard shortcuts enabled)
            let script = format!(
                r#"tell application "System Events" to key code {} using control down"#,
                match space {
                    1 => 18,
                    2 => 19,
                    3 => 20,
                    4 => 21,
                    5 => 23,
                    6 => 22,
                    7 => 26,
                    8 => 28,
                    9 => 25,
                    _ => return Err(Error::Tool("Space number must be 1-9".to_string())),
                }
            );
            let output = tokio::process::Command::new("osascript")
                .arg("-e")
                .arg(&script)
                .output()
                .await
                .map_err(|e| Error::Tool(format!("Failed: {}", e)))?;
            if output.status.success() {
                Ok(format!("Switched to space {}", space))
            } else {
                Err(Error::Tool(
                    String::from_utf8_lossy(&output.stderr).to_string(),
                ))
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = space;
            Ok("switch_space only available on macOS".to_string())
        }
    }
}

/// Wait for an application to launch
pub struct WaitForAppTool;

#[async_trait]
impl TalosTool for WaitForAppTool {
    fn name(&self) -> &'static str {
        "wait_for_app"
    }
    fn description(&self) -> &'static str {
        "Wait until a specific application is running (with timeout)"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("name", "string", "Application name to wait for", true)
            .with_param(
                "timeout_secs",
                "integer",
                "Max seconds to wait (default 30)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("'name' is required".to_string()))?;
        let timeout = args
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(30);

        let start = std::time::Instant::now();
        loop {
            let mut sys = System::new();
            sys.refresh_processes(sysinfo::ProcessesToUpdate::All);
            let found = sys.processes().values().any(|p| {
                p.name()
                    .to_string_lossy()
                    .to_lowercase()
                    .contains(&name.to_lowercase())
            });
            if found {
                return Ok(json!({
                    "found": true,
                    "app": name,
                    "waited_secs": start.elapsed().as_secs(),
                })
                .to_string());
            }
            if start.elapsed().as_secs() >= timeout {
                return Ok(json!({
                    "found": false,
                    "app": name,
                    "timeout": true,
                    "waited_secs": timeout,
                })
                .to_string());
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }
}

/// Wait/sleep for a specified number of seconds
pub struct WaitSecondsTool;

#[async_trait]
impl TalosTool for WaitSecondsTool {
    fn name(&self) -> &'static str {
        "wait_seconds"
    }
    fn description(&self) -> &'static str {
        "Wait/sleep for a specified number of seconds"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "seconds",
            "number",
            "Number of seconds to wait",
            true,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let secs = args
            .get("seconds")
            .and_then(|v| v.as_f64())
            .ok_or_else(|| Error::Tool("'seconds' is required".to_string()))?;
        if !(0.0..=300.0).contains(&secs) {
            return Err(Error::Tool("seconds must be between 0 and 300".to_string()));
        }
        tokio::time::sleep(std::time::Duration::from_secs_f64(secs)).await;
        Ok(format!("Waited {:.1} seconds", secs))
    }
}

/// Toggle WiFi power on/off
pub struct WifiPowerTool;

#[async_trait]
impl TalosTool for WifiPowerTool {
    fn name(&self) -> &'static str {
        "wifi_power"
    }
    fn description(&self) -> &'static str {
        "Toggle WiFi power on or off"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "enabled",
            "boolean",
            "true to enable WiFi, false to disable",
            true,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let enabled = args
            .get("enabled")
            .and_then(|v| v.as_bool())
            .ok_or_else(|| Error::Tool("'enabled' is required".to_string()))?;

        #[cfg(target_os = "macos")]
        {
            let state = if enabled { "on" } else { "off" };
            let output = tokio::process::Command::new("networksetup")
                .args(["-setairportpower", "en0", state])
                .output()
                .await
                .map_err(|e| Error::Tool(format!("Failed: {}", e)))?;
            if output.status.success() {
                Ok(format!("WiFi turned {}", state))
            } else {
                Err(Error::Tool(
                    String::from_utf8_lossy(&output.stderr).to_string(),
                ))
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = enabled;
            Ok("wifi_power only available on macOS".to_string())
        }
    }
}

/// Take a screenshot of a specific window
pub struct ScreenshotWindowTool;

#[async_trait]
impl TalosTool for ScreenshotWindowTool {
    fn name(&self) -> &'static str {
        "screenshot_window"
    }
    fn description(&self) -> &'static str {
        "Take a screenshot of a specific window by application name"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("app", "string", "Application name to screenshot", true)
            .with_param(
                "path",
                "string",
                "Output file path (default: /tmp/screenshot_window.png)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        let app = args
            .get("app")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Tool("'app' is required".to_string()))?;
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("/tmp/screenshot_window.png");

        #[cfg(target_os = "macos")]
        {
            // Get window ID via AppleScript, then use screencapture -l
            let script = format!(
                r#"tell application "System Events"
                    set wid to id of front window of application process "{}"
                end tell
                return wid"#,
                app
            );
            let wid_output = tokio::process::Command::new("osascript")
                .arg("-e")
                .arg(&script)
                .output()
                .await
                .map_err(|e| Error::Tool(format!("Failed to get window ID: {}", e)))?;

            if wid_output.status.success() {
                let wid = String::from_utf8_lossy(&wid_output.stdout)
                    .trim()
                    .to_string();
                let output = tokio::process::Command::new("screencapture")
                    .args(["-l", &wid, "-o", path])
                    .output()
                    .await
                    .map_err(|e| Error::Tool(format!("Screenshot failed: {}", e)))?;
                if output.status.success() {
                    Ok(json!({ "path": path, "app": app, "window_id": wid }).to_string())
                } else {
                    Err(Error::Tool(
                        String::from_utf8_lossy(&output.stderr).to_string(),
                    ))
                }
            } else {
                Err(Error::Tool(format!("Could not find window for '{}'", app)))
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (app, path);
            Ok("screenshot_window only available on macOS".to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// T10 — Screen recording (macOS screencapture)
// ---------------------------------------------------------------------------

/// Start a screen recording to a file
pub struct ScreenRecordStartTool;

#[async_trait]
impl TalosTool for ScreenRecordStartTool {
    fn name(&self) -> &'static str {
        "screen_record_start"
    }
    fn description(&self) -> &'static str {
        "Start a screen recording using macOS screencapture (saves to MOV)"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("output", "string", "Output file path (default: ~/Desktop/zeus_recording_<timestamp>.mov)", false)
            .with_param("duration", "integer", "Max duration in seconds (default: 60, max: 300)", false)
            .with_param("audio", "boolean", "Record audio (default: false)", false)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let output = if let Some(path) = args.get("output").and_then(|v| v.as_str()) {
                path.to_string()
            } else {
                let ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                format!("{}/Desktop/zeus_recording_{}.mov", std::env::var("HOME").unwrap_or_default(), ts)
            };

            let duration = args.get("duration").and_then(|v| v.as_u64()).unwrap_or(60).min(300);
            let audio = args.get("audio").and_then(|v| v.as_bool()).unwrap_or(false);

            let mut cmd = tokio::process::Command::new("screencapture");
            cmd.arg("-v") // video mode
                .arg("-t").arg("mov")
                .arg("-T").arg(duration.to_string())
                .arg(&output);

            if audio {
                cmd.arg("-A"); // include audio (macOS 14+)
            }

            // Spawn in background so we don't block
            let output_path = output.clone();
            tokio::spawn(async move {
                let _ = cmd.output().await;
            });

            Ok(format!(
                "Screen recording started. Saving to: {} (max {}s, audio: {}). Use screen_record_stop to end early.",
                output_path, duration, audio
            ))
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("screen_record_start only available on macOS".to_string())
        }
    }
}

/// Stop an active screen recording
pub struct ScreenRecordStopTool;

#[async_trait]
impl TalosTool for ScreenRecordStopTool {
    fn name(&self) -> &'static str {
        "screen_record_stop"
    }
    fn description(&self) -> &'static str {
        "Stop the active screen recording (sends SIGINT to screencapture)"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let output = tokio::process::Command::new("pkill")
                .arg("-INT") // graceful stop
                .arg("screencapture")
                .output()
                .await
                .map_err(|e| Error::Tool(format!("Failed to stop recording: {}", e)))?;

            if output.status.success() {
                Ok("Screen recording stopped. File saved to the path specified in screen_record_start.".to_string())
            } else {
                Ok("No active screen recording found (screencapture was not running).".to_string())
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            Ok("screen_record_stop only available on macOS".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_enable_focus_schema() {
        let tool = EnableFocusTool;
        assert_eq!(tool.name(), "enable_focus");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("should be an object");
        let props = params["properties"]
            .as_object()
            .expect("should be an object");
        assert!(props.contains_key("mode"));
        let required = params["required"].as_array().expect("should be an array");
        assert!(!required.iter().any(|v| v.as_str() == Some("mode")));
    }

    #[test]
    fn test_disable_focus_schema() {
        let tool = DisableFocusTool;
        assert_eq!(tool.name(), "disable_focus");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("should be an object");
        let props = params["properties"]
            .as_object()
            .expect("should be an object");
        assert!(props.contains_key("mode"));
        let required = params["required"].as_array().expect("should be an array");
        assert!(!required.iter().any(|v| v.as_str() == Some("mode")));
    }

    #[test]
    fn test_env_vars_schema() {
        let tool = EnvVarsTool;
        assert_eq!(tool.name(), "env_vars");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("should be an object");
        let props = params["properties"]
            .as_object()
            .expect("should be an object");
        assert!(props.contains_key("name"));
        let required = params["required"].as_array().expect("should be an array");
        assert!(!required.iter().any(|v| v.as_str() == Some("name")));
    }

    #[test]
    fn test_check_permissions_schema() {
        let tool = CheckPermissionsTool;
        assert_eq!(tool.name(), "check_permissions");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("should be an object");
        let props = params["properties"]
            .as_object()
            .expect("should be an object");
        assert!(props.contains_key("path"));
        let required = params["required"].as_array().expect("should be an array");
        assert!(required.iter().any(|v| v.as_str() == Some("path")));
    }

    #[test]
    fn test_list_shortcuts_schema() {
        let tool = ListShortcutsTool;
        assert_eq!(tool.name(), "list_shortcuts");
        let _schema = tool.schema();
    }

    #[test]
    fn test_run_shortcut_schema() {
        let tool = RunShortcutTool;
        assert_eq!(tool.name(), "run_shortcut");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("should be an object");
        let props = params["properties"]
            .as_object()
            .expect("should be an object");
        assert!(props.contains_key("name"));
        assert!(props.contains_key("input"));
        let required = params["required"].as_array().expect("should be an array");
        assert!(required.iter().any(|v| v.as_str() == Some("name")));
    }

    #[test]
    fn test_screenshot_region_schema() {
        let tool = ScreenshotRegionTool;
        assert_eq!(tool.name(), "screenshot_region");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("should be an object");
        let props = params["properties"]
            .as_object()
            .expect("should be an object");
        assert!(props.contains_key("x"));
        assert!(props.contains_key("y"));
        assert!(props.contains_key("width"));
        assert!(props.contains_key("height"));
        assert!(props.contains_key("output"));
        let required = params["required"].as_array().expect("should be an array");
        assert!(required.iter().any(|v| v.as_str() == Some("x")));
        assert!(required.iter().any(|v| v.as_str() == Some("y")));
        assert!(required.iter().any(|v| v.as_str() == Some("width")));
        assert!(required.iter().any(|v| v.as_str() == Some("height")));
    }

    #[test]
    fn test_execute_applescript_schema() {
        let tool = ExecuteApplescriptTool;
        assert_eq!(tool.name(), "execute_applescript");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("should be an object");
        let props = params["properties"]
            .as_object()
            .expect("should be an object");
        assert!(props.contains_key("script"));
        let required = params["required"].as_array().expect("should be an array");
        assert!(required.iter().any(|v| v.as_str() == Some("script")));
    }

    #[test]
    fn test_validate_applescript_allows_normal() {
        assert!(
            validate_applescript(r#"tell application "Finder" to get name of front window"#)
                .is_ok()
        );
        assert!(validate_applescript(r#"display dialog "Hello""#).is_ok());
    }

    #[test]
    fn test_validate_applescript_blocks_null_bytes() {
        assert!(validate_applescript("tell application\0\"Finder\"").is_err());
    }

    #[test]
    fn test_validate_applescript_blocks_too_long() {
        let long_script = "a".repeat(50_001);
        assert!(validate_applescript(&long_script).is_err());
    }

    #[test]
    fn test_validate_applescript_blocks_dangerous_shell() {
        assert!(validate_applescript(r#"do shell script "rm -rf /""#).is_err());
        assert!(validate_applescript(r#"do shell script "cat /etc/shadow""#).is_err());
    }

    #[test]
    fn test_retry_command_schema() {
        let tool = RetryCommandTool;
        assert_eq!(tool.name(), "retry_command");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("should be an object");
        let props = params["properties"]
            .as_object()
            .expect("should be an object");
        assert!(props.contains_key("command"));
        assert!(props.contains_key("retries"));
        assert!(props.contains_key("delay_ms"));
        let required = params["required"].as_array().expect("should be an array");
        assert!(required.iter().any(|v| v.as_str() == Some("command")));
    }

    #[test]
    fn test_service_status_schema() {
        let tool = ServiceStatusTool;
        assert_eq!(tool.name(), "service_status");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("should be an object");
        let props = params["properties"]
            .as_object()
            .expect("should be an object");
        assert!(props.contains_key("service"));
        let required = params["required"].as_array().expect("should be an array");
        assert!(required.iter().any(|v| v.as_str() == Some("service")));
    }

    #[test]
    fn test_speak_text_schema() {
        let tool = SpeakTextTool;
        assert_eq!(tool.name(), "speak_text");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("should be an object");
        let props = params["properties"]
            .as_object()
            .expect("should be an object");
        assert!(props.contains_key("text"));
        assert!(props.contains_key("voice"));
        assert!(props.contains_key("rate"));
        let required = params["required"].as_array().expect("should be an array");
        assert!(required.iter().any(|v| v.as_str() == Some("text")));
    }

    #[test]
    fn test_send_notification_schema() {
        let tool = SendNotificationTool;
        assert_eq!(tool.name(), "send_notification");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("should be an object");
        let props = params["properties"]
            .as_object()
            .expect("should be an object");
        assert!(props.contains_key("title"));
        assert!(props.contains_key("message"));
        assert!(props.contains_key("subtitle"));
        assert!(props.contains_key("sound"));
        let required = params["required"].as_array().expect("should be an array");
        assert!(required.iter().any(|v| v.as_str() == Some("title")));
        assert!(required.iter().any(|v| v.as_str() == Some("message")));
    }

    #[test]
    fn test_set_mute_schema() {
        let tool = SetMuteTool;
        assert_eq!(tool.name(), "set_mute");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("should be an object");
        let props = params["properties"]
            .as_object()
            .expect("should be an object");
        assert!(props.contains_key("muted"));
        let required = params["required"].as_array().expect("should be an array");
        assert!(required.iter().any(|v| v.as_str() == Some("muted")));
    }

    #[tokio::test]
    async fn test_env_vars() {
        let tool = EnvVarsTool;
        let result = tool.execute(json!({})).await.expect("SQL should execute");
        assert!(!result.is_empty());
        // Should be valid JSON
        let parsed: serde_json::Value =
            serde_json::from_str(&result).expect("should parse successfully");
        assert!(parsed.is_object());
        // Should contain at least some variables
        assert!(
            !parsed
                .as_object()
                .expect("should parse successfully")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn test_retry_command_success() {
        let tool = RetryCommandTool;
        let result = tool
            .execute(json!({
                "command": "echo hello",
                "retries": 1,
                "delay_ms": 100
            }))
            .await
            .expect("async operation should succeed");
        let parsed: serde_json::Value =
            serde_json::from_str(&result).expect("should parse successfully");
        assert_eq!(parsed["success"], true);
        assert_eq!(parsed["attempt"], 1);
        assert_eq!(parsed["output"], "hello");
    }
}
