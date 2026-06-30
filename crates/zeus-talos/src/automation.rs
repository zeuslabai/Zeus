//! UI Automation tools - keystrokes, mouse, window management

use crate::TalosTool;
use async_trait::async_trait;
use serde_json::Value;
#[cfg(target_os = "macos")]
use serde_json::json;
#[cfg(target_os = "macos")]
use zeus_core::Error;
use zeus_core::{Result, ToolSchema};

#[allow(dead_code)]
fn parse_modifiers(modifiers: &str) -> zeus_core::Result<String> {
    let mut parts = Vec::new();
    for m in modifiers.split(',') {
        let m = m.trim().to_lowercase();
        match m.as_str() {
            "command" | "cmd" => parts.push("command down"),
            "shift" => parts.push("shift down"),
            "option" | "alt" => parts.push("option down"),
            "control" | "ctrl" => parts.push("control down"),
            "" => {}
            other => {
                return Err(zeus_core::Error::Tool(format!(
                    "Unknown modifier '{}'. Valid: command, shift, option, control",
                    other
                )));
            }
        }
    }
    Ok(format!("{{{}}}", parts.join(", ")))
}

// ---------------------------------------------------------------------------
// 1. KeystrokeTool
// ---------------------------------------------------------------------------

/// Send keystrokes to the frontmost application
pub struct KeystrokeTool;

#[async_trait]
impl TalosTool for KeystrokeTool {
    fn name(&self) -> &'static str {
        "keystroke"
    }
    fn description(&self) -> &'static str {
        "Send keystrokes to the frontmost application"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("text", "string", "Text to type as keystrokes", true)
            .with_param(
                "modifiers",
                "string",
                "Comma-separated modifiers: command, shift, option, control (optional)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let text = args
                .get("text")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing text".to_string()))?;

            let escaped = crate::sanitize_applescript(text);

            let script = if let Some(mods) = args.get("modifiers").and_then(|v| v.as_str()) {
                let modifier_list = parse_modifiers(mods)?;
                format!(
                    r#"tell application "System Events" to keystroke "{}" using {}"#,
                    escaped, modifier_list
                )
            } else {
                format!(
                    r#"tell application "System Events" to keystroke "{}""#,
                    escaped
                )
            };

            crate::run_applescript(&script)?;
            Ok(format!("Sent keystroke: {}", text))
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Keystroke tool only available on macOS".to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// 2. KeyCodeTool
// ---------------------------------------------------------------------------

/// Send a key code for special keys (e.g. return=36, escape=53, tab=48)
pub struct KeyCodeTool;

#[async_trait]
impl TalosTool for KeyCodeTool {
    fn name(&self) -> &'static str {
        "key_code"
    }
    fn description(&self) -> &'static str {
        "Send a key code to the frontmost application (for special keys like return, escape, arrows)"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "code",
                "integer",
                "macOS key code (e.g. 36=return, 53=escape, 48=tab, 123-126=arrows)",
                true,
            )
            .with_param(
                "modifiers",
                "string",
                "Comma-separated modifiers: command, shift, option, control (optional)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let code = args
                .get("code")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| Error::Tool("Missing code".to_string()))?;

            let script = if let Some(mods) = args.get("modifiers").and_then(|v| v.as_str()) {
                let modifier_list = parse_modifiers(mods)?;
                format!(
                    r#"tell application "System Events" to key code {} using {}"#,
                    code, modifier_list
                )
            } else {
                format!(r#"tell application "System Events" to key code {}"#, code)
            };

            crate::run_applescript(&script)?;
            Ok(format!("Sent key code: {}", code))
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Key code tool only available on macOS".to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// 3. MouseClickTool
// ---------------------------------------------------------------------------

/// Click at screen coordinates
pub struct MouseClickTool;

#[async_trait]
impl TalosTool for MouseClickTool {
    fn name(&self) -> &'static str {
        "mouse_click"
    }
    fn description(&self) -> &'static str {
        "Click at screen coordinates using the mouse"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("x", "integer", "X screen coordinate", true)
            .with_param("y", "integer", "Y screen coordinate", true)
            .with_param(
                "button",
                "string",
                "Mouse button: 'left' or 'right' (default 'left')",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let x = args
                .get("x")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| Error::Tool("Missing x coordinate".to_string()))?;

            let y = args
                .get("y")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| Error::Tool("Missing y coordinate".to_string()))?;

            let button = args
                .get("button")
                .and_then(|v| v.as_str())
                .unwrap_or("left");

            // Try cliclick first, fall back to AppleScript
            let output = tokio::process::Command::new("which")
                .arg("cliclick")
                .output()
                .await;

            let has_cliclick = output.map(|o| o.status.success()).unwrap_or(false);

            if has_cliclick {
                let cmd = match button {
                    "right" => format!("rc:{}:{}", x, y),
                    _ => format!("c:{}:{}", x, y),
                };
                let result = tokio::process::Command::new("cliclick")
                    .arg(&cmd)
                    .output()
                    .await
                    .map_err(|e| Error::Tool(format!("cliclick failed: {}", e)))?;

                if result.status.success() {
                    Ok(format!("Clicked {} at ({}, {})", button, x, y))
                } else {
                    Err(Error::Tool(format!(
                        "cliclick error: {}",
                        String::from_utf8_lossy(&result.stderr)
                    )))
                }
            } else {
                // Fallback: AppleScript with System Events
                let click_type = match button {
                    "right" => "right",
                    _ => "left",
                };
                let script = format!(
                    r#"
                    do shell script "python3 -c '
import Quartz
point = Quartz.CGPointMake({x}, {y})
if \"{click_type}\" == \"right\":
    down = Quartz.CGEventCreateMouseEvent(None, Quartz.kCGEventRightMouseDown, point, Quartz.kCGMouseButtonRight)
    up = Quartz.CGEventCreateMouseEvent(None, Quartz.kCGEventRightMouseUp, point, Quartz.kCGMouseButtonRight)
else:
    down = Quartz.CGEventCreateMouseEvent(None, Quartz.kCGEventLeftMouseDown, point, Quartz.kCGMouseButtonLeft)
    up = Quartz.CGEventCreateMouseEvent(None, Quartz.kCGEventLeftMouseUp, point, Quartz.kCGMouseButtonLeft)
Quartz.CGEventPost(Quartz.kCGHIDEventTap, down)
Quartz.CGEventPost(Quartz.kCGHIDEventTap, up)
'"#,
                    x = x,
                    y = y,
                    click_type = click_type
                );
                crate::run_applescript(&script)?;
                Ok(format!("Clicked {} at ({}, {})", button, x, y))
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Mouse click tool only available on macOS".to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// 4. MouseMoveTool
// ---------------------------------------------------------------------------

/// Move the mouse to screen coordinates
pub struct MouseMoveTool;

#[async_trait]
impl TalosTool for MouseMoveTool {
    fn name(&self) -> &'static str {
        "mouse_move"
    }
    fn description(&self) -> &'static str {
        "Move the mouse cursor to screen coordinates"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("x", "integer", "X screen coordinate", true)
            .with_param("y", "integer", "Y screen coordinate", true)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let x = args
                .get("x")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| Error::Tool("Missing x coordinate".to_string()))?;

            let y = args
                .get("y")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| Error::Tool("Missing y coordinate".to_string()))?;

            // Try cliclick first
            let output = tokio::process::Command::new("which")
                .arg("cliclick")
                .output()
                .await;

            let has_cliclick = output.map(|o| o.status.success()).unwrap_or(false);

            if has_cliclick {
                let result = tokio::process::Command::new("cliclick")
                    .arg(format!("m:{}:{}", x, y))
                    .output()
                    .await
                    .map_err(|e| Error::Tool(format!("cliclick failed: {}", e)))?;

                if result.status.success() {
                    Ok(format!("Moved mouse to ({}, {})", x, y))
                } else {
                    Err(Error::Tool(format!(
                        "cliclick error: {}",
                        String::from_utf8_lossy(&result.stderr)
                    )))
                }
            } else {
                let script = format!(
                    r#"
                    do shell script "python3 -c '
import Quartz
point = Quartz.CGPointMake({}, {})
move = Quartz.CGEventCreateMouseEvent(None, Quartz.kCGEventMouseMoved, point, Quartz.kCGMouseButtonLeft)
Quartz.CGEventPost(Quartz.kCGHIDEventTap, move)
'"#,
                    x, y
                );
                crate::run_applescript(&script)?;
                Ok(format!("Moved mouse to ({}, {})", x, y))
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Mouse move tool only available on macOS".to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// 5. ActivateAppTool
// ---------------------------------------------------------------------------

/// Bring an application to the front
pub struct ActivateAppTool;

#[async_trait]
impl TalosTool for ActivateAppTool {
    fn name(&self) -> &'static str {
        "activate_app"
    }
    fn description(&self) -> &'static str {
        "Bring an application to the front and activate it"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "name",
            "string",
            "Application name (e.g. 'Safari', 'Finder', 'Terminal')",
            true,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let name = args
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing name".to_string()))?;

            let script = format!(
                r#"tell application "{}" to activate"#,
                crate::sanitize_applescript(name)
            );
            crate::run_applescript(&script)?;
            Ok(format!("Activated {}", name))
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Activate app tool only available on macOS".to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// 6. WindowListTool
// ---------------------------------------------------------------------------

/// List all windows of an application
pub struct WindowListTool;

#[async_trait]
impl TalosTool for WindowListTool {
    fn name(&self) -> &'static str {
        "window_list"
    }
    fn description(&self) -> &'static str {
        "List all windows of an application with their titles, positions, and sizes"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "app",
            "string",
            "Application name (optional, defaults to frontmost app)",
            false,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let app_clause = if let Some(app) = args.get("app").and_then(|v| v.as_str()) {
                format!(
                    r#"application process "{}""#,
                    crate::sanitize_applescript(app)
                )
            } else {
                "first application process whose frontmost is true".to_string()
            };

            let script = format!(
                r#"
                set windowInfo to ""
                tell application "System Events"
                    set targetApp to {}
                    set appName to name of targetApp
                    set winList to windows of targetApp
                    repeat with w in winList
                        set winTitle to title of w
                        set winPos to position of w
                        set winSize to size of w
                        set windowInfo to windowInfo & "Title: " & winTitle & ", Position: (" & (item 1 of winPos) & ", " & (item 2 of winPos) & "), Size: (" & (item 1 of winSize) & "x" & (item 2 of winSize) & ")" & linefeed
                    end repeat
                end tell
                if windowInfo is "" then
                    return "No windows found"
                end if
                return windowInfo
                "#,
                app_clause
            );

            crate::run_applescript(&script)
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Window list tool only available on macOS".to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// 7. WindowResizeTool
// ---------------------------------------------------------------------------

/// Resize a window of an application
pub struct WindowResizeTool;

#[async_trait]
impl TalosTool for WindowResizeTool {
    fn name(&self) -> &'static str {
        "window_resize"
    }
    fn description(&self) -> &'static str {
        "Resize the frontmost window of an application"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("app", "string", "Application name", true)
            .with_param("width", "integer", "New width in pixels", true)
            .with_param("height", "integer", "New height in pixels", true)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let app = args
                .get("app")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing app".to_string()))?;

            let width = args
                .get("width")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| Error::Tool("Missing width".to_string()))?;

            let height = args
                .get("height")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| Error::Tool("Missing height".to_string()))?;

            let script = format!(
                r#"
                tell application "System Events"
                    set size of window 1 of application process "{}" to {{{}, {}}}
                end tell
                "#,
                crate::sanitize_applescript(app),
                width,
                height
            );

            crate::run_applescript(&script)?;
            Ok(format!("Resized {} window to {}x{}", app, width, height))
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Window resize tool only available on macOS".to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// 8. WindowMoveTool
// ---------------------------------------------------------------------------

/// Move a window of an application
pub struct WindowMoveTool;

#[async_trait]
impl TalosTool for WindowMoveTool {
    fn name(&self) -> &'static str {
        "window_move"
    }
    fn description(&self) -> &'static str {
        "Move the frontmost window of an application to new coordinates"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("app", "string", "Application name", true)
            .with_param("x", "integer", "New X position", true)
            .with_param("y", "integer", "New Y position", true)
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let app = args
                .get("app")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing app".to_string()))?;

            let x = args
                .get("x")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| Error::Tool("Missing x coordinate".to_string()))?;

            let y = args
                .get("y")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| Error::Tool("Missing y coordinate".to_string()))?;

            let script = format!(
                r#"
                tell application "System Events"
                    set position of window 1 of application process "{}" to {{{}, {}}}
                end tell
                "#,
                crate::sanitize_applescript(app),
                x,
                y
            );

            crate::run_applescript(&script)?;
            Ok(format!("Moved {} window to ({}, {})", app, x, y))
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Window move tool only available on macOS".to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// 9. WindowMinimizeTool
// ---------------------------------------------------------------------------

/// Minimize a window
pub struct WindowMinimizeTool;

#[async_trait]
impl TalosTool for WindowMinimizeTool {
    fn name(&self) -> &'static str {
        "window_minimize"
    }
    fn description(&self) -> &'static str {
        "Minimize the frontmost window of an application"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "app",
            "string",
            "Application name",
            true,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let app = args
                .get("app")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing app".to_string()))?;

            let script = format!(
                r#"
                tell application "System Events"
                    set miniaturized of window 1 of application process "{}" to true
                end tell
                "#,
                crate::sanitize_applescript(app)
            );

            crate::run_applescript(&script)?;
            Ok(format!("Minimized {} window", app))
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Window minimize tool only available on macOS".to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// 10. WindowCloseTool
// ---------------------------------------------------------------------------

/// Close the frontmost window of an application
pub struct WindowCloseTool;

#[async_trait]
impl TalosTool for WindowCloseTool {
    fn name(&self) -> &'static str {
        "window_close"
    }
    fn description(&self) -> &'static str {
        "Close the frontmost window of an application"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "app",
            "string",
            "Application name",
            true,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let app = args
                .get("app")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing app".to_string()))?;

            let script = format!(
                r#"
                tell application "System Events"
                    tell window 1 of application process "{}" to close
                end tell
                "#,
                crate::sanitize_applescript(app)
            );

            crate::run_applescript(&script)?;
            Ok(format!("Closed {} window", app))
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Window close tool only available on macOS".to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// 11. WindowFullscreenTool
// ---------------------------------------------------------------------------

/// Toggle fullscreen for an application
pub struct WindowFullscreenTool;

#[async_trait]
impl TalosTool for WindowFullscreenTool {
    fn name(&self) -> &'static str {
        "window_fullscreen"
    }
    fn description(&self) -> &'static str {
        "Toggle fullscreen mode for an application window"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "app",
            "string",
            "Application name",
            true,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let app = args
                .get("app")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing app".to_string()))?;

            // First activate the app, then send the fullscreen shortcut
            let activate_script = format!(
                r#"tell application "{}" to activate"#,
                crate::sanitize_applescript(app)
            );
            crate::run_applescript(&activate_script)?;

            // Small delay to ensure the app is frontmost
            let script = r#"
                delay 0.3
                tell application "System Events" to keystroke "f" using {command down, control down}
            "#;
            crate::run_applescript(script)?;
            Ok(format!("Toggled fullscreen for {}", app))
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Window fullscreen tool only available on macOS".to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// 12. MenuClickTool
// ---------------------------------------------------------------------------

/// Click a menu item in an application
pub struct MenuClickTool;

#[async_trait]
impl TalosTool for MenuClickTool {
    fn name(&self) -> &'static str {
        "menu_click"
    }
    fn description(&self) -> &'static str {
        "Click a menu item in an application's menu bar"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("app", "string", "Application name", true)
            .with_param("menu", "string", "Menu name (e.g. 'File', 'Edit')", true)
            .with_param(
                "item",
                "string",
                "Menu item name (e.g. 'Save', 'Copy')",
                true,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let app = args
                .get("app")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing app".to_string()))?;

            let menu = args
                .get("menu")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing menu".to_string()))?;

            let item = args
                .get("item")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing item".to_string()))?;

            let script = format!(
                r#"
                tell application "{}" to activate
                delay 0.3
                tell application "System Events"
                    tell process "{}"
                        click menu item "{}" of menu 1 of menu bar item "{}" of menu bar 1
                    end tell
                end tell
                "#,
                crate::sanitize_applescript(app),
                crate::sanitize_applescript(app),
                crate::sanitize_applescript(item),
                crate::sanitize_applescript(menu)
            );

            crate::run_applescript(&script)?;
            Ok(format!("Clicked {} > {} in {}", menu, item, app))
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Menu click tool only available on macOS".to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// 13. TypeTextTool
// ---------------------------------------------------------------------------

/// Type text with a delay between characters (human-like typing)
pub struct TypeTextTool;

#[async_trait]
impl TalosTool for TypeTextTool {
    fn name(&self) -> &'static str {
        "type_text"
    }
    fn description(&self) -> &'static str {
        "Type text with a delay between characters, simulating human typing"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param("text", "string", "Text to type", true)
            .with_param(
                "delay",
                "number",
                "Delay in seconds between each character (default 0.05)",
                false,
            )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let text = args
                .get("text")
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Tool("Missing text".to_string()))?;

            let delay = args.get("delay").and_then(|v| v.as_f64()).unwrap_or(0.05);

            // Clamp delay to a reasonable range
            let delay = delay.clamp(0.0, 2.0);

            let escaped = crate::sanitize_applescript(text);

            let script = format!(
                r#"
                set charList to every character of "{}"
                tell application "System Events"
                    repeat with c in charList
                        keystroke c
                        delay {}
                    end repeat
                end tell
                "#,
                escaped, delay
            );

            crate::run_applescript(&script)?;
            Ok(format!(
                "Typed {} characters with {:.2}s delay",
                text.len(),
                delay
            ))
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Type text tool only available on macOS".to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// 14. ScreenSizeTool
// ---------------------------------------------------------------------------

/// Get the screen dimensions
pub struct ScreenSizeTool;

#[async_trait]
impl TalosTool for ScreenSizeTool {
    fn name(&self) -> &'static str {
        "screen_size"
    }
    fn description(&self) -> &'static str {
        "Get the main screen dimensions (width and height in pixels)"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let script = r#"
                tell application "Finder"
                    set screenBounds to bounds of window of desktop
                    set screenWidth to item 3 of screenBounds
                    set screenHeight to item 4 of screenBounds
                end tell
                return (screenWidth as text) & "x" & (screenHeight as text)
            "#;

            let result = crate::run_applescript(script)?;
            let parts: Vec<&str> = result.split('x').collect();
            if parts.len() == 2 {
                let info = json!({
                    "width": parts[0].trim().parse::<i64>().unwrap_or(0),
                    "height": parts[1].trim().parse::<i64>().unwrap_or(0),
                    "raw": result,
                });
                Ok(serde_json::to_string_pretty(&info)?)
            } else {
                Ok(format!("Screen size: {}", result))
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            Ok("Screen size tool only available on macOS".to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// 15. WindowBoundsTool
// ---------------------------------------------------------------------------

/// Get the position and size of a window
pub struct WindowBoundsTool;

#[async_trait]
impl TalosTool for WindowBoundsTool {
    fn name(&self) -> &'static str {
        "window_bounds"
    }
    fn description(&self) -> &'static str {
        "Get the position and size of the frontmost window of an application"
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description()).with_param(
            "app",
            "string",
            "Application name (optional, defaults to frontmost app)",
            false,
        )
    }

    async fn execute(&self, args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let app_clause = if let Some(app) = args.get("app").and_then(|v| v.as_str()) {
                format!(
                    r#"application process "{}""#,
                    crate::sanitize_applescript(app)
                )
            } else {
                "first application process whose frontmost is true".to_string()
            };

            let script = format!(
                r#"
                tell application "System Events"
                    set targetApp to {}
                    set appName to name of targetApp
                    set winPos to position of window 1 of targetApp
                    set winSize to size of window 1 of targetApp
                    set posX to item 1 of winPos
                    set posY to item 2 of winPos
                    set sizeW to item 1 of winSize
                    set sizeH to item 2 of winSize
                end tell
                return (posX as text) & "," & (posY as text) & "," & (sizeW as text) & "," & (sizeH as text) & "," & appName
                "#,
                app_clause
            );

            let result = crate::run_applescript(&script)?;
            let parts: Vec<&str> = result.split(',').collect();
            if parts.len() >= 4 {
                let app_name = if parts.len() >= 5 {
                    parts[4..].join(",")
                } else {
                    "unknown".to_string()
                };
                let info = json!({
                    "app": app_name.trim(),
                    "x": parts[0].trim().parse::<i64>().unwrap_or(0),
                    "y": parts[1].trim().parse::<i64>().unwrap_or(0),
                    "width": parts[2].trim().parse::<i64>().unwrap_or(0),
                    "height": parts[3].trim().parse::<i64>().unwrap_or(0),
                });
                Ok(serde_json::to_string_pretty(&info)?)
            } else {
                Ok(format!("Window bounds: {}", result))
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = args;
            Ok("Window bounds tool only available on macOS".to_string())
        }
    }
}

// ── Extended UI automation tools ─────────────────────────────────────

/// Scroll at current mouse position or specified coordinates
pub struct UiScrollTool;

#[async_trait]
impl TalosTool for UiScrollTool {
    fn name(&self) -> &'static str {
        "ui_scroll"
    }
    fn description(&self) -> &'static str {
        "Scroll up or down at the current mouse position"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
            .with_param(
                "direction",
                "string",
                "Scroll direction: 'up' or 'down' (default 'down')",
                false,
            )
            .with_param(
                "amount",
                "integer",
                "Scroll amount in lines (default 3)",
                false,
            )
    }
    async fn execute(&self, args: Value) -> Result<String> {
        let direction = args
            .get("direction")
            .and_then(|v| v.as_str())
            .unwrap_or("down");
        let amount = args.get("amount").and_then(|v| v.as_i64()).unwrap_or(3);
        let scroll_amount = if direction == "up" { amount } else { -amount };

        #[cfg(target_os = "macos")]
        {
            let script = format!(
                r#"
                do shell script "python3 -c \"
import Quartz
event = Quartz.CGEventCreateScrollWheelEvent(None, Quartz.kCGScrollEventUnitLine, 1, {})
Quartz.CGEventPost(Quartz.kCGHIDEventTap, event)
\""#,
                scroll_amount
            );
            let output = tokio::process::Command::new("osascript")
                .arg("-e")
                .arg(&script)
                .output()
                .await
                .map_err(|e| Error::Tool(format!("Failed: {}", e)))?;
            if output.status.success() {
                Ok(format!("Scrolled {} by {} lines", direction, amount))
            } else {
                Err(Error::Tool(
                    String::from_utf8_lossy(&output.stderr).to_string(),
                ))
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = scroll_amount;
            Ok("ui_scroll only available on macOS".to_string())
        }
    }
}

/// Get current mouse position
pub struct UiGetMousePositionTool;

#[async_trait]
impl TalosTool for UiGetMousePositionTool {
    fn name(&self) -> &'static str {
        "ui_get_mouse_position"
    }
    fn description(&self) -> &'static str {
        "Get the current mouse cursor position (x, y coordinates)"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(), self.description())
    }

    async fn execute(&self, _args: Value) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let script = r#"
                do shell script "python3 -c \"
import Quartz
loc = Quartz.NSEvent.mouseLocation()
import AppKit
screen_h = AppKit.NSScreen.mainScreen().frame().size.height
print(f'{int(loc.x)},{int(screen_h - loc.y)}')
\""#;
            let output = tokio::process::Command::new("osascript")
                .arg("-e")
                .arg(script)
                .output()
                .await
                .map_err(|e| Error::Tool(format!("Failed: {}", e)))?;
            let coords = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let parts: Vec<&str> = coords.split(',').collect();
            if parts.len() == 2 {
                Ok(serde_json::to_string_pretty(&json!({
                    "x": parts[0].parse::<i64>().unwrap_or(0),
                    "y": parts[1].parse::<i64>().unwrap_or(0),
                }))?)
            } else {
                Ok(json!({ "raw": coords }).to_string())
            }
        }
        #[cfg(not(target_os = "macos"))]
        Ok("ui_get_mouse_position only available on macOS".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_modifiers_single() {
        let result = parse_modifiers("command").expect("should parse successfully");
        assert_eq!(result, "{command down}");
    }

    #[test]
    fn test_parse_modifiers_multiple() {
        let result = parse_modifiers("command, shift").expect("should parse successfully");
        assert_eq!(result, "{command down, shift down}");
    }

    #[test]
    fn test_parse_modifiers_aliases() {
        let result = parse_modifiers("cmd, alt, ctrl").expect("should parse successfully");
        assert_eq!(result, "{command down, option down, control down}");
    }

    #[test]
    fn test_parse_modifiers_empty() {
        let result = parse_modifiers("").expect("should parse successfully");
        assert_eq!(result, "{}");
    }

    #[test]
    fn test_parse_modifiers_invalid() {
        let result = parse_modifiers("super");
        assert!(result.is_err());
    }

    #[test]
    fn test_keystroke_schema() {
        let tool = KeystrokeTool;
        assert_eq!(tool.name(), "keystroke");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("should be an object");
        let props = params["properties"]
            .as_object()
            .expect("should be an object");
        assert!(props.contains_key("text"));
        assert!(props.contains_key("modifiers"));
        let required = params["required"].as_array().expect("should be an array");
        assert!(required.iter().any(|v| v.as_str() == Some("text")));
        assert!(!required.iter().any(|v| v.as_str() == Some("modifiers")));
    }

    #[test]
    fn test_key_code_schema() {
        let tool = KeyCodeTool;
        assert_eq!(tool.name(), "key_code");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("should be an object");
        let props = params["properties"]
            .as_object()
            .expect("should be an object");
        assert!(props.contains_key("code"));
        assert!(props.contains_key("modifiers"));
    }

    #[test]
    fn test_mouse_click_schema() {
        let tool = MouseClickTool;
        assert_eq!(tool.name(), "mouse_click");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("should be an object");
        let props = params["properties"]
            .as_object()
            .expect("should be an object");
        assert!(props.contains_key("x"));
        assert!(props.contains_key("y"));
        assert!(props.contains_key("button"));
    }

    #[test]
    fn test_mouse_move_schema() {
        let tool = MouseMoveTool;
        assert_eq!(tool.name(), "mouse_move");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("should be an object");
        let props = params["properties"]
            .as_object()
            .expect("should be an object");
        assert!(props.contains_key("x"));
        assert!(props.contains_key("y"));
    }

    #[test]
    fn test_activate_app_schema() {
        let tool = ActivateAppTool;
        assert_eq!(tool.name(), "activate_app");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("should be an object");
        let props = params["properties"]
            .as_object()
            .expect("should be an object");
        assert!(props.contains_key("name"));
    }

    #[test]
    fn test_window_list_schema() {
        let tool = WindowListTool;
        assert_eq!(tool.name(), "window_list");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("should be an object");
        let props = params["properties"]
            .as_object()
            .expect("should be an object");
        assert!(props.contains_key("app"));
    }

    #[test]
    fn test_window_resize_schema() {
        let tool = WindowResizeTool;
        assert_eq!(tool.name(), "window_resize");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("should be an object");
        let required = params["required"].as_array().expect("should be an array");
        assert!(required.iter().any(|v| v.as_str() == Some("app")));
        assert!(required.iter().any(|v| v.as_str() == Some("width")));
        assert!(required.iter().any(|v| v.as_str() == Some("height")));
    }

    #[test]
    fn test_window_move_schema() {
        let tool = WindowMoveTool;
        assert_eq!(tool.name(), "window_move");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("should be an object");
        let required = params["required"].as_array().expect("should be an array");
        assert!(required.iter().any(|v| v.as_str() == Some("app")));
        assert!(required.iter().any(|v| v.as_str() == Some("x")));
        assert!(required.iter().any(|v| v.as_str() == Some("y")));
    }

    #[test]
    fn test_window_minimize_schema() {
        let tool = WindowMinimizeTool;
        assert_eq!(tool.name(), "window_minimize");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("should be an object");
        let required = params["required"].as_array().expect("should be an array");
        assert!(required.iter().any(|v| v.as_str() == Some("app")));
    }

    #[test]
    fn test_window_close_schema() {
        let tool = WindowCloseTool;
        assert_eq!(tool.name(), "window_close");
    }

    #[test]
    fn test_window_fullscreen_schema() {
        let tool = WindowFullscreenTool;
        assert_eq!(tool.name(), "window_fullscreen");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("should be an object");
        let required = params["required"].as_array().expect("should be an array");
        assert!(required.iter().any(|v| v.as_str() == Some("app")));
    }

    #[test]
    fn test_menu_click_schema() {
        let tool = MenuClickTool;
        assert_eq!(tool.name(), "menu_click");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("should be an object");
        let required = params["required"].as_array().expect("should be an array");
        assert!(required.iter().any(|v| v.as_str() == Some("app")));
        assert!(required.iter().any(|v| v.as_str() == Some("menu")));
        assert!(required.iter().any(|v| v.as_str() == Some("item")));
    }

    #[test]
    fn test_type_text_schema() {
        let tool = TypeTextTool;
        assert_eq!(tool.name(), "type_text");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("should be an object");
        let props = params["properties"]
            .as_object()
            .expect("should be an object");
        assert!(props.contains_key("text"));
        assert!(props.contains_key("delay"));
        let required = params["required"].as_array().expect("should be an array");
        assert!(required.iter().any(|v| v.as_str() == Some("text")));
        assert!(!required.iter().any(|v| v.as_str() == Some("delay")));
    }

    #[test]
    fn test_screen_size_schema() {
        let tool = ScreenSizeTool;
        assert_eq!(tool.name(), "screen_size");
    }

    #[test]
    fn test_window_bounds_schema() {
        let tool = WindowBoundsTool;
        assert_eq!(tool.name(), "window_bounds");
        let schema = tool.schema();
        let params = schema.parameters.as_object().expect("should be an object");
        let props = params["properties"]
            .as_object()
            .expect("should be an object");
        assert!(props.contains_key("app"));
        // app is optional
        let required = params["required"].as_array().expect("should be an array");
        assert!(!required.iter().any(|v| v.as_str() == Some("app")));
    }
}
