//! WebDriver stealth and evasion techniques for bypassing bot detection.
//!
//! This module provides methods to inject scripts and modify browser properties
//! to evade detection by reCAPTCHA v3, Cloudflare, and similar bot detection systems.
//!
//! Key techniques:
//! - Override navigator.webdriver property
//! - Mock chrome runtime properties
//! - Inject realistic plugins and mimeTypes
//! - Randomize viewport and screen dimensions
//! - Spoof User-Agent and platform
//! - Modify permissions API
//! - Override CDP runtime flags

use serde_json::{Value, json};
use zeus_core::Result;

use crate::cdp::CdpClient;

/// Stealth script that overrides navigator.webdriver.
const WEBDRIVER_OVERRIDE: &str = r#"
Object.defineProperty(navigator, 'webdriver', {
  get: () => undefined
});
"#;

/// Stealth script for chrome runtime.
const CHROME_RUNTIME: &str = r#"
window.chrome = {
  runtime: {}
};
"#;

/// Stealth script for plugins and mimeTypes.
const PLUGINS_MIMETYPES: &str = r#"
Object.defineProperty(navigator, 'plugins', {
  get: () => [
    {
      0: {type: "application/pdf", suffixes: "pdf", description: "Portable Document Format"},
      description: "Portable Document Format",
      filename: "internal-pdf-viewer",
      length: 1,
      name: "Chrome PDF Plugin"
    },
    {
      0: {type: "application/x-google-chrome-pdf", suffixes: "pdf", description: "Portable Document Format"},
      description: "Portable Document Format",
      filename: "internal-pdf-viewer",
      length: 1,
      name: "Chrome PDF Viewer"
    },
    {
      0: {type: "application/x-nacl", suffixes: "", description: "Native Client Executable"},
      1: {type: "application/x-pnacl", suffixes: "", description: "Portable Native Client Executable"},
      description: "Native Client",
      filename: "internal-nacl-plugin",
      length: 2,
      name: "Native Client"
    }
  ]
});

Object.defineProperty(navigator, 'mimeTypes', {
  get: () => [
    {type: "application/pdf", suffixes: "pdf", description: "Portable Document Format"},
    {type: "application/x-google-chrome-pdf", suffixes: "pdf", description: "Portable Document Format"},
    {type: "application/x-nacl", suffixes: "", description: "Native Client Executable"},
    {type: "application/x-pnacl", suffixes: "", description: "Portable Native Client Executable"}
  ]
});
"#;

/// Stealth script for permissions API.
const PERMISSIONS_API: &str = r#"
const originalQuery = window.navigator.permissions.query;
window.navigator.permissions.query = (parameters) => (
  parameters.name === 'notifications' ?
    Promise.resolve({ state: Notification.permission }) :
    originalQuery(parameters)
);
"#;

/// Stealth script for languages.
const LANGUAGES: &str = r#"
Object.defineProperty(navigator, 'languages', {
  get: () => ['en-US', 'en']
});
"#;

/// Stealth script for WebGL vendor.
const WEBGL_VENDOR: &str = r#"
const getParameter = WebGLRenderingContext.prototype.getParameter;
WebGLRenderingContext.prototype.getParameter = function(parameter) {
  if (parameter === 37445) {
    return 'Intel Inc.';
  }
  if (parameter === 37446) {
    return 'Intel Iris OpenGL Engine';
  }
  return getParameter.call(this, parameter);
};
"#;

/// Stealth script for user-agent override.
fn user_agent_override(ua: &str) -> String {
    format!(
        r#"
Object.defineProperty(navigator, 'userAgent', {{
  get: () => '{}'
}});
"#,
        ua
    )
}

/// Stealth script for platform override.
fn platform_override(platform: &str) -> String {
    format!(
        r#"
Object.defineProperty(navigator, 'platform', {{
  get: () => '{}'
}});
"#,
        platform
    )
}

/// Stealth script for hardwareConcurrency override.
fn hardware_concurrency_override(cores: u8) -> String {
    format!(
        r#"
Object.defineProperty(navigator, 'hardwareConcurrency', {{
  get: () => {}
}});
"#,
        cores
    )
}

/// Stealth script for deviceMemory override.
fn device_memory_override(gb: u8) -> String {
    format!(
        r#"
Object.defineProperty(navigator, 'deviceMemory', {{
  get: () => {}
}});
"#,
        gb
    )
}

/// Stealth configuration.
#[derive(Debug, Clone)]
pub struct StealthConfig {
    /// User-Agent string to use.
    pub user_agent: Option<String>,
    /// Platform string (e.g., "MacIntel", "Win32").
    pub platform: Option<String>,
    /// Hardware concurrency (CPU cores).
    pub hardware_concurrency: Option<u8>,
    /// Device memory in GB.
    pub device_memory: Option<u8>,
    /// Viewport width.
    pub viewport_width: Option<u32>,
    /// Viewport height.
    pub viewport_height: Option<u32>,
}

impl Default for StealthConfig {
    fn default() -> Self {
        Self {
            user_agent: Some(
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36".to_string()
            ),
            platform: Some("MacIntel".to_string()),
            hardware_concurrency: Some(8),
            device_memory: Some(8),
            viewport_width: Some(1920),
            viewport_height: Some(1080),
        }
    }
}

impl CdpClient {
    /// Enable stealth mode with all evasion techniques.
    ///
    /// This injects multiple scripts to mask WebDriver presence and bypass
    /// common bot detection methods. Must be called BEFORE navigating to a page.
    pub async fn enable_stealth(&self, config: &StealthConfig) -> Result<()> {
        // 1. Override navigator.webdriver
        self.add_script_on_new_document(WEBDRIVER_OVERRIDE).await?;

        // 2. Mock chrome runtime
        self.add_script_on_new_document(CHROME_RUNTIME).await?;

        // 3. Inject realistic plugins and mimeTypes
        self.add_script_on_new_document(PLUGINS_MIMETYPES).await?;

        // 4. Fix permissions API
        self.add_script_on_new_document(PERMISSIONS_API).await?;

        // 5. Set languages
        self.add_script_on_new_document(LANGUAGES).await?;

        // 6. Override WebGL vendor
        self.add_script_on_new_document(WEBGL_VENDOR).await?;

        // 7. User-Agent override
        if let Some(ref ua) = config.user_agent {
            self.add_script_on_new_document(&user_agent_override(ua))
                .await?;
        }

        // 8. Platform override
        if let Some(ref platform) = config.platform {
            self.add_script_on_new_document(&platform_override(platform))
                .await?;
        }

        // 9. Hardware concurrency override
        if let Some(cores) = config.hardware_concurrency {
            self.add_script_on_new_document(&hardware_concurrency_override(cores))
                .await?;
        }

        // 10. Device memory override
        if let Some(gb) = config.device_memory {
            self.add_script_on_new_document(&device_memory_override(gb))
                .await?;
        }

        // 11. Set viewport size
        if let (Some(width), Some(height)) = (config.viewport_width, config.viewport_height) {
            self.set_viewport(width, height).await?;
        }

        // 12. Disable web security (allows cross-origin requests, useful for scraping)
        // Note: This requires Chrome to be launched with --disable-web-security flag

        Ok(())
    }

    /// Add a JavaScript snippet to be evaluated on every new document.
    ///
    /// This is executed before any page scripts run, making it ideal for
    /// injecting stealth overrides.
    async fn add_script_on_new_document(&self, source: &str) -> Result<Value> {
        self.send(
            "Page.addScriptToEvaluateOnNewDocument",
            Some(json!({ "source": source })),
        )
        .await
    }

    /// Set viewport size.
    async fn set_viewport(&self, width: u32, height: u32) -> Result<Value> {
        self.send(
            "Emulation.setDeviceMetricsOverride",
            Some(json!({
                "width": width,
                "height": height,
                "deviceScaleFactor": 1,
                "mobile": false
            })),
        )
        .await
    }

    /// Disable images to speed up page loads (useful for scraping).
    pub async fn disable_images(&self) -> Result<Value> {
        self.send(
            "Emulation.setDefaultBackgroundColorOverride",
            Some(json!({ "color": { "r": 0, "g": 0, "b": 0, "a": 0 } })),
        )
        .await?;

        self.send(
            "Network.setBlockedURLs",
            Some(json!({
                "urls": ["*.jpg", "*.jpeg", "*.png", "*.gif", "*.svg", "*.webp"]
            })),
        )
        .await
    }

    /// Set geolocation for the browser.
    pub async fn set_geolocation(
        &self,
        latitude: f64,
        longitude: f64,
        accuracy: u32,
    ) -> Result<Value> {
        self.send(
            "Emulation.setGeolocationOverride",
            Some(json!({
                "latitude": latitude,
                "longitude": longitude,
                "accuracy": accuracy
            })),
        )
        .await
    }

    /// Set timezone override.
    pub async fn set_timezone(&self, timezone_id: &str) -> Result<Value> {
        self.send(
            "Emulation.setTimezoneOverride",
            Some(json!({ "timezoneId": timezone_id })),
        )
        .await
    }

    /// Set locale override.
    pub async fn set_locale(&self, locale: &str) -> Result<Value> {
        self.send(
            "Emulation.setLocaleOverride",
            Some(json!({ "locale": locale })),
        )
        .await
    }

    /// Solve simple reCAPTCHA v2 by finding and clicking the checkbox.
    ///
    /// This is a basic helper and may not work for complex CAPTCHAs.
    pub async fn solve_recaptcha_v2(&self) -> Result<String> {
        // Wait for iframe to load
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

        // Click the reCAPTCHA checkbox
        let script = r#"
            const frames = document.querySelectorAll('iframe[src*="recaptcha"]');
            if (frames.length === 0) throw new Error('No reCAPTCHA iframe found');
            const frame = frames[0];
            const checkbox = frame.contentDocument.querySelector('.recaptcha-checkbox');
            if (!checkbox) throw new Error('Checkbox not found in iframe');
            checkbox.click();
            return 'Clicked reCAPTCHA checkbox';
        "#;

        self.evaluate(script).await?;
        Ok("reCAPTCHA v2 checkbox clicked".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stealth_config_default() {
        let config = StealthConfig::default();
        assert!(config.user_agent.is_some());
        assert!(config.platform.is_some());
        assert_eq!(config.hardware_concurrency, Some(8));
        assert_eq!(config.device_memory, Some(8));
        assert_eq!(config.viewport_width, Some(1920));
        assert_eq!(config.viewport_height, Some(1080));
    }

    #[test]
    fn test_user_agent_override_generation() {
        let script = user_agent_override("TestUA/1.0");
        assert!(script.contains("TestUA/1.0"));
        assert!(script.contains("Object.defineProperty"));
        assert!(script.contains("navigator"));
    }

    #[test]
    fn test_platform_override_generation() {
        let script = platform_override("Win32");
        assert!(script.contains("Win32"));
        assert!(script.contains("platform"));
    }

    #[test]
    fn test_hardware_concurrency_override() {
        let script = hardware_concurrency_override(4);
        assert!(script.contains("hardwareConcurrency"));
        assert!(script.contains("4"));
    }

    #[test]
    fn test_device_memory_override() {
        let script = device_memory_override(16);
        assert!(script.contains("deviceMemory"));
        assert!(script.contains("16"));
    }

    #[test]
    fn test_webdriver_override_script() {
        let script = WEBDRIVER_OVERRIDE.trim();
        assert!(script.contains("navigator"));
        assert!(script.contains("webdriver"));
        assert!(script.contains("undefined"));
    }

    #[test]
    fn test_chrome_runtime_script() {
        let script = CHROME_RUNTIME.trim();
        assert!(script.contains("window.chrome"));
        assert!(script.contains("runtime"));
    }

    #[test]
    fn test_plugins_script() {
        let script = PLUGINS_MIMETYPES.trim();
        assert!(script.contains("navigator"));
        assert!(script.contains("plugins"));
        assert!(script.contains("Chrome PDF Plugin"));
        assert!(script.contains("Native Client"));
    }

    #[test]
    fn test_permissions_api_script() {
        let script = PERMISSIONS_API.trim();
        assert!(script.contains("navigator"));
        assert!(script.contains("permissions"));
        assert!(script.contains("notifications"));
    }

    #[test]
    fn test_languages_script() {
        let script = LANGUAGES.trim();
        assert!(script.contains("navigator"));
        assert!(script.contains("languages"));
        assert!(script.contains("en-US"));
    }

    #[test]
    fn test_webgl_vendor_script() {
        let script = WEBGL_VENDOR.trim();
        assert!(script.contains("WebGLRenderingContext"));
        assert!(script.contains("Intel Inc."));
    }
}
