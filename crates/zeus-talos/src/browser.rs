//! Browser automation tools (macOS)

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::Value;
#[cfg(target_os = "macos")]
use zeus_core::Error;
use zeus_core::{Result, ToolSchema};

#[cfg(target_os = "macos")]
use crate::run_applescript;

/// Get current Safari URL
pub struct SafariUrlTool;

#[async_trait]
impl TalosTool for SafariUrlTool {
    fn name(&self) -> &'static str {
        "safari_url"
    }
    fn description(&self) -> &'static str {
        "Get the URL of the current Safari tab"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let script = r#"
                tell application "Safari"
                    set currentURL to URL of current tab of window 1
                    set pageTitle to name of current tab of window 1
                end tell
                return "Title: " & pageTitle & linefeed & "URL: " & currentURL
            "#;

            run_applescript(script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            Ok("Browser tools only available on macOS".to_string())
        }
    }
}

/// Get all Safari tabs
pub struct SafariTabsTool;

#[async_trait]
impl TalosTool for SafariTabsTool {
    fn name(&self) -> &'static str {
        "safari_tabs"
    }
    fn description(&self) -> &'static str {
        "List all open Safari tabs with titles and URLs"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let script = r#"
                set tabList to ""
                tell application "Safari"
                    repeat with w in windows
                        repeat with t in tabs of w
                            set tabList to tabList & (name of t) & " | " & (URL of t) & linefeed
                        end repeat
                    end repeat
                end tell
                return tabList
            "#;

            run_applescript(script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            Ok("Browser tools only available on macOS".to_string())
        }
    }
}

/// Execute JavaScript in Safari
pub struct SafariJsTool;

#[async_trait]
impl TalosTool for SafariJsTool {
    fn name(&self) -> &'static str {
        "safari_execute_js"
    }
    fn description(&self) -> &'static str {
        "Execute JavaScript in the current Safari tab and return the result"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "code",
            "string",
            "JavaScript code to execute",
            true,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let code = args
                .get("code")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing code".to_string()))?;

            let escaped = crate::sanitize_applescript(code).replace('\n', "\\n");
            let script = format!(
                r#"
                tell application "Safari"
                    set jsResult to do JavaScript "{}" in current tab of window 1
                    return jsResult as text
                end tell
            "#,
                escaped
            );

            run_applescript(&script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Browser tools only available on macOS".to_string())
        }
    }
}

/// Navigate Safari to a URL
pub struct SafariNavigateTool;

#[async_trait]
impl TalosTool for SafariNavigateTool {
    fn name(&self) -> &'static str {
        "safari_navigate"
    }
    fn description(&self) -> &'static str {
        "Navigate Safari to a URL"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("url", "string", "URL to navigate to", true)
            .with_param(
                "new_tab",
                "boolean",
                "Open in new tab (default false)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let url = args
                .get("url")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing url".to_string()))?;

            let new_tab = args
                .get("new_tab")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            let escaped = crate::sanitize_applescript(url);
            let script = if new_tab {
                format!(
                    r#"
                    tell application "Safari"
                        activate
                        tell window 1
                            set newTab to make new tab
                            set URL of newTab to "{}"
                        end tell
                    end tell
                    return "Opened in new tab"
                "#,
                    escaped
                )
            } else {
                format!(
                    r#"
                    tell application "Safari"
                        activate
                        set URL of current tab of window 1 to "{}"
                    end tell
                    return "Navigated to URL"
                "#,
                    escaped
                )
            };

            run_applescript(&script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Browser tools only available on macOS".to_string())
        }
    }
}

/// Open a new Safari tab with an optional URL
pub struct SafariNewTabTool;

#[async_trait]
impl TalosTool for SafariNewTabTool {
    fn name(&self) -> &'static str {
        "safari_new_tab"
    }
    fn description(&self) -> &'static str {
        "Open a new tab in Safari with an optional URL"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "url",
            "string",
            "URL to open in the new tab (optional)",
            false,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let url = args.get("url").and_then(|v| v.as_str());

            let script = if let Some(url) = url {
                let escaped = crate::sanitize_applescript(url);
                format!(
                    r#"
                    tell application "Safari"
                        activate
                        tell window 1
                            set newTab to make new tab at end of tabs
                            set URL of newTab to "{}"
                        end tell
                    end tell
                    return "New tab opened with URL"
                "#,
                    escaped
                )
            } else {
                r#"
                    tell application "Safari"
                        activate
                        tell window 1
                            make new tab at end of tabs
                        end tell
                    end tell
                    return "New tab opened"
                "#
                .to_string()
            };

            run_applescript(&script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Browser tools only available on macOS".to_string())
        }
    }
}

/// Close a Safari tab by index
pub struct SafariCloseTabTool;

#[async_trait]
impl TalosTool for SafariCloseTabTool {
    fn name(&self) -> &'static str {
        "safari_close_tab"
    }
    fn description(&self) -> &'static str {
        "Close the current or a specific Safari tab by index"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "index",
            "integer",
            "Tab index to close (1-based, default: current tab)",
            false,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let script = if let Some(index) = args.get("index").and_then(|v| v.as_u64()) {
                format!(
                    r#"
                    tell application "Safari"
                        close tab {} of window 1
                    end tell
                    return "Tab {} closed"
                "#,
                    index, index
                )
            } else {
                r#"
                    tell application "Safari"
                        close current tab of window 1
                    end tell
                    return "Current tab closed"
                "#
                .to_string()
            };

            run_applescript(&script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Browser tools only available on macOS".to_string())
        }
    }
}

/// Get recent Safari browser history
pub struct SafariHistoryTool;

#[async_trait]
impl TalosTool for SafariHistoryTool {
    fn name(&self) -> &'static str {
        "safari_history"
    }
    fn description(&self) -> &'static str {
        "Get recent Safari browser history"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "limit",
            "integer",
            "Max number of history entries to return (default 20)",
            false,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20);

            let home = dirs::home_dir()
                .ok_or_else(|| Error::Tool("Could not determine home directory".to_string()))?;
            let db_path = home.join("Library/Safari/History.db");

            let output = tokio::process::Command::new("sqlite3")
                .arg("-separator")
                .arg(" | ")
                .arg(db_path)
                .arg(format!(
                    "SELECT hi.url, hv.title, datetime(hv.visit_time + 978307200, 'unixepoch', 'localtime') as visit_date \
                     FROM history_items hi \
                     JOIN history_visits hv ON hi.id = hv.history_item \
                     ORDER BY hv.visit_time DESC \
                     LIMIT {};",
                    limit
                ))
                .output()
                .await
                .map_err(|e| Error::Tool(format!("Failed to query Safari history: {}", e)))?;

            if output.status.success() {
                let result = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if result.is_empty() {
                    Ok(
                        "No history entries found (database may require Full Disk Access)"
                            .to_string(),
                    )
                } else {
                    Ok(result)
                }
            } else {
                Err(Error::Tool(format!(
                    "Safari history query failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                )))
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Browser tools only available on macOS".to_string())
        }
    }
}

/// List Safari bookmarks
pub struct SafariBookmarksTool;

#[async_trait]
impl TalosTool for SafariBookmarksTool {
    fn name(&self) -> &'static str {
        "safari_bookmarks"
    }
    fn description(&self) -> &'static str {
        "List Safari bookmarks, optionally filtered by folder"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "folder",
            "string",
            "Filter bookmarks by folder name (optional)",
            false,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let folder = args.get("folder").and_then(|v| v.as_str());

            let home = dirs::home_dir()
                .ok_or_else(|| Error::Tool("Could not determine home directory".to_string()))?;
            let db_path = home.join("Library/Safari/Bookmarks.db");

            let query = if let Some(folder_name) = folder {
                let sanitized = crate::sanitize_shell_arg(folder_name);
                format!(
                    "SELECT b.title, b.url FROM bookmarks b \
                     JOIN bookmarks p ON b.parent = p.id \
                     WHERE b.url IS NOT NULL AND p.title = {} \
                     ORDER BY b.title;",
                    sanitized
                )
            } else {
                "SELECT title, url FROM bookmarks WHERE url IS NOT NULL ORDER BY title LIMIT 50;"
                    .to_string()
            };

            let output = tokio::process::Command::new("sqlite3")
                .arg("-separator")
                .arg(" | ")
                .arg(db_path)
                .arg(query)
                .output()
                .await
                .map_err(|e| Error::Tool(format!("Failed to query Safari bookmarks: {}", e)))?;

            if output.status.success() {
                let result = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if result.is_empty() {
                    Ok("No bookmarks found (database may require Full Disk Access)".to_string())
                } else {
                    Ok(result)
                }
            } else {
                Err(Error::Tool(format!(
                    "Safari bookmarks query failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                )))
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Browser tools only available on macOS".to_string())
        }
    }
}

/// Bookmark the current Safari page
pub struct SafariAddBookmarkTool;

#[async_trait]
impl TalosTool for SafariAddBookmarkTool {
    fn name(&self) -> &'static str {
        "safari_add_bookmark"
    }
    fn description(&self) -> &'static str {
        "Bookmark the current Safari page"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "folder",
            "string",
            "Bookmark folder name (optional, default: Bookmarks Menu)",
            false,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let folder = args
                .get("folder")
                .and_then(|v| v.as_str())
                .unwrap_or("Bookmarks Menu");

            let escaped_folder = crate::sanitize_applescript(folder);
            let script = format!(
                r#"
                tell application "Safari"
                    set pageURL to URL of current tab of window 1
                    set pageTitle to name of current tab of window 1
                end tell
                tell application "System Events"
                    tell process "Safari"
                        click menu item "Add Bookmark\u{{2026}}" of menu "Bookmarks" of menu bar 1
                        delay 1
                        keystroke return
                    end tell
                end tell
                return "Bookmarked: " & pageTitle & " (" & pageURL & ") to folder {}"
            "#,
                escaped_folder
            );

            run_applescript(&script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Browser tools only available on macOS".to_string())
        }
    }
}

/// Get Safari reading list items
pub struct SafariReadingListTool;

#[async_trait]
impl TalosTool for SafariReadingListTool {
    fn name(&self) -> &'static str {
        "safari_reading_list"
    }
    fn description(&self) -> &'static str {
        "Get items from the Safari reading list"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let home = dirs::home_dir()
                .ok_or_else(|| Error::Tool("Could not determine home directory".to_string()))?;
            let plist_path = home.join("Library/Safari/Bookmarks.plist");

            // Use plutil to convert binary plist to xml, then extract reading list entries
            let output = tokio::process::Command::new("plutil")
                .arg("-convert")
                .arg("xml1")
                .arg("-o")
                .arg("-")
                .arg(&plist_path)
                .output()
                .await
                .map_err(|e| {
                    Error::Tool(format!("Failed to read Safari bookmarks plist: {}", e))
                })?;

            if !output.status.success() {
                return Err(Error::Tool(format!(
                    "Failed to convert plist: {}",
                    String::from_utf8_lossy(&output.stderr)
                )));
            }

            let plist_xml = String::from_utf8_lossy(&output.stdout);

            // Parse reading list entries from the XML plist
            let mut results = Vec::new();
            let mut in_reading_list = false;
            let mut current_title: Option<String> = None;
            let mut next_is_value = false;
            let mut value_key = String::new();

            for line in plist_xml.lines() {
                let trimmed = line.trim();

                if trimmed.contains("com.apple.ReadingList") {
                    in_reading_list = true;
                }

                if in_reading_list {
                    if trimmed == "<key>URLString</key>" {
                        next_is_value = true;
                        value_key = "url".to_string();
                    } else if trimmed == "<key>title</key>" {
                        next_is_value = true;
                        value_key = "title".to_string();
                    } else if next_is_value && trimmed.starts_with("<string>") {
                        let val = trimmed
                            .trim_start_matches("<string>")
                            .trim_end_matches("</string>");
                        if value_key == "title" {
                            current_title = Some(val.to_string());
                        } else if value_key == "url" {
                            let title = current_title.take().unwrap_or_default();
                            results.push(format!("{} | {}", title, val));
                        }
                        next_is_value = false;
                    }
                }
            }

            if results.is_empty() {
                Ok("No reading list items found (may require Full Disk Access)".to_string())
            } else {
                Ok(results.join("\n"))
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            Ok("Browser tools only available on macOS".to_string())
        }
    }
}

/// Get page source of the current Safari tab
pub struct SafariSourceTool;

#[async_trait]
impl TalosTool for SafariSourceTool {
    fn name(&self) -> &'static str {
        "safari_source"
    }
    fn description(&self) -> &'static str {
        "Get the HTML source of the current Safari tab"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let script = r#"
                tell application "Safari"
                    set pageSource to do JavaScript "document.documentElement.outerHTML" in current tab of window 1
                    return pageSource
                end tell
            "#;

            run_applescript(script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            Ok("Browser tools only available on macOS".to_string())
        }
    }
}

/// Get the title of the current Safari tab
pub struct SafariTitleTool;

#[async_trait]
impl TalosTool for SafariTitleTool {
    fn name(&self) -> &'static str {
        "safari_title"
    }
    fn description(&self) -> &'static str {
        "Get the title of the current Safari tab"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let script = r#"
                tell application "Safari"
                    set tabTitle to name of current tab of window 1
                    return tabTitle
                end tell
            "#;

            run_applescript(script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            Ok("Browser tools only available on macOS".to_string())
        }
    }
}

/// Navigate back in Safari
pub struct SafariBackTool;

#[async_trait]
impl TalosTool for SafariBackTool {
    fn name(&self) -> &'static str {
        "safari_back"
    }
    fn description(&self) -> &'static str {
        "Navigate back in Safari browser history"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let script = r#"
                tell application "Safari"
                    do JavaScript "history.back()" in current tab of window 1
                end tell
                return "Navigated back"
            "#;

            run_applescript(script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            Ok("Browser tools only available on macOS".to_string())
        }
    }
}

/// Navigate forward in Safari
pub struct SafariForwardTool;

#[async_trait]
impl TalosTool for SafariForwardTool {
    fn name(&self) -> &'static str {
        "safari_forward"
    }
    fn description(&self) -> &'static str {
        "Navigate forward in Safari browser history"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let script = r#"
                tell application "Safari"
                    do JavaScript "history.forward()" in current tab of window 1
                end tell
                return "Navigated forward"
            "#;

            run_applescript(script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            Ok("Browser tools only available on macOS".to_string())
        }
    }
}
