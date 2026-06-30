//! Channel Plugin System
//!
//! Provides a plugin architecture for transforming messages as they flow
//! through channel adapters. Plugins can normalize inbound messages,
//! transform outbound messages, and restrict actions per channel.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

// ============================================================================
// Plugin Trait
// ============================================================================

/// A channel plugin that can transform messages in the channel pipeline.
///
/// Plugins are applied in registration order. Each plugin gets to:
/// - Normalize inbound messages (strip HTML, fix encoding, etc.)
/// - Transform outbound messages (add footers, enforce limits, etc.)
/// - Declare which actions are allowed for a given channel type
pub trait ChannelPlugin: Send + Sync {
    /// Unique name of this plugin.
    fn name(&self) -> &str;

    /// Normalize an inbound message before it reaches the agent.
    ///
    /// Default: pass through unchanged.
    fn normalize_message(&self, content: &str, _channel_type: &str) -> String {
        content.to_string()
    }

    /// Transform an outbound message before it is sent to the channel.
    ///
    /// Default: pass through unchanged.
    fn transform_outbound(&self, content: &str, _channel_type: &str) -> String {
        content.to_string()
    }

    /// Return the list of actions this plugin allows for the given channel type.
    ///
    /// An empty vec means "no restrictions from this plugin" (all actions allowed).
    /// Non-empty means only those actions are permitted.
    fn allowed_actions(&self, _channel_type: &str) -> Vec<String> {
        vec![]
    }
}

// ============================================================================
// Plugin Registry
// ============================================================================

/// Registry of channel plugins. Thread-safe via RwLock<HashMap>.
pub struct ChannelPluginRegistry {
    plugins: RwLock<HashMap<String, Arc<dyn ChannelPlugin>>>,
    /// Insertion order for deterministic pipeline execution
    order: RwLock<Vec<String>>,
}

impl ChannelPluginRegistry {
    /// Create an empty plugin registry.
    pub fn new() -> Self {
        Self {
            plugins: RwLock::new(HashMap::new()),
            order: RwLock::new(Vec::new()),
        }
    }

    /// Register a plugin. If a plugin with the same name exists, it is replaced.
    pub fn register(&self, plugin: Arc<dyn ChannelPlugin>) {
        let name = plugin.name().to_string();
        if let Ok(mut plugins) = self.plugins.write() {
            let is_new = !plugins.contains_key(&name);
            plugins.insert(name.clone(), plugin);
            if is_new && let Ok(mut order) = self.order.write() {
                order.push(name);
            }
        }
    }

    /// Unregister a plugin by name. Returns true if it was found and removed.
    pub fn unregister(&self, name: &str) -> bool {
        let removed = if let Ok(mut plugins) = self.plugins.write() {
            plugins.remove(name).is_some()
        } else {
            false
        };
        if removed && let Ok(mut order) = self.order.write() {
            order.retain(|n| n != name);
        }
        removed
    }

    /// List all registered plugin names in registration order.
    pub fn list(&self) -> Vec<String> {
        self.order.read().map(|o| o.clone()).unwrap_or_default()
    }

    /// Number of registered plugins.
    pub fn len(&self) -> usize {
        self.plugins.read().map(|p| p.len()).unwrap_or(0)
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Run all plugins' normalize_message in order on the given content.
    pub fn normalize(&self, content: &str, channel_type: &str) -> String {
        let order = self.order.read().map(|o| o.clone()).unwrap_or_default();
        let plugins = self.plugins.read().ok();
        let mut result = content.to_string();
        if let Some(ref plugins) = plugins {
            for name in &order {
                if let Some(plugin) = plugins.get(name) {
                    result = plugin.normalize_message(&result, channel_type);
                }
            }
        }
        result
    }

    /// Run all plugins' transform_outbound in order on the given content.
    pub fn transform(&self, content: &str, channel_type: &str) -> String {
        let order = self.order.read().map(|o| o.clone()).unwrap_or_default();
        let plugins = self.plugins.read().ok();
        let mut result = content.to_string();
        if let Some(ref plugins) = plugins {
            for name in &order {
                if let Some(plugin) = plugins.get(name) {
                    result = plugin.transform_outbound(&result, channel_type);
                }
            }
        }
        result
    }

    /// Compute the intersection of allowed actions across all plugins.
    ///
    /// If no plugin restricts actions, returns None (all allowed).
    /// If any plugin restricts, returns the intersection of all restrictions.
    pub fn allowed_actions(&self, channel_type: &str) -> Option<Vec<String>> {
        let order = self.order.read().map(|o| o.clone()).unwrap_or_default();
        let plugins = self.plugins.read().ok()?;
        let mut result: Option<Vec<String>> = None;

        for name in &order {
            if let Some(plugin) = plugins.get(name) {
                let actions = plugin.allowed_actions(channel_type);
                if !actions.is_empty() {
                    result = Some(match result {
                        None => actions,
                        Some(existing) => existing
                            .into_iter()
                            .filter(|a| actions.contains(a))
                            .collect(),
                    });
                }
            }
        }

        result
    }
}

impl Default for ChannelPluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Built-in Plugins
// ============================================================================

/// Channel-specific message length limits.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelLimits {
    pub channel_type: String,
    pub max_length: usize,
}

/// Default message length limits per channel type.
fn default_channel_limits() -> Vec<ChannelLimits> {
    vec![
        ChannelLimits {
            channel_type: "telegram".into(),
            max_length: 4096,
        },
        ChannelLimits {
            channel_type: "discord".into(),
            max_length: 2000,
        },
        ChannelLimits {
            channel_type: "slack".into(),
            max_length: 40000,
        },
        ChannelLimits {
            channel_type: "sms".into(),
            max_length: 1600,
        },
        ChannelLimits {
            channel_type: "whatsapp".into(),
            max_length: 65536,
        },
        ChannelLimits {
            channel_type: "irc".into(),
            max_length: 512,
        },
        ChannelLimits {
            channel_type: "matrix".into(),
            max_length: 65536,
        },
        ChannelLimits {
            channel_type: "email".into(),
            max_length: 1_000_000,
        },
    ]
}

/// Get the max message length for a channel type.
fn channel_max_length(channel_type: &str) -> usize {
    for limit in default_channel_limits() {
        if limit.channel_type == channel_type {
            return limit.max_length;
        }
    }
    // Default fallback: 4096
    4096
}

/// Default normalizer plugin that:
/// - Strips HTML tags from inbound messages
/// - Normalizes whitespace (collapses multiple spaces/newlines)
/// - Truncates outbound messages to channel-specific limits
pub struct DefaultNormalizer;

impl ChannelPlugin for DefaultNormalizer {
    fn name(&self) -> &str {
        "default_normalizer"
    }

    fn normalize_message(&self, content: &str, _channel_type: &str) -> String {
        let stripped = strip_html(content);
        normalize_whitespace(&stripped)
    }

    fn transform_outbound(&self, content: &str, channel_type: &str) -> String {
        let max_len = channel_max_length(channel_type);
        truncate_to_limit(content, max_len)
    }

    fn allowed_actions(&self, _channel_type: &str) -> Vec<String> {
        // No restrictions — all actions allowed
        vec![]
    }
}

// ============================================================================
// Text Processing Helpers
// ============================================================================

/// Strip HTML tags from text, keeping content between tags.
fn strip_html(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut in_tag = false;

    for ch in input.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(ch),
            _ => {}
        }
    }

    // Decode common HTML entities
    result
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
}

/// Collapse multiple whitespace characters into single spaces, trim edges.
fn normalize_whitespace(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut prev_was_space = false;
    let mut prev_was_newline = false;

    for ch in input.chars() {
        if ch == '\n' {
            if !prev_was_newline {
                result.push('\n');
            }
            prev_was_newline = true;
            prev_was_space = false;
        } else if ch.is_whitespace() {
            if !prev_was_space && !prev_was_newline {
                result.push(' ');
            }
            prev_was_space = true;
        } else {
            result.push(ch);
            prev_was_space = false;
            prev_was_newline = false;
        }
    }

    result.trim().to_string()
}

/// Truncate text to a byte-safe character boundary, appending "…" if truncated.
fn truncate_to_limit(input: &str, max_len: usize) -> String {
    if input.len() <= max_len {
        return input.to_string();
    }

    // Find a safe UTF-8 boundary
    let mut end = max_len.saturating_sub(3); // Reserve space for "..."
    while end > 0 && !input.is_char_boundary(end) {
        end -= 1;
    }

    format!("{}...", &input[..end])
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ── strip_html ───────────────────────────────────────────────────────

    #[test]
    fn test_strip_html_basic() {
        assert_eq!(strip_html("<b>hello</b>"), "hello");
        assert_eq!(strip_html("<p>one</p><p>two</p>"), "onetwo");
        assert_eq!(strip_html("no tags here"), "no tags here");
    }

    #[test]
    fn test_strip_html_entities() {
        assert_eq!(strip_html("a &amp; b &lt; c"), "a & b < c");
        assert_eq!(strip_html("&quot;quoted&quot;"), "\"quoted\"");
        assert_eq!(strip_html("it&#39;s"), "it's");
    }

    // ── normalize_whitespace ─────────────────────────────────────────────

    #[test]
    fn test_normalize_whitespace() {
        assert_eq!(normalize_whitespace("hello   world"), "hello world");
        assert_eq!(normalize_whitespace("  leading"), "leading");
        assert_eq!(normalize_whitespace("trailing  "), "trailing");
        assert_eq!(normalize_whitespace("a\n\n\nb"), "a\nb");
    }

    // ── truncate_to_limit ────────────────────────────────────────────────

    #[test]
    fn test_truncate_short_message() {
        assert_eq!(truncate_to_limit("hello", 100), "hello");
    }

    #[test]
    fn test_truncate_long_message() {
        let long = "a".repeat(5000);
        let result = truncate_to_limit(&long, 4096);
        assert!(result.len() <= 4096);
        assert!(result.ends_with("..."));
    }

    // ── DefaultNormalizer ────────────────────────────────────────────────

    #[test]
    fn test_default_normalizer_inbound() {
        let plugin = DefaultNormalizer;
        let input = "<b>Hello</b>   <i>World</i>";
        let result = plugin.normalize_message(input, "telegram");
        assert_eq!(result, "Hello World");
    }

    #[test]
    fn test_default_normalizer_outbound_truncation() {
        let plugin = DefaultNormalizer;
        let long = "x".repeat(5000);
        let result = plugin.transform_outbound(&long, "discord");
        assert!(result.len() <= 2000);
        assert!(result.ends_with("..."));
    }

    // ── ChannelPluginRegistry ────────────────────────────────────────────

    #[test]
    fn test_registry_register_and_list() {
        let registry = ChannelPluginRegistry::new();
        assert!(registry.is_empty());

        registry.register(Arc::new(DefaultNormalizer));
        assert_eq!(registry.len(), 1);
        assert_eq!(registry.list(), vec!["default_normalizer"]);
    }

    #[test]
    fn test_registry_unregister() {
        let registry = ChannelPluginRegistry::new();
        registry.register(Arc::new(DefaultNormalizer));
        assert_eq!(registry.len(), 1);

        assert!(registry.unregister("default_normalizer"));
        assert!(registry.is_empty());
        assert!(registry.list().is_empty());

        // Unregister non-existent
        assert!(!registry.unregister("nonexistent"));
    }

    #[test]
    fn test_registry_normalize_pipeline() {
        let registry = ChannelPluginRegistry::new();
        registry.register(Arc::new(DefaultNormalizer));

        let result = registry.normalize("<b>Hello</b>   World", "telegram");
        assert_eq!(result, "Hello World");
    }

    #[test]
    fn test_registry_transform_pipeline() {
        let registry = ChannelPluginRegistry::new();
        registry.register(Arc::new(DefaultNormalizer));

        let long = "x".repeat(3000);
        let result = registry.transform(&long, "discord");
        assert!(result.len() <= 2000);
    }

    // ── Custom plugin ────────────────────────────────────────────────────

    struct FooterPlugin;

    impl ChannelPlugin for FooterPlugin {
        fn name(&self) -> &str {
            "footer"
        }

        fn transform_outbound(&self, content: &str, _channel_type: &str) -> String {
            format!("{}\n— Zeus", content)
        }

        fn allowed_actions(&self, channel_type: &str) -> Vec<String> {
            match channel_type {
                "sms" => vec!["send".to_string()],
                _ => vec![],
            }
        }
    }

    #[test]
    fn test_custom_plugin_pipeline() {
        let registry = ChannelPluginRegistry::new();
        registry.register(Arc::new(DefaultNormalizer));
        registry.register(Arc::new(FooterPlugin));

        let result = registry.transform("Hello", "telegram");
        assert_eq!(result, "Hello\n— Zeus");
    }

    #[test]
    fn test_allowed_actions_intersection() {
        let registry = ChannelPluginRegistry::new();
        registry.register(Arc::new(FooterPlugin));

        // SMS should be restricted
        let actions = registry.allowed_actions("sms");
        assert_eq!(actions, Some(vec!["send".to_string()]));

        // Telegram: no restrictions from FooterPlugin
        let actions = registry.allowed_actions("telegram");
        assert!(actions.is_none());
    }

    // ── Channel limits ───────────────────────────────────────────────────

    #[test]
    fn test_channel_limits() {
        assert_eq!(channel_max_length("telegram"), 4096);
        assert_eq!(channel_max_length("discord"), 2000);
        assert_eq!(channel_max_length("slack"), 40000);
        assert_eq!(channel_max_length("sms"), 1600);
        assert_eq!(channel_max_length("irc"), 512);
        // Unknown channel gets default
        assert_eq!(channel_max_length("unknown_channel"), 4096);
    }

    #[test]
    fn test_registry_replace_plugin() {
        let registry = ChannelPluginRegistry::new();
        registry.register(Arc::new(DefaultNormalizer));
        assert_eq!(registry.len(), 1);

        // Re-register same name — should replace, not duplicate
        registry.register(Arc::new(DefaultNormalizer));
        assert_eq!(registry.len(), 1);
        assert_eq!(registry.list().len(), 1);
    }
}
