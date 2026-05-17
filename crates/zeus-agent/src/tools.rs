//! 14 Core Tools
//!
//! 1. read_file - Read any file
//! 2. write_file - Create/overwrite
//! 3. edit_file - Search/replace
//! 4. list_dir - List directory
//! 5. shell - Execute commands
//! 6. web_fetch - Fetch URLs
//! 7. spawn - Background subagent
//! 8. message - Send to channel
//! 9. link_understanding - Analyze and understand URL content
//! 10. media_understanding - Analyze media files (images, audio, video)
//! 11. auto_reply - Configure automatic reply rules for channels
//! 12. polls - Create and manage polls across channels
//! 13. gmail_pubsub - Setup Gmail push notifications via Google Pub/Sub
//! 14. apply_patch  - Apply unified diff patches to files

use serde_json::Value;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use tokio::fs;
use tokio::process::Command;
use tracing::{debug, instrument};

// ============================================================================
// Global sandbox level for tool-level security checks
// ============================================================================
// 0 = None (skip all checks), 1 = Standard, 2 = Strict
static SANDBOX_LEVEL: AtomicU8 = AtomicU8::new(1);

/// Set the global sandbox level from agent config.
/// Called once during agent initialization.
#[allow(dead_code)]
pub fn set_sandbox_level(level: &str) {
    let val = match level.to_lowercase().as_str() {
        "none" => 0,
        "strict" => 2,
        _ => 1, // standard
    };
    SANDBOX_LEVEL.store(val, Ordering::Relaxed);
}

/// Returns true if sandbox_level is "none" — all tool-level checks should be skipped.
fn sandbox_is_none() -> bool {
    SANDBOX_LEVEL.load(Ordering::Relaxed) == 0
}
use zeus_browser::BrowserRegistry;
use zeus_core::{tool_err, Error, Result, ToolSchema};
use zeus_talos::TalosRegistry;

use crate::channels::Channel;
use zeus_channels::ChannelManager;

// ============================================================================
// Tool Registry
// ============================================================================

pub struct ToolRegistry {
    /// Optional Talos tool registry for native automation
    talos: Option<TalosRegistry>,
    /// Optional Browser tool registry for CDP browser automation
    browser: Option<BrowserRegistry>,
    /// Optional Trigger executor for cron-based background triggers
    trigger: Option<Arc<dyn zeus_core::TriggerExecutor>>,
    /// Optional ChannelManager for platform channels (Discord, Slack, etc.)
    /// Plain Arc — ChannelManager::send takes &self, no write-lock semantically needed.
    /// Aligns with Agent.channels type so set_shared_channels can propagate the same Arc.
    channels: Option<Arc<ChannelManager>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            talos: None,
            browser: None,
            trigger: None,
            channels: None,
        }
    }

    /// Create with default core tools only
    pub fn with_defaults() -> Self {
        Self::new()
    }

    /// Create with core tools + Talos automation tools
    pub fn with_talos(talos: TalosRegistry) -> Self {
        Self {
            talos: Some(talos),
            browser: None,
            trigger: None,
            channels: None,
        }
    }

    /// Create with core tools + Talos + Browser automation tools
    pub fn with_talos_and_browser(talos: TalosRegistry, browser: BrowserRegistry) -> Self {
        Self {
            talos: Some(talos),
            browser: Some(browser),
            trigger: None,
            channels: None,
        }
    }

    /// Create with core tools + Browser automation tools only
    pub fn with_browser(browser: BrowserRegistry) -> Self {
        Self {
            talos: None,
            browser: Some(browser),
            trigger: None,
            channels: None,
        }
    }

    /// Create with core tools + Trigger executor for cron scheduling
    pub fn with_trigger(trigger: Arc<dyn zeus_core::TriggerExecutor>) -> Self {
        Self {
            talos: None,
            browser: None,
            trigger: Some(trigger),
            channels: None,
        }
    }

    /// Set browser registry after construction
    pub fn set_browser(&mut self, browser: BrowserRegistry) {
        self.browser = Some(browser);
    }

    /// Set trigger executor after construction
    pub fn set_trigger(&mut self, trigger: Arc<dyn zeus_core::TriggerExecutor>) {
        self.trigger = Some(trigger);
    }

    /// Set channel manager after construction (enables platform channels like Discord)
    pub fn set_channels(&mut self, channels: Arc<ChannelManager>) {
        self.channels = Some(channels);
    }

    /// Get all tool schemas (core + talos + browser + trigger)
    pub fn schemas(&self) -> Vec<ToolSchema> {
        let mut schemas = self.core_schemas();
        if let Some(ref talos) = self.talos {
            schemas.extend(talos.schemas());
        }
        if let Some(ref browser) = self.browser {
            schemas.extend(browser.schemas());
        }
        // Trigger schemas are already included in core_schemas (create_trigger, list_triggers, remove_trigger)
        schemas
    }

    /// Get context-aware tool schemas based on what's configured and model size.
    ///
    /// Tiered loading for Ollama:
    /// - Tier 0: Model doesn't support tools → empty vec + negative prompt hint
    /// - Tier 1 (<14B params): Core tools only (8 essentials)
    /// - Tier 2 (14B-30B): Core + configured channels + git
    /// - Tier 3 (≥30B or non-Ollama): Full set based on config
    /// Message-aware tool schemas: for Ollama, filters tools by relevance
    /// to the user's message using keyword matching. Cloud providers get all tools.
    pub fn context_schemas_for_message(&self, config: &zeus_core::Config, message: &str) -> Vec<ToolSchema> {
        let (provider, _) = config.parse_model();
        let mut schemas = self.context_schemas(config);

        // Smart loading for non-cloud providers — filter by message relevance
        let needs_smart_loading = !matches!(provider,
            zeus_core::Provider::OpenAI | zeus_core::Provider::Anthropic |
            zeus_core::Provider::Google | zeus_core::Provider::GoogleGeminiCli |
            zeus_core::Provider::OpenRouter | zeus_core::Provider::Azure |
            zeus_core::Provider::Bedrock
        );
        // Tighter cap for GLM/ZAI — no parallel tool calls means each tool schema
        // costs tokens on every iteration. 5 tools instead of 8 saves ~1000 tokens/iter.
        let tool_cap = if provider == zeus_core::Provider::Zai { 5 } else { 8 };
        if needs_smart_loading && schemas.len() > tool_cap {
            schemas = Self::smart_filter_tools(schemas, config, Some(message));
            // Enforce the cap after filtering (smart_filter may still return > cap)
            if provider == zeus_core::Provider::Zai && schemas.len() > 5 {
                schemas.truncate(5);
            }
        }

        schemas
    }

    pub fn context_schemas(&self, config: &zeus_core::Config) -> Vec<ToolSchema> {
        let (provider, _) = config.parse_model();

        // Determine tool tier for Ollama based on model capabilities
        let tool_tier: u8 = if provider == zeus_core::Provider::Ollama {
            let model_name = config.model.split('/').last().unwrap_or(&config.model);
            let caps_cache = zeus_llm::ollama::CAPABILITIES_CACHE.lock().ok();
            if let Some(ref cache) = caps_cache {
                let key = (config.ollama.url.clone(), model_name.to_string());
                if let Some(Some(caps)) = cache.get(&key) {
                    if !caps.supports_tools {
                        tracing::info!("Ollama model {} doesn't support tools — text-only mode", model_name);
                        return Vec::new(); // Tier 0
                    }
                    // Determine tier from model family/size heuristic
                    // Models with "vision" in name or family with known large sizes get tier 3
                    if caps.family.contains("gemma4") || caps.family.contains("qwen35")
                        || caps.family.contains("glm4") || model_name.contains(":70b")
                        || model_name.contains(":35b") || model_name.contains(":26b")
                        || model_name.contains(":31b") {
                        3 // Large model — full tools
                    } else if model_name.contains(":14b") || model_name.contains(":13b")
                        || model_name.contains(":8b") || model_name.contains(":7b") {
                        1 // Small model — core only
                    } else {
                        2 // Medium or unknown — core + channels + git
                    }
                } else {
                    2 // Not in cache — assume medium
                }
            } else {
                2 // Cache unavailable — assume medium
            }
        } else {
            3 // Non-Ollama providers get full tools
        };

        tracing::debug!("Tool loading tier {} for model {}", tool_tier, config.model);

        // Tier 1: Core tools only (8 essentials) — small Ollama models
        if tool_tier == 1 {
            return self.core_schemas();
        }

        let mut schemas = self.core_schemas();

        if let Some(ref talos) = self.talos {
            // Tier 2: Core + essential dev tools (git, file ops, web search)
            let tier2_include = [
                "file_search", "find_files", "grep_files",
                "git_status", "git_log", "git_diff", "git_add", "git_commit",
                "web_search",
            ];

            // Tier 3: Full dev tools (everything from tier 2 + system + more)
            let tier3_include = [
                "file_search", "file_metadata", "file_copy", "file_move", "file_rename",
                "file_stat", "find_files", "directory_create", "file_append", "file_create",
                "file_delete", "grep_files", "head_file", "tail_file",
                "git_status", "git_log", "git_diff", "git_add", "git_commit",
                "git_push", "git_pull", "git_branch_list", "git_branch_create",
                "git_checkout", "git_stash", "git_stash_pop", "git_diff_stat",
                "git_clone",
                "web_search", "spotlight_search",
                "system_info", "process_list", "disk_usage", "memory_info",
                "clipboard_read", "clipboard_write", "screenshot",
            ];

            let always_include: &[&str] = if tool_tier >= 3 { &tier3_include } else { &tier2_include };

            // Channel tools — only if channel is configured
            let discord_configured = config.channels.as_ref()
                .and_then(|c| c.discord.as_ref())
                .map(|d| !d.token.is_empty())
                .unwrap_or(false);
            let telegram_configured = config.telegram_relay.is_some()
                || config.channels.as_ref()
                    .and_then(|c| c.telegram.as_ref())
                    .is_some();
            let slack_configured = config.channels.as_ref()
                .and_then(|c| c.slack.as_ref())
                .is_some();
            let email_configured = config.channels.as_ref()
                .and_then(|c| c.email.as_ref())
                .is_some();
            let matrix_configured = config.channels.as_ref()
                .and_then(|c| c.matrix.as_ref())
                .is_some();
            let signal_configured = config.channels.as_ref()
                .and_then(|c| c.signal.as_ref())
                .is_some();

            // Collect tool prefixes to include — tier 3 only for Apple tools
            let mut include_prefixes: Vec<&str> = Vec::new();
            if tool_tier >= 3 {
                // Apple tools only on macOS and only for large models
                #[cfg(target_os = "macos")]
                {
                    include_prefixes.extend_from_slice(&[
                        "calendar_", "reminders_", "notes_", "contacts_",
                        "music_",
                    ]);
                }
                include_prefixes.push("pdf_"); // PDF is cross-platform
            }
            if discord_configured { include_prefixes.push("discord_"); }
            if telegram_configured { include_prefixes.push("telegram_"); }
            if slack_configured { include_prefixes.push("slack_"); }
            if email_configured { include_prefixes.push("mail_"); }
            if matrix_configured { include_prefixes.push("matrix_"); }
            if signal_configured { include_prefixes.push("signal_"); }

            for schema in talos.schemas() {
                let name = &schema.name;
                if always_include.contains(&name.as_str()) {
                    schemas.push(schema);
                } else if include_prefixes.iter().any(|p| name.starts_with(p)) {
                    schemas.push(schema);
                }
                // Skip tools whose integration isn't configured
            }
        }

        if let Some(ref browser) = self.browser {
            schemas.extend(browser.schemas());
        }

        // Smart tool loading for Ollama: instead of sending all tools every request,
        // filter by relevance to the current task. Core tools always included,
        // domain tools loaded lazily based on what the model actually needs.
        // Cloud providers handle large tool sets server-side with no penalty.
        // Smart tool loading for non-cloud providers to avoid overwhelming the model.
        // Cloud providers (OpenAI, Anthropic, Google) handle large tool sets server-side.
        let needs_smart_loading = !matches!(provider,
            zeus_core::Provider::OpenAI | zeus_core::Provider::Anthropic |
            zeus_core::Provider::Google | zeus_core::Provider::GoogleGeminiCli |
            zeus_core::Provider::OpenRouter | zeus_core::Provider::Azure |
            zeus_core::Provider::Bedrock
        );
        if needs_smart_loading && schemas.len() > 8 {
            schemas = Self::smart_filter_tools(schemas, config, None);
        }

        schemas
    }

    /// Smart tool filtering for Ollama: always include core tools,
    /// lazy-load domain tools based on message keywords.
    /// Keeps prompt overhead minimal while giving the model full
    /// capability when it needs it.
    fn smart_filter_tools(schemas: Vec<ToolSchema>, config: &zeus_core::Config, message: Option<&str>) -> Vec<ToolSchema> {
        let msg_lower = message.unwrap_or("").to_lowercase();

        // Core tools always included (the 8 essentials)
        let core_names: &[&str] = &[
            "read_file", "write_file", "edit_file", "list_dir",
            "shell", "web_fetch", "spawn", "message",
        ];

        // Domain → (tool name prefixes/names, trigger keywords in message)
        let domains: &[(&[&str], &[&str])] = &[
            // Git tools — loaded when user mentions git, commit, branch, etc.
            (&["git_"], &["git", "commit", "branch", "merge", "push", "pull", "diff", "stash", "checkout", "repo"]),
            // Browser tools — loaded for web automation tasks
            (&["navigate", "click", "type_text", "get_text", "execute_js", "console_logs", "scroll", "wait"],
             &["browser", "chrome", "webpage", "website", "navigate", "click", "scrape", "automate"]),
            // Calendar tools
            (&["calendar_"], &["calendar", "event", "meeting", "schedule", "appointment"]),
            // Mail tools
            (&["mail_"], &["email", "mail", "inbox", "send email", "smtp", "imap"]),
            // Notes tools
            (&["notes_"], &["note", "notes", "apple notes", "obsidian"]),
            // Reminders
            (&["reminders_"], &["reminder", "reminders", "todo", "due"]),
            // Contacts
            (&["contacts_"], &["contact", "contacts", "phone number", "address book"]),
            // Music
            (&["music_"], &["music", "play", "song", "playlist", "spotify", "pause"]),
            // Screenshot / media
            (&["screenshot", "image_generate", "media_understanding", "transcribe"],
             &["screenshot", "image", "photo", "picture", "generate image", "transcribe", "audio"]),
            // System tools
            (&["system_info", "process_list", "cpu_info", "memory_info", "disk_usage", "battery"],
             &["system", "process", "cpu", "memory", "disk", "battery", "uptime"]),
            // Network
            (&["dns_", "ping", "port_check", "network_", "wifi_", "ip_"],
             &["network", "dns", "ping", "port", "wifi", "ip address"]),
            // Bluetooth
            (&["bluetooth_"], &["bluetooth", "airpods", "pair", "unpair"]),
            // Discord
            (&["discord_"], &["discord", "server", "channel"]),
            // Telegram
            (&["telegram_"], &["telegram"]),
            // Slack
            (&["slack_"], &["slack"]),
            // Signal
            (&["signal_"], &["signal"]),
            // PDF
            (&["pdf_"], &["pdf", "extract text", "merge pdf", "split pdf"]),
            // Safari
            (&["safari_"], &["safari", "bookmark", "tab"]),
        ];

        let mut selected: Vec<ToolSchema> = Vec::new();

        // Always include core tools
        for s in &schemas {
            if core_names.contains(&s.name.as_str()) {
                selected.push(s.clone());
            }
        }

        // Always include git tools (most common dev workflow)
        for s in &schemas {
            if s.name.starts_with("git_") && !selected.iter().any(|sel| sel.name == s.name) {
                selected.push(s.clone());
            }
        }

        // Lazy-load domain tools based on message keywords
        if !msg_lower.is_empty() {
            for (tool_prefixes, keywords) in domains {
                let domain_relevant = keywords.iter().any(|kw| msg_lower.contains(kw));
                if domain_relevant {
                    for s in &schemas {
                        let matches = tool_prefixes.iter().any(|prefix| {
                            if prefix.ends_with('_') {
                                s.name.starts_with(prefix)
                            } else {
                                s.name == *prefix
                            }
                        });
                        if matches && !selected.iter().any(|sel| sel.name == s.name) {
                            selected.push(s.clone());
                        }
                    }
                }
            }
        }

        // If no message context, include channel tools when configured
        if msg_lower.is_empty() {
            let ch = config.channels.as_ref();
            if ch.and_then(|c| c.discord.as_ref()).is_some() {
                for s in &schemas {
                    if s.name.starts_with("discord_") && !selected.iter().any(|sel| sel.name == s.name) {
                        selected.push(s.clone());
                    }
                }
            }
        }

        // Fill remaining slots up to max_tools
        let max = config.ollama.max_tools.unwrap_or(30);
        if max > 0 && selected.len() < max {
            for s in &schemas {
                if selected.len() >= max { break; }
                if !selected.iter().any(|sel| sel.name == s.name) {
                    selected.push(s.clone());
                }
            }
        }

        tracing::info!(
            "Ollama smart tool loading: {} available → {} selected (core + git + {} message-relevant)",
            schemas.len(), selected.len(),
            if msg_lower.is_empty() { "no message" } else { "keyword-matched" }
        );
        selected
    }

    /// Get core tool schemas only (8 essentials — used for Ollama lazy loading)
    pub fn core_schemas(&self) -> Vec<ToolSchema> {
        vec![
            ToolSchema::new("read_file", "Read the contents of a file")
                .with_param("path", "string", "Path to the file to read", true),

            ToolSchema::new("write_file", "Create or overwrite a file with content")
                .with_param("path", "string", "Path to the file to write", true)
                .with_param("content", "string", "Content to write to the file", true),

            ToolSchema::new("edit_file", "Search and replace text in a file")
                .with_param("path", "string", "Path to the file to edit", true)
                .with_param("search", "string", "Text to search for", true)
                .with_param("replace", "string", "Text to replace with", true)
                .with_param("all", "boolean", "Replace all occurrences (default: false)", false),

            ToolSchema::new("list_dir", "List contents of a directory")
                .with_param("path", "string", "Path to the directory to list", true)
                .with_param("recursive", "boolean", "List recursively (default: false)", false),

            ToolSchema::new("shell", "Execute a shell command")
                .with_param("command", "string", "The command to execute", true)
                .with_param("cwd", "string", "Working directory (optional)", false)
                .with_param("timeout", "integer", "Timeout in seconds (default: 60)", false),

            ToolSchema::new("web_fetch", "Fetch content from a URL. Returns page content plus structured metadata (title, description, Open Graph tags) for HTML pages.")
                .with_param("url", "string", "The URL to fetch", true)
                .with_param("method", "string", "HTTP method (default: GET)", false)
                .with_param("metadata_only", "boolean", "If true, return only structured metadata (title, description, OG tags) without full page content", false),

            ToolSchema::new("spawn", "Spawn a background subagent to handle a task. The subagent runs independently with its own context and tools. Use for parallelizable work or long-running tasks. Set gateway_url to dispatch to a remote Zeus gateway instead of running locally.")
                .with_param("task", "string", "Description of the task for the subagent", true)
                .with_param("context", "string", "Additional context for the subagent", false)
                .with_param("max_iterations", "integer", "Maximum iterations for subagent (default: 15)", false)
                .with_param("wait", "boolean", "Wait for completion and return result (default: false)", false)
                .with_param("gateway_url", "string", "Remote Zeus gateway URL to dispatch to (e.g. http://192.168.1.100:8080). Omit for local execution.", false)
                .with_param("auth_token", "string", "Bearer token for remote gateway authentication", false)
                .with_param("mission_id", "string", "Mission ID for result aggregation (set by Pantheon missions)", false),

            ToolSchema::new("collect_spawns", "Wait for all spawned background subagents to complete and return their collected results. Call this after spawning multiple agents to gather their outputs for synthesis. Returns a JSON array of subagent results with id, success, output, and iterations.")
                .with_param("timeout_seconds", "integer", "Maximum seconds to wait for all subagents (default: 300)", false),

            ToolSchema::new("message", "Send a message or file through a channel. Platform channels (require config): 'telegram', 'discord', 'slack', 'email', 'imessage', 'irc', 'matrix', 'whatsapp', 'signal', 'mattermost', 'x_twitter'. Simple channels: 'file' (writes to ~/.zeus/notifications.md), 'file:/path' (custom file), 'webhook:URL' (POST to URL), 'console' (print). To send a file attachment, provide 'attachment' with the file path.")
                .with_param("channel", "string", "Channel: 'telegram', 'discord', 'slack', 'email', 'imessage', 'file', 'file:/path', 'webhook:URL', 'x_twitter', or 'console'", true)
                .with_param("content", "string", "Message content (or caption when sending a file)", true)
                .with_param("target", "string", "Target: chat_id (telegram/discord/slack), email address, phone number, etc.", false)
                .with_param("attachment", "string", "Path to a file to send as attachment (audio, image, document, etc.)", false),

            ToolSchema::new("send_file", "Send a file (audio, image, document) to a channel. Audio files (.aiff, .wav, .mp3) sent to Discord are auto-converted to OGG/Opus voice messages. Use this tool to send attachments — do NOT use spawn for this.")
                .with_param("path", "string", "Path to the file to send", true)
                .with_param("channel", "string", "Channel: 'discord', 'telegram', 'slack'", true)
                .with_param("target", "string", "Channel/chat ID to send to", true)
                .with_param("caption", "string", "Optional caption/message text", false),

            ToolSchema::new("link_understanding", "Analyze and understand the content from a URL. Fetches the page, extracts text, and provides a structured analysis including title, main content, key points, and metadata.")
                .with_param("url", "string", "URL to analyze", true)
                .with_param("depth", "string", "Analysis depth: 'shallow' (title+summary), 'medium' (key points), 'deep' (full analysis). Default: medium", false)
                .with_param("focus", "string", "Optional focus area: 'technical', 'summary', 'facts', 'links'", false),

            ToolSchema::new("media_understanding", "Analyze media files (images, audio, video). For images: describe contents, extract text (OCR). For audio: transcribe speech. For video: describe scenes and extract audio.\n\nIMPORTANT — Vision-capable providers (XiaomiMimo, OpenAI GPT-4V, Claude, etc.):\nWhen the current LLM provider supports vision, DO NOT use this tool for image analysis. Instead, attach the image directly to your message as a multimodal content block (image_url with base64 data). This tool only performs local analysis (tesseract OCR, ffprobe) and cannot leverage the LLM's vision capabilities.\n\nWorkflow for images with vision-capable providers:\n1. Read the image file using read_file (returns base64)\n2. Include it as an image_url content block in your next message to the user\n3. The LLM will analyze the image directly with full vision capabilities\n\nUse this tool ONLY for:\n- Audio transcription (always local)\n- Video metadata extraction (always local)\n- Image OCR when specifically requested with analysis='ocr'\n- Providers that do NOT support vision")
                .with_param("path", "string", "Path to the media file", true)
                .with_param("media_type", "string", "Media type: 'image', 'audio', 'video', or 'auto' (detect from extension). Default: auto", false)
                .with_param("analysis", "string", "Analysis type: 'describe', 'ocr', 'transcribe', 'objects'. Default: describe", false),

            ToolSchema::new("auto_reply", "Configure automatic reply rules for channels. Set up rules that automatically respond to messages matching certain patterns or conditions.")
                .with_param("action", "string", "Action: 'add', 'remove', 'list', 'enable', 'disable'", true)
                .with_param("channel", "string", "Channel to apply rule to (telegram, discord, slack, email, etc.)", false)
                .with_param("pattern", "string", "Regex pattern to match incoming messages", false)
                .with_param("response", "string", "Auto-reply message to send", false)
                .with_param("rule_id", "string", "Rule ID for remove/enable/disable actions", false)
                .with_param("conditions", "object", "Additional conditions: {hours: '9-17', days: 'mon-fri', sender_pattern: '.*'}", false),

            ToolSchema::new("polls", "Create and manage polls across messaging channels. Support for single-choice, multi-choice, and timed polls.")
                .with_param("action", "string", "Action: 'create', 'close', 'results', 'list'", true)
                .with_param("channel", "string", "Channel to create poll in", false)
                .with_param("target", "string", "Chat/channel ID for the poll", false)
                .with_param("question", "string", "Poll question", false)
                .with_param("options", "array", "Array of poll options (strings)", false)
                .with_param("poll_id", "string", "Poll ID for close/results actions", false)
                .with_param("multi_select", "boolean", "Allow multiple selections (default: false)", false)
                .with_param("duration_minutes", "integer", "Auto-close after N minutes (optional)", false),

            ToolSchema::new("apply_patch", "Apply a unified diff patch to one or more files. Accepts standard unified diff format (as produced by `diff -u` or `git diff`).")
                .with_param("patch", "string", "The unified diff patch content to apply", true)
                .with_param("strip", "integer", "Number of leading path components to strip (default: 0, like patch -p0)", false)
                .with_param("dry_run", "boolean", "If true, only check if patch applies cleanly without modifying files (default: false)", false),

            ToolSchema::new("gmail_pubsub", "Setup Gmail push notifications via Google Cloud Pub/Sub. Receive real-time notifications when new emails arrive instead of polling.")
                .with_param("action", "string", "Action: 'setup', 'watch', 'stop', 'status', 'process'", true)
                .with_param("topic", "string", "Pub/Sub topic name (for setup)", false)
                .with_param("subscription", "string", "Pub/Sub subscription name", false)
                .with_param("labels", "array", "Gmail labels to watch (default: INBOX)", false)
                .with_param("webhook_url", "string", "Webhook URL for push notifications", false)
                .with_param("history_id", "string", "Gmail history ID for processing changes", false),

            ToolSchema::new("web_search", "Search the web using DuckDuckGo. Returns titles, URLs, and snippets for each result.")
                .with_param("query", "string", "Search query", true)
                .with_param("max_results", "integer", "Maximum results to return (default: 5, max: 20)", false),

            ToolSchema::new("deep_research", "Perform deep multi-step research on a topic. Decomposes the query into sub-queries, searches the web in parallel, fetches and reads multiple sources, then synthesizes findings into a comprehensive report with citations. Use for complex questions requiring multiple sources.")
                .with_param("query", "string", "The research question or topic to investigate", true)
                .with_param("max_sources", "integer", "Maximum sources to fetch per sub-query (default: 3)", false)
                .with_param("max_queries", "integer", "Maximum sub-queries to decompose into (default: 5)", false),

            ToolSchema::new("loop", "Schedule a self-message to wake the agent up after a delay and continue working autonomously. Use this to chain tasks, poll for completion, or keep working without human intervention.")
                .with_param("message", "string", "The message/task to send to yourself after the delay", true)
                .with_param("delay_seconds", "integer", "Seconds to wait before sending the self-message (default: 5, min: 1, max: 3600)", false)
                .with_param("condition", "string", "Optional condition description — the agent will only continue if this condition is met", false),

            ToolSchema::new("create_trigger", "Create a background scheduled trigger that runs a shell command on a cron schedule and injects the output as a system message into the agent context.")
                .with_param("name", "string", "Human-readable name for this trigger", true)
                .with_param("cron", "string", "Cron expression or human schedule (e.g. '*/5 * * * *', 'every 10 minutes', 'daily at 9am')", true)
                .with_param("command", "string", "Shell command to run when the trigger fires", true)
                .with_param("description", "string", "Optional description of what this trigger does", false),

            ToolSchema::new("list_triggers", "List all currently scheduled background triggers with their IDs, schedules, and status.")
                .with_param("channel", "string", "Channel to list triggers for (unused, for schema compatibility)", false),

            ToolSchema::new("remove_trigger", "Remove a previously created background trigger by its ID.")
                .with_param("id", "string", "The trigger ID to remove", true),
        ]
    }

    #[instrument(skip(self, args))]
    pub async fn execute(&self, name: &str, args: Value) -> Result<String> {
        // Try core tools first
        match name {
            "read_file" => return read_file(args).await,
            "write_file" => return write_file(args).await,
            "edit_file" => return edit_file(args).await,
            "list_dir" => return list_dir(args).await,
            "shell" => return shell(args).await,
            "web_fetch" => return web_fetch(args).await,
            "spawn" => return spawn(args).await,
            "collect_spawns" => {
                return Ok(
                    "collect_spawns requires agent context. Use through the agent loop."
                        .to_string(),
                );
            }
            "message" => {
                let channel_spec = args
                    .get("channel")
                    .and_then(|c| c.as_str())
                    .ok_or_else(|| tool_err!(validation, "Missing 'channel' argument"))?;
                let content = args
                    .get("content")
                    .and_then(|c| c.as_str())
                    .ok_or_else(|| tool_err!(validation, "Missing 'content' argument"))?;
                let target = args.get("target").and_then(|t| t.as_str());

                debug!(
                    "message: {} -> {:?} ({} chars)",
                    channel_spec,
                    target,
                    content.len()
                );

                // Use ChannelManager for platform channels when available (Discord, Slack, etc.)
                // This routes through zeus-channels with proper auth/adapter handling.
                // Fall back to simple Channel::parse for file/webhook/console.
                match channel_spec {
                    "telegram" | "discord" | "slack" | "email" | "imessage"
                    | "irc" | "matrix" | "whatsapp" | "signal" | "mattermost" | "mqtt"
                    | "x_twitter" | "x" => {
                        if let Some(ref channels) = self.channels {
                            let source = zeus_channels::ChannelSource::with_chat(
                                channel_spec,
                                "agent",
                                target.unwrap_or(""),
                            );
                            channels.send(&source, content).await?;
                            return Ok(format!("Message sent via {} to {}", channel_spec, target.unwrap_or("<missing target>")));
                        }
                        // Fall through to simple channel handler if no ChannelManager
                        let channel = Channel::parse(channel_spec);
                        return channel.send(content, target).await;
                    }
                    _ => {
                        let channel = Channel::parse(channel_spec);
                        return channel.send(content, target).await;
                    }
                }
            }
            "send_file" => {
                // send_file is intercepted upstream in Agent::execute_tools() before
                // reaching this registry. If we get here, it means send_file was called
                // outside the agent loop (e.g. from a subagent or skill context that
                // doesn't have channel access). Return a clear, actionable error.
                return Ok("send_file: no channel context available in this execution scope. \
                    File uploads must go through the agent loop (not subagents or skills). \
                    Write the file to disk and report its path instead.".to_string());
            }
            "link_understanding" => return link_understanding(args).await,
            "media_understanding" => return media_understanding(args).await,
            "auto_reply" => return auto_reply(args).await,
            "polls" => return polls(args).await,
            "gmail_pubsub" => return gmail_pubsub(args).await,
            "web_search" => return web_search_tool(args).await,
            "deep_research" => return deep_research_fallback(args).await,
            "apply_patch" => return apply_patch(args).await,
            "loop" => return loop_tool(args).await,
            "create_trigger" => {
                if let Some(ref trigger) = self.trigger {
                    return trigger.execute("create_trigger", &args).await.map_err(|e| e.into());
                }
                // Fallback: write trigger to ~/.zeus/triggers/ for the gateway to pick up
                let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("unnamed");
                let cron = args.get("cron").and_then(|v| v.as_str()).unwrap_or("*/5 * * * *");
                let command = args.get("command").and_then(|v| v.as_str()).unwrap_or("");
                if let Some(home) = dirs::home_dir() {
                    let triggers_dir = home.join(".zeus").join("triggers");
                    let _ = std::fs::create_dir_all(&triggers_dir);
                    let trigger_file = triggers_dir.join(format!("{}.toml", name));
                    let content = format!("name = \"{}\"\ncron = \"{}\"\ncommand = \"{}\"\nenabled = true\n", name, cron, command);
                    if let Err(e) = std::fs::write(&trigger_file, &content) {
                        return Ok(format!("Error creating trigger: {}", e));
                    }
                    return Ok(format!("Trigger '{}' created at {:?}. Schedule: '{}', Command: '{}'. Gateway will pick it up on next scheduler cycle.", name, trigger_file, cron, command));
                }
                return Ok("Error: could not find home directory".to_string());
            }
            "list_triggers" => {
                if let Some(ref trigger) = self.trigger {
                    return trigger.execute("list_triggers", &args).await.map_err(|e| e.into());
                }
                return Ok("No trigger scheduler available. Triggers are not connected to a live scheduler.".to_string());
            }
            "remove_trigger" => {
                if let Some(ref trigger) = self.trigger {
                    return trigger.execute("remove_trigger", &args).await.map_err(|e| e.into());
                }
                return Ok("No trigger scheduler available. Cannot remove triggers without a live scheduler.".to_string());
            }
            _ => {}
        }

        // Then try Talos tools
        if let Some(ref talos) = self.talos
            && talos.get(name).is_some()
        {
            return talos.execute(name, args).await;
        }

        // Then try Browser tools
        if let Some(ref browser) = self.browser
            && browser.get(name).is_some()
        {
            return browser.execute(name, args).await;
        }

        Err(tool_err!(not_found, "Unknown tool: {}", name))
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::with_defaults()
    }
}

// ============================================================================
// Helper function for external use
// ============================================================================

pub async fn execute_tool(name: &str, args: Value) -> Result<String> {
    let registry = ToolRegistry::with_defaults();
    registry.execute(name, args).await
}

// ============================================================================
// Path security
// ============================================================================

/// Validate and resolve a tool path, blocking path traversal attacks.
///
/// Resolves `..` components lexically and rejects paths that escape the
/// filesystem root or contain suspicious traversal patterns. Returns the
/// canonicalized path string for use in file operations.
fn validate_tool_path(path: &str) -> Result<String> {
    // Reject null bytes (can truncate paths in C-based filesystem calls)
    if path.contains('\0') {
        return Err(tool_err!(security, "Path contains null bytes"));
    }

    // Reject empty paths
    if path.trim().is_empty() {
        return Err(tool_err!(validation, "Path cannot be empty"));
    }

    // Expand ~ to home directory
    let expanded = if path.starts_with("~/") {
        if let Some(home) = dirs::home_dir() {
            home.join(&path[2..]).to_string_lossy().to_string()
        } else {
            path.to_string()
        }
    } else if path == "~" {
        dirs::home_dir().map(|h| h.to_string_lossy().to_string()).unwrap_or_else(|| path.to_string())
    } else {
        path.to_string()
    };

    let normalized = Path::new(&expanded);
    let mut resolved = std::path::PathBuf::new();

    for component in normalized.components() {
        match component {
            std::path::Component::ParentDir => {
                if !resolved.pop() {
                    return Err(tool_err!(security, "Path traversal denied: '{}'", path));
                }
            }
            other => resolved.push(other),
        }
    }

    // Block sensitive system paths
    let resolved_str = resolved.to_string_lossy();
    let blocked_prefixes = [
        "/etc/shadow",
        "/etc/master.passwd",
        "/etc/sudoers",
        "/proc/",
        "/sys/",
        "/dev/",
        "/boot/",
        "/private/etc/",
    ];
    let blocked_exact = ["/etc/passwd"];

    for prefix in &blocked_prefixes {
        if resolved_str.starts_with(prefix) {
            return Err(tool_err!(security, "Access to '{}' is blocked by security policy",
                path));
        }
    }
    for exact in &blocked_exact {
        if resolved_str.as_ref() == *exact {
            return Err(tool_err!(security, "Access to '{}' is blocked by security policy",
                path));
        }
    }

    // If the path exists, resolve symlinks to prevent symlink-based traversal
    let final_path = if resolved.exists() {
        match resolved.canonicalize() {
            Ok(canonical) => {
                let canonical_str = canonical.to_string_lossy();
                // Re-check the canonical (symlink-resolved) path
                for prefix in &blocked_prefixes {
                    if canonical_str.starts_with(prefix) {
                        return Err(tool_err!(security, "Access to '{}' is blocked by security policy (symlink target)",
                            path));
                    }
                }
                for exact in &blocked_exact {
                    if canonical_str.as_ref() == *exact {
                        return Err(tool_err!(security, "Access to '{}' is blocked by security policy (symlink target)",
                            path));
                    }
                }
                canonical.to_string_lossy().into_owned()
            }
            Err(_) => resolved.to_string_lossy().into_owned(),
        }
    } else {
        resolved.to_string_lossy().into_owned()
    };

    Ok(final_path)
}

// ============================================================================
// 1. read_file
// ============================================================================

async fn read_file(args: Value) -> Result<String> {
    let raw_path = args
        .get("path")
        .and_then(|p| p.as_str())
        .ok_or_else(|| tool_err!(validation, "Missing 'path' argument"))?;

    let path = validate_tool_path(raw_path)?;
    debug!("read_file: {}", path);

    let content = fs::read_to_string(&path)
        .await
        .map_err(|e| tool_err!(tool, "Failed to read {}: {}", path, e))?;

    // Limit output size
    if content.len() > zeus_core::MAX_CONTENT_BYTES {
        Ok(format!(
            "{}\n\n... (truncated, {} total bytes)",
            zeus_core::truncate_str(&content, zeus_core::MAX_CONTENT_BYTES),
            content.len()
        ))
    } else {
        Ok(content)
    }
}

// ============================================================================
// 2. write_file
// ============================================================================

async fn write_file(args: Value) -> Result<String> {
    let raw_path = args
        .get("path")
        .and_then(|p| p.as_str())
        .ok_or_else(|| tool_err!(validation, "Missing 'path' argument"))?;

    let content = args
        .get("content")
        .and_then(|c| c.as_str())
        .ok_or_else(|| tool_err!(validation, "Missing 'content' argument"))?;

    let path = validate_tool_path(raw_path)?;
    debug!("write_file: {} ({} bytes)", path, content.len());

    // Create parent directories if needed
    if let Some(parent) = Path::new(&path).parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .await
            .map_err(|e| tool_err!(tool, "Failed to create directories: {}", e))?;
    }

    fs::write(path.as_str(), content)
        .await
        .map_err(|e| tool_err!(tool, "Failed to write {}: {}", path, e))?;

    Ok(format!("Wrote {} bytes to {}", content.len(), path))
}

// ============================================================================
// 3. edit_file
// ============================================================================

async fn edit_file(args: Value) -> Result<String> {
    let raw_path = args
        .get("path")
        .and_then(|p| p.as_str())
        .ok_or_else(|| tool_err!(validation, "Missing 'path' argument"))?;

    let path = validate_tool_path(raw_path)?;

    let search = args
        .get("search")
        .and_then(|s| s.as_str())
        .ok_or_else(|| tool_err!(validation, "Missing 'search' argument"))?;

    let replace = args
        .get("replace")
        .and_then(|r| r.as_str())
        .ok_or_else(|| tool_err!(validation, "Missing 'replace' argument"))?;

    let replace_all = args.get("all").and_then(|a| a.as_bool()).unwrap_or(false);

    debug!(
        "edit_file: {} (search: {} chars, replace: {} chars, all: {})",
        path,
        search.len(),
        replace.len(),
        replace_all
    );

    let content = fs::read_to_string(path.as_str())
        .await
        .map_err(|e| tool_err!(tool, "Failed to read {}: {}", path, e))?;

    if !content.contains(search) {
        return Err(tool_err!(not_found, "Search text not found in {}", path));
    }

    let (new_content, count) = if replace_all {
        let count = content.matches(search).count();
        (content.replace(search, replace), count)
    } else {
        (content.replacen(search, replace, 1), 1)
    };

    fs::write(path.as_str(), &new_content)
        .await
        .map_err(|e| tool_err!(tool, "Failed to write {}: {}", path, e))?;

    Ok(format!("Replaced {} occurrence(s) in {}", count, path))
}

// ============================================================================
// 4. list_dir
// ============================================================================

async fn list_dir(args: Value) -> Result<String> {
    let raw_path = args
        .get("path")
        .and_then(|p| p.as_str())
        .ok_or_else(|| tool_err!(validation, "Missing 'path' argument"))?;

    let path = validate_tool_path(raw_path)?;

    let recursive = args
        .get("recursive")
        .and_then(|r| r.as_bool())
        .unwrap_or(false);

    debug!("list_dir: {} (recursive: {})", path, recursive);

    if recursive {
        list_recursive(Path::new(&path), Path::new(&path), 0).await
    } else {
        list_single(Path::new(&path)).await
    }
}

async fn list_single(path: &Path) -> Result<String> {
    let mut entries = fs::read_dir(path)
        .await
        .map_err(|e| tool_err!(tool, "Failed to read {}: {}", path.display(), e))?;

    let mut items = Vec::new();
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| tool_err!(tool, "Failed to read entry: {}", e))?
    {
        let name = entry.file_name().to_string_lossy().to_string();
        let file_type = entry.file_type().await.ok();
        let suffix = if file_type.map(|t| t.is_dir()).unwrap_or(false) {
            "/"
        } else {
            ""
        };
        items.push(format!("{}{}", name, suffix));
    }

    items.sort();
    Ok(items.join("\n"))
}

async fn list_recursive(base: &Path, path: &Path, depth: usize) -> Result<String> {
    if depth > 10 {
        return Ok("(max depth reached)".to_string());
    }

    let mut result = Vec::new();
    let mut entries = fs::read_dir(path)
        .await
        .map_err(|e| tool_err!(tool, "Failed to read {}: {}", path.display(), e))?;

    let mut items = Vec::new();
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| tool_err!(tool, "Failed to read entry: {}", e))?
    {
        items.push(entry);
    }

    items.sort_by_key(|a| a.file_name());

    for entry in items {
        let name = entry.file_name().to_string_lossy().to_string();
        let relative = entry
            .path()
            .strip_prefix(base)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or(name.clone());

        let file_type = entry.file_type().await.ok();
        if file_type.map(|t| t.is_dir()).unwrap_or(false) {
            result.push(format!("{}/", relative));
            if let Ok(sub) = Box::pin(list_recursive(base, &entry.path(), depth + 1)).await {
                result.push(sub);
            }
        } else {
            result.push(relative);
        }
    }

    Ok(result.join("\n"))
}

// ============================================================================
// 5. shell
// ============================================================================

/// Validate a shell command for dangerous patterns.
///
/// Blocks commands that attempt to:
/// - Access sensitive system files directly
/// - Use shell injection via backtick or $() substitution in dangerous contexts
/// - Execute destructive operations against critical system paths
fn validate_shell_command(command: &str) -> Result<()> {
    // Reject null bytes which can truncate strings in C-based tools
    if command.contains('\0') {
        return Err(tool_err!(security, "Shell command contains null bytes"));
    }

    // Cap command length to prevent abuse (always checked)
    if command.len() > 10_000 {
        return Err(Error::Tool(
            "Shell command too long (max 10,000 characters)".to_string(),
        ));
    }

    // When sandbox_level = "none", skip all other security checks.
    // Only null bytes and length are enforced (absolute minimum safety).
    if sandbox_is_none() {
        return Ok(());
    }

    // Block direct access to sensitive system files
    let sensitive_paths = [
        "/etc/shadow",
        "/etc/master.passwd",
        "/etc/sudoers",
        "/proc/kcore",
    ];
    for path in &sensitive_paths {
        if command.contains(path) {
            return Err(tool_err!(security, "Shell command blocked: access to '{}' is restricted",
                path));
        }
    }

    // Block destructive patterns targeting system-critical paths.
    // We check that the destructive command (e.g. "rm") actually targets a
    // system path as its argument — not just that both substrings appear
    // somewhere in the command. This prevents false positives like:
    //   "rm -f ~/.zeus/gateway.pid; /usr/local/bin/zeus gateway"
    // where "rm " and "/usr" both appear but rm doesn't target /usr.
    let cmd_lower = command.to_lowercase();

    // For rm: check each semicolon/&&/||-separated segment independently
    let rm_protected_paths = [
        "/-", "/bin", "/usr", "/etc", "/sbin", "/system", "/library",
    ];
    for segment in cmd_lower.split(|c| c == ';' || c == '|' || c == '&') {
        let seg = segment.trim();
        if seg.starts_with("rm ") || seg.contains(" rm ") {
            for path in &rm_protected_paths {
                if seg.contains(&format!("rm {}", path))
                    || seg.contains(&format!("rm -rf {}", path))
                    || seg.contains(&format!("rm -f {}", path))
                    || seg.contains(&format!("rm -r {}", path))
                {
                    return Err(Error::Tool(
                        "Shell command blocked: destructive operation on system path"
                            .to_string(),
                    ));
                }
            }
        }
    }

    // For mkfs/dd: these are always dangerous with /dev/ targets
    if cmd_lower.contains("mkfs") && cmd_lower.contains("/dev/") {
        return Err(Error::Tool(
            "Shell command blocked: destructive operation on system path".to_string(),
        ));
    }
    if cmd_lower.contains("dd ") && cmd_lower.contains("of=/dev/") {
        return Err(Error::Tool(
            "Shell command blocked: destructive operation on system path".to_string(),
        ));
    }

    // Block environment variable injection via dangerous override vars.
    // These can hijack shell initialization, the dynamic linker, or
    // Python/Ruby/Perl import paths — all known privilege escalation vectors.
    //
    // Note: "env=" is intentionally NOT in this list as a bare substring because
    // it would match legitimate *_ENV= patterns like RAILS_ENV=, NODE_ENV=, MIX_ENV=.
    // The POSIX ENV= shell init var is checked separately with word-boundary logic below.
    let env_injection_vars = [
        "bash_env=",
        "ld_preload=",
        "ld_library_path=",
        "dyld_insert_libraries=",
        "dyld_library_path=",
        "dyld_framework_path=",
        "cdpath=",
        "ifs=",
        "pythonpath=",
        "rubylib=",
        "perl5lib=",
        "node_options=",
    ];
    for var in &env_injection_vars {
        if cmd_lower.contains(var) {
            return Err(tool_err!(security, "Shell command blocked: environment variable injection via '{}'",
                var.trim_end_matches('=').to_uppercase()));
        }
    }

    // Block bare ENV= (POSIX shell init file override) but only as a standalone
    // var — not as a suffix like RAILS_ENV= or NODE_ENV=.
    // Match: start of command, or preceded by whitespace/semicolon.
    let env_standalone = cmd_lower.starts_with("env=")
        || cmd_lower.contains(" env=")
        || cmd_lower.contains("\tenv=")
        || cmd_lower.contains(";env=")
        || cmd_lower.contains("&&env=");
    if env_standalone {
        return Err(Error::Tool(
            "Shell command blocked: environment variable injection via 'ENV'".to_string(),
        ));
    }

    // Line-continuation sequences (\<newline>) are allowed — they're standard
    // shell syntax for multi-line commands. The earlier pattern-based checks
    // already catch dangerous commands regardless of line continuation.

    // Cap command length to prevent abuse
    if command.len() > 10_000 {
        return Err(Error::Tool(
            "Shell command too long (max 10,000 characters)".to_string(),
        ));
    }

    Ok(())
}

async fn shell(args: Value) -> Result<String> {
    let command = args
        .get("command")
        .and_then(|c| c.as_str())
        .ok_or_else(|| tool_err!(validation, "Missing 'command' argument"))?;

    let cwd = args.get("cwd").and_then(|c| c.as_str());
    let timeout_secs = args.get("timeout").and_then(|t| t.as_u64()).unwrap_or(60);

    // Validate command before execution
    validate_shell_command(command)?;

    debug!(
        "shell: {} (cwd: {:?}, timeout: {}s)",
        command, cwd, timeout_secs
    );

    // Use login shell (-l) so PATH includes ~/.cargo/bin, homebrew, etc.
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "sh".to_string());
    let mut cmd = Command::new(&shell);
    cmd.arg("-lc").arg(command);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    if let Some(dir) = cwd {
        let validated_dir = validate_tool_path(dir)?;
        cmd.current_dir(validated_dir);
    }

    let output = tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), cmd.output())
        .await
        .map_err(|_| tool_err!(timeout, "Command timed out after {}s", timeout_secs))?
        .map_err(|e| tool_err!(tool, "Failed to execute command: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    let mut result = String::new();
    if !stdout.is_empty() {
        result.push_str(&stdout);
    }
    if !stderr.is_empty() {
        if !result.is_empty() {
            result.push_str("\n--- stderr ---\n");
        }
        result.push_str(&stderr);
    }

    if output.status.success() {
        Ok(result)
    } else {
        let code = output.status.code().unwrap_or(-1);
        Err(tool_err!(tool, "Command exited with code {}\n{}",
            code, result))
    }
}

// ============================================================================
// 6. web_fetch
// ============================================================================

/// Validate a URL for safe fetching.
///
/// Blocks:
/// - Non-HTTP(S) schemes (file://, ftp://, gopher://, etc.)
/// - URLs targeting private/internal IP ranges (SSRF prevention)
/// - URLs with userinfo components used for host confusion
/// - Excessively long URLs
fn validate_fetch_url(url: &str) -> Result<()> {
    // Reject null bytes
    if url.contains('\0') {
        return Err(tool_err!(security, "URL contains null bytes"));
    }

    // Cap length
    if url.len() > 4096 {
        return Err(Error::Tool(
            "URL too long (max 4096 characters)".to_string(),
        ));
    }

    // Require http:// or https:// scheme
    let url_lower = url.to_lowercase();
    if !url_lower.starts_with("http://") && !url_lower.starts_with("https://") {
        return Err(tool_err!(validation, "Only http:// and https:// URLs are allowed, got: {}",
            url.chars().take(50).collect::<String>()));
    }

    // Parse to extract host
    let after_scheme = if url_lower.starts_with("https://") {
        &url[8..]
    } else {
        &url[7..]
    };

    let authority = after_scheme.split('/').next().unwrap_or("");

    // Reject userinfo in URL (commonly used for confusion attacks)
    if authority.contains('@') {
        return Err(Error::Tool(
            "URLs with userinfo (user@host) are not allowed".to_string(),
        ));
    }

    // Extract host (strip port)
    let host = if authority.starts_with('[') {
        // IPv6: [::1]:port
        authority
            .split(']')
            .next()
            .unwrap_or("")
            .trim_start_matches('[')
    } else {
        authority.split(':').next().unwrap_or("")
    };

    if host.is_empty() {
        return Err(tool_err!(validation, "URL has empty host"));
    }

    // Block private/internal IP ranges (SSRF prevention)
    // Note: loopback (127.0.0.1, ::1) is allowed — Zeus services run locally
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        let is_private = match ip {
            std::net::IpAddr::V4(v4) => {
                v4.is_private()        // 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16
                    || v4.is_link_local()  // 169.254.0.0/16
                    || v4.is_unspecified() // 0.0.0.0
                    || v4.octets()[0] == 100 && (v4.octets()[1] & 0xC0) == 64 // 100.64.0.0/10 (CGN)
            }
            std::net::IpAddr::V6(v6) => {
                let segs = v6.segments();
                v6.is_unspecified() // ::
                    // NAT64 well-known prefix
                    || (segs[0] == 0x0064 && segs[1] == 0xff9b && segs[2] == 0 && segs[3] == 0)
                    // 6to4 (2002::/16)
                    || segs[0] == 0x2002
                    // Teredo (2001:0000::/32)
                    || (segs[0] == 0x2001 && segs[1] == 0x0000)
                    // IPv6-mapped IPv4 private (::ffff:10.x, ::ffff:192.168.x, etc.)
                    || {
                        if let Some(v4) = v6.to_ipv4_mapped() {
                            v4.is_private() || v4.is_link_local() || v4.is_unspecified()
                        } else {
                            false
                        }
                    }
                    // Unique local (fc00::/7)
                    || (segs[0] & 0xfe00) == 0xfc00
                    // Link-local (fe80::/10)
                    || (segs[0] & 0xffc0) == 0xfe80
            }
        };
        if is_private {
            return Err(tool_err!(security, "URLs targeting private/internal IP addresses are blocked: {}",
                host));
        }
    }

    // Block common internal hostnames
    let host_lower = host.to_lowercase();
    // Note: localhost is NOT blocked — Zeus services (Ollama, Whisper, Piper, ComfyUI) run locally
    let internal_hosts = ["metadata.google.internal", "169.254.169.254"];
    for internal in &internal_hosts {
        if host_lower == *internal {
            return Err(tool_err!(security, "URLs targeting internal host '{}' are blocked",
                host));
        }
    }

    Ok(())
}

async fn web_fetch(args: Value) -> Result<String> {
    let url = args
        .get("url")
        .and_then(|u| u.as_str())
        .ok_or_else(|| tool_err!(validation, "Missing 'url' argument"))?;

    let method = args.get("method").and_then(|m| m.as_str()).unwrap_or("GET");

    // Validate URL before fetching
    validate_fetch_url(url)?;

    // DNS-based SSRF check: resolve hostname and verify IPs are public
    {
        let parsed =
            url::Url::parse(url).map_err(|e| tool_err!(network, "Invalid URL: {}", e))?;
        if let Some(host) = parsed.host_str() {
            validate_resolved_ips(host).await?;
        }
    }

    debug!("web_fetch: {} {}", method, url);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| tool_err!(network, "Failed to create client: {}", e))?;

    let request = match method.to_uppercase().as_str() {
        "GET" => client.get(url),
        "POST" => client.post(url),
        "PUT" => client.put(url),
        "DELETE" => client.delete(url),
        "HEAD" => client.head(url),
        _ => return Err(tool_err!(validation, "Unsupported method: {}", method)),
    };

    let response = request
        .send()
        .await
        .map_err(|e| tool_err!(network, "Request failed: {}", e))?;

    let status = response.status();
    let is_html = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|ct| ct.to_lowercase().contains("text/html"))
        .unwrap_or(false);
    let text = response
        .text()
        .await
        .map_err(|e| tool_err!(network, "Failed to read response: {}", e))?;

    let metadata_only = args
        .get("metadata_only")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    if status.is_success() {
        if is_html {
            let metadata = extract_page_metadata(&text);
            let stripped = strip_hidden_html_content(&text);

            if metadata_only {
                // Return only structured metadata
                return Ok(serde_json::to_string_pretty(&metadata)
                    .unwrap_or_else(|_| format!("{:?}", metadata)));
            }

            // Prepend metadata header to content
            let mut result = String::new();
            if !metadata.title.is_empty() {
                result.push_str(&format!("Title: {}\n", metadata.title));
            }
            if !metadata.description.is_empty() {
                result.push_str(&format!("Description: {}\n", metadata.description));
            }
            if !metadata.og_type.is_empty() {
                result.push_str(&format!("Type: {}\n", metadata.og_type));
            }
            if !metadata.og_image.is_empty() {
                result.push_str(&format!("Image: {}\n", metadata.og_image));
            }
            if !metadata.canonical_url.is_empty() {
                result.push_str(&format!("Canonical: {}\n", metadata.canonical_url));
            }
            if !result.is_empty() {
                result.push_str("---\n");
            }
            result.push_str(&stripped);

            // Limit response size
            if result.len() > zeus_core::MAX_CONTENT_BYTES {
                Ok(format!(
                    "{}\n\n... (truncated, {} total bytes)",
                    zeus_core::truncate_str(&result, zeus_core::MAX_CONTENT_BYTES),
                    result.len()
                ))
            } else {
                Ok(result)
            }
        } else {
            // Non-HTML: return as-is
            if text.len() > zeus_core::MAX_CONTENT_BYTES {
                Ok(format!(
                    "{}\n\n... (truncated, {} total bytes)",
                    zeus_core::truncate_str(&text, zeus_core::MAX_CONTENT_BYTES),
                    text.len()
                ))
            } else {
                Ok(text)
            }
        }
    } else {
        Err(tool_err!(network, "HTTP {} - {}", status, text))
    }
}

/// Structured metadata extracted from an HTML page.
#[derive(Debug, serde::Serialize)]
struct PageMetadata {
    title: String,
    description: String,
    og_type: String,
    og_image: String,
    og_site_name: String,
    canonical_url: String,
    author: String,
}

/// Extract structured metadata from HTML (title, meta tags, Open Graph).
fn extract_page_metadata(html: &str) -> PageMetadata {
    use regex::Regex;
    use std::sync::OnceLock;

    static TITLE_RE: OnceLock<Regex> = OnceLock::new();
    static META_RE: OnceLock<Regex> = OnceLock::new();
    static CANONICAL_RE: OnceLock<Regex> = OnceLock::new();

    let title_re = TITLE_RE
        .get_or_init(|| Regex::new(r"(?is)<title[^>]*>(.*?)</title\s*>").expect("valid regex"));
    let meta_re = META_RE
        .get_or_init(|| Regex::new(r#"(?is)<meta\s+([^>]+?)/?>"#).expect("valid regex"));
    let canonical_re = CANONICAL_RE
        .get_or_init(|| Regex::new(r#"(?is)<link[^>]+rel\s*=\s*["']canonical["'][^>]+href\s*=\s*["']([^"']+)["']"#).expect("valid regex"));

    let title = title_re
        .captures(html)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().trim().to_string())
        .unwrap_or_default();

    let canonical_url = canonical_re
        .captures(html)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
        .unwrap_or_default();

    let mut description = String::new();
    let mut og_type = String::new();
    let mut og_image = String::new();
    let mut og_site_name = String::new();
    let mut author = String::new();

    // Parse meta tags for name/property + content pairs
    for cap in meta_re.captures_iter(html) {
        let attrs = cap.get(1).map(|m| m.as_str()).unwrap_or("");
        let content = extract_attr(attrs, "content").unwrap_or_default();
        if content.is_empty() {
            continue;
        }

        if let Some(name) = extract_attr(attrs, "name") {
            match name.to_lowercase().as_str() {
                "description" if description.is_empty() => description = content.clone(),
                "author" if author.is_empty() => author = content.clone(),
                _ => {}
            }
        }
        if let Some(property) = extract_attr(attrs, "property") {
            match property.to_lowercase().as_str() {
                "og:description" => description = content.clone(),
                "og:type" => og_type = content.clone(),
                "og:image" => og_image = content.clone(),
                "og:site_name" => og_site_name = content.clone(),
                _ => {}
            }
        }
    }

    PageMetadata {
        title,
        description,
        og_type,
        og_image,
        og_site_name,
        canonical_url,
        author,
    }
}

/// Extract an HTML attribute value from an attribute string.
fn extract_attr(attrs: &str, name: &str) -> Option<String> {
    use regex::Regex;
    let pattern = format!(r#"(?i){}\s*=\s*["']([^"']*)["']"#, regex::escape(name));
    Regex::new(&pattern)
        .ok()
        .and_then(|re| re.captures(attrs))
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

/// Strip hidden HTML content to prevent prompt injection via hidden elements.
///
/// Removes:
/// - `<script>` blocks (JS execution and inline JS injection)
/// - `<style>` blocks (CSS-based content hiding)
/// - HTML comments (hidden instruction injection via `<!-- -->`)
/// - Elements with `display:none` or `visibility:hidden` inline styles
/// - Elements with the `hidden` HTML attribute
fn strip_hidden_html_content(html: &str) -> String {
    use regex::Regex;
    use std::sync::OnceLock;

    static SCRIPT_RE: OnceLock<Regex> = OnceLock::new();
    static STYLE_BLOCK_RE: OnceLock<Regex> = OnceLock::new();
    static COMMENT_RE: OnceLock<Regex> = OnceLock::new();
    static HIDDEN_STYLE_RE: OnceLock<Regex> = OnceLock::new();
    static HIDDEN_ATTR_RE: OnceLock<Regex> = OnceLock::new();

    let script_re = SCRIPT_RE
        .get_or_init(|| Regex::new(r"(?is)<script[^>]*>.*?</script\s*>").expect("valid regex"));
    let style_re = STYLE_BLOCK_RE
        .get_or_init(|| Regex::new(r"(?is)<style[^>]*>.*?</style\s*>").expect("valid regex"));
    let comment_re = COMMENT_RE.get_or_init(|| Regex::new(r"(?s)<!--.*?-->").expect("valid regex"));
    // Elements whose inline style contains display:none or visibility:hidden.
    // Non-greedy match handles simple (non-nested) hidden containers.
    let hidden_style_re = HIDDEN_STYLE_RE.get_or_init(|| {
        Regex::new(
            r#"(?is)<[a-z][a-z0-9]*[^>]+style\s*=\s*["'][^"']*(?:display\s*:\s*none|visibility\s*:\s*hidden)[^"']*["'][^>]*>.*?</[a-z][a-z0-9]*\s*>"#,
        )
        .expect("valid regex")
    });
    // Elements with the bare `hidden` attribute.
    let hidden_attr_re = HIDDEN_ATTR_RE.get_or_init(|| {
        Regex::new(
            r#"(?is)<[a-z][a-z0-9]*(?:\s[^>]*)?\shidden(?:\s[^>]*)?>.*?</[a-z][a-z0-9]*\s*>"#,
        )
        .expect("valid regex")
    });

    let s = script_re.replace_all(html, "");
    let s = style_re.replace_all(&s, "");
    let s = comment_re.replace_all(&s, "");
    let s = hidden_style_re.replace_all(&s, "");
    let s = hidden_attr_re.replace_all(&s, "");
    s.into_owned()
}

// ============================================================================
// web_search - Search the web using DuckDuckGo HTML
// ============================================================================

async fn web_search_tool(args: Value) -> Result<String> {
    let query = args
        .get("query")
        .and_then(|q| q.as_str())
        .ok_or_else(|| tool_err!(validation, "Missing 'query' argument"))?;

    let max_results = args
        .get("max_results")
        .and_then(|m| m.as_u64())
        .unwrap_or(5)
        .min(20) as usize;

    debug!("web_search: query='{}' max_results={}", query, max_results);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent("Mozilla/5.0 (compatible; Zeus/1.0)")
        .build()
        .map_err(|e| tool_err!(network, "Failed to create client: {}", e))?;

    let response = client
        .get("https://html.duckduckgo.com/html/")
        .query(&[("q", query)])
        .send()
        .await
        .map_err(|e| tool_err!(network, "Search request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(tool_err!(network, "Search returned HTTP {}",
            response.status()));
    }

    let html = response
        .text()
        .await
        .map_err(|e| tool_err!(network, "Failed to read search response: {}", e))?;

    // Parse results from DuckDuckGo HTML
    let mut results = Vec::new();

    static RESULT_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    static SNIPPET_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();

    let result_re = RESULT_RE.get_or_init(|| {
        regex::Regex::new(
            r#"<a rel="nofollow" class="result__a" href="([^"]*)"[^>]*>([^<]*(?:<b>[^<]*</b>[^<]*)*)</a>"#
        ).expect("compile-time constant regex")
    });
    let snippet_re = SNIPPET_RE.get_or_init(|| {
        regex::Regex::new(r#"<a class="result__snippet"[^>]*>([^<]*(?:<b>[^<]*</b>[^<]*)*)</a>"#)
            .expect("compile-time constant regex")
    });

    let urls: Vec<(String, String)> = result_re
        .captures_iter(&html)
        .map(|cap| {
            let url = cap[1].to_string();
            let title = cap[2].replace("<b>", "").replace("</b>", "");
            (url, title)
        })
        .collect();

    let snippets: Vec<String> = snippet_re
        .captures_iter(&html)
        .map(|cap| {
            cap[1]
                .replace("<b>", "")
                .replace("</b>", "")
                .trim()
                .to_string()
        })
        .collect();

    for (i, (url, title)) in urls.iter().enumerate().take(max_results) {
        let snippet = snippets.get(i).map(|s| s.as_str()).unwrap_or("");
        results.push(format!(
            "{}. {}
   URL: {}
   {}",
            i + 1,
            title.trim(),
            url,
            snippet
        ));
    }

    if results.is_empty() {
        Ok(format!("No results found for: {}", query))
    } else {
        Ok(format!(
            "Search results for '{}':\n\n{}",
            query,
            results.join("\n\n")
        ))
    }
}

// ============================================================================
// SSRF DNS resolution check
// ============================================================================

/// Resolve a hostname and verify none of the resolved IPs are private/internal.
/// This catches DNS rebinding and hostnames that resolve to internal IPs.
async fn validate_resolved_ips(host: &str) -> Result<()> {
    use std::net::IpAddr;

    // Skip IP literals (already checked in validate_fetch_url)
    if host.parse::<IpAddr>().is_ok() {
        return Ok(());
    }

    // Resolve hostname
    let addrs: Vec<std::net::SocketAddr> = tokio::net::lookup_host(format!("{}:443", host))
        .await
        .map_err(|e| tool_err!(network, "DNS resolution failed for '{}': {}", host, e))?
        .collect();

    if addrs.is_empty() {
        return Err(tool_err!(network, "DNS resolution returned no addresses for '{}'",
            host));
    }

    for addr in &addrs {
        let ip = addr.ip();
        let is_private = match ip {
            IpAddr::V4(v4) => {
                v4.is_loopback()
                    || v4.is_private()
                    || v4.is_link_local()
                    || v4.is_unspecified()
                    || v4.octets()[0] == 100 && (v4.octets()[1] & 0xC0) == 64
            }
            IpAddr::V6(v6) => {
                let segs = v6.segments();
                v6.is_loopback()
                    || v6.is_unspecified()
                    || (segs[0] == 0x0064 && segs[1] == 0xff9b)
                    || segs[0] == 0x2002
                    || (segs[0] == 0x2001 && segs[1] == 0x0000)
                    || v6
                        .to_ipv4_mapped()
                        .is_some_and(|v4| v4.is_loopback() || v4.is_private() || v4.is_link_local())
                    || (segs[0] & 0xfe00) == 0xfc00
                    || (segs[0] & 0xffc0) == 0xfe80
            }
        };
        if is_private {
            return Err(tool_err!(security, "SSRF blocked: '{}' resolves to private IP {}",
                host, ip));
        }
    }

    Ok(())
}

// ============================================================================
// 7. spawn - handled specially by agent loop (this is a fallback)
// ============================================================================

async fn spawn(args: Value) -> Result<String> {
    let task = args
        .get("task")
        .and_then(|t| t.as_str())
        .ok_or_else(|| tool_err!(validation, "Missing 'task' argument"))?;

    debug!("spawn fallback called for: {}", task);

    // This function is a fallback - actual spawn is handled by Agent.execute_spawn()
    // If we reach here, it means spawn was called outside the agent context
    Ok(format!(
        "Spawn request noted for task: '{}'. Note: Full subagent spawning requires agent context.",
        task
    ))
}

// ============================================================================
// 8. message - send to channels (file, webhook, console)
// ============================================================================

async fn message(args: Value) -> Result<String> {
    let channel_spec = args
        .get("channel")
        .and_then(|c| c.as_str())
        .ok_or_else(|| tool_err!(validation, "Missing 'channel' argument"))?;

    let content = args
        .get("content")
        .and_then(|c| c.as_str())
        .ok_or_else(|| tool_err!(validation, "Missing 'content' argument"))?;

    let target = args.get("target").and_then(|t| t.as_str());

    debug!(
        "message: {} -> {:?} ({} chars)",
        channel_spec,
        target,
        content.len()
    );

    let channel = Channel::parse(channel_spec);
    channel.send(content, target).await
}

// ============================================================================
// deep_research — standalone function, callable from cooking loop
// ============================================================================

async fn deep_research_fallback(args: Value) -> Result<String> {
    // Delegate to the standalone function — no LLM available, return stub message
    // This path is only hit when deep_research is called without an LLM client.
    let _ = args;
    Ok(
        "Deep research requires agent context with LLM access. Use through the agent loop."
            .to_string(),
    )
}

/// Standalone deep_research implementation — callable from the cooking loop.
///
/// Extracted from `Agent::execute_deep_research()` in agent_loop.rs.
/// Requires only an `LlmClient`; no full agent state needed.
/// This mirrors the send_file_to_channel pattern from S79.
pub async fn execute_deep_research(args: &Value, llm: &zeus_llm::LlmClient) -> zeus_core::ToolResult {
    let query = match args.get("query").and_then(|q| q.as_str()) {
        Some(q) => q,
        None => {
            return zeus_core::ToolResult {
                call_id: String::new(),
                success: false,
                output: "Missing 'query' argument".to_string(),
            };
        }
    };

    let mut config = crate::research::ResearchConfig::from_env();
    if let Some(max_sources) = args.get("max_sources").and_then(|m| m.as_u64()) {
        config.max_sources = max_sources as usize;
    }
    if let Some(max_queries) = args.get("max_queries").and_then(|m| m.as_u64()) {
        config.max_queries = max_queries as usize;
    }

    let engine = crate::research::ResearchEngine::new(config);

    match engine.research(query, llm).await {
        Ok(report) => {
            let formatted = crate::research::format_report(&report);
            zeus_core::ToolResult {
                call_id: String::new(),
                success: true,
                output: formatted,
            }
        }
        Err(e) => zeus_core::ToolResult {
            call_id: String::new(),
            success: false,
            output: format!("Deep research failed: {}", e),
        },
    }
}

// ============================================================================
// 9. link_understanding - Analyze and understand URL content
// ============================================================================

async fn link_understanding(args: Value) -> Result<String> {
    let url = args
        .get("url")
        .and_then(|u| u.as_str())
        .ok_or_else(|| tool_err!(validation, "Missing 'url' argument"))?;

    let depth = args
        .get("depth")
        .and_then(|d| d.as_str())
        .unwrap_or("medium");

    let focus = args.get("focus").and_then(|f| f.as_str());

    debug!(
        "link_understanding: {} (depth: {}, focus: {:?})",
        url, depth, focus
    );

    // Fetch the URL content
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent("Zeus/1.0 (Link Understanding Bot)")
        .build()
        .map_err(|e| tool_err!(network, "Failed to create client: {}", e))?;

    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| tool_err!(network, "Failed to fetch URL: {}", e))?;

    let status = response.status();
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("text/html")
        .to_string();

    if !status.is_success() {
        return Err(tool_err!(network, "HTTP {} for {}", status, url));
    }

    let body = response
        .text()
        .await
        .map_err(|e| tool_err!(network, "Failed to read response: {}", e))?;

    // Extract structured content
    let mut result = String::new();
    result.push_str(&format!("# Link Analysis: {}\n\n", url));
    result.push_str(&format!("**Content-Type:** {}\n", content_type));
    result.push_str(&format!("**Status:** {}\n\n", status));

    if content_type.contains("text/html") || content_type.contains("application/xhtml") {
        // Extract title
        if let Some(title) = extract_html_title(&body) {
            result.push_str(&format!("## Title\n{}\n\n", title));
        }

        // Extract main text content (strip HTML tags)
        let text_content = strip_html_tags(&body);
        let text_content = collapse_whitespace(&text_content);

        match depth {
            "shallow" => {
                // Just title and first paragraph
                let preview = text_content.chars().take(500).collect::<String>();
                result.push_str(&format!("## Preview\n{}\n", preview));
            }
            "deep" => {
                // Full content (truncated to reasonable size)
                let full = if text_content.len() > 10000 {
                    let end = zeus_core::floor_char_boundary(&text_content, 10000);
                    format!(
                        "{}...\n\n(truncated from {} chars)",
                        &text_content[..end],
                        text_content.len()
                    )
                } else {
                    text_content.clone()
                };
                result.push_str(&format!("## Full Content\n{}\n", full));

                // Extract links if focus is on links
                if focus == Some("links") {
                    let links = extract_html_links(&body, url);
                    if !links.is_empty() {
                        result.push_str("\n## Links Found\n");
                        for link in links.iter().take(50) {
                            result.push_str(&format!("- {}\n", link));
                        }
                    }
                }
            }
            _ => {
                // Medium depth: summary with key points
                let summary = if text_content.len() > 2000 {
                    let end = zeus_core::floor_char_boundary(&text_content, 2000);
                    format!("{}...", &text_content[..end])
                } else {
                    text_content.clone()
                };
                result.push_str(&format!("## Content Summary\n{}\n", summary));

                // Extract metadata
                if let Some(description) = extract_html_meta(&body, "description") {
                    result.push_str(&format!("\n## Meta Description\n{}\n", description));
                }
            }
        }
    } else if content_type.contains("application/json") {
        // JSON content - pretty print
        if let Ok(json) = serde_json::from_str::<Value>(&body) {
            let pretty = serde_json::to_string_pretty(&json).unwrap_or_else(|_| body.clone());
            let display = if pretty.len() > 5000 {
                format!("{}...\n\n(truncated)", &pretty[..zeus_core::floor_char_boundary(&pretty, 5000)])
            } else {
                pretty
            };
            result.push_str(&format!("## JSON Content\n```json\n{}\n```\n", display));
        } else {
            result.push_str(&format!(
                "## Raw Content\n{}\n",
                &body[..body.len().min(5000)]
            ));
        }
    } else {
        // Other content types - show raw preview
        let preview = body.chars().take(2000).collect::<String>();
        result.push_str(&format!("## Content Preview\n{}\n", preview));
    }

    Ok(result)
}

/// Extract the title from HTML content
fn extract_html_title(html: &str) -> Option<String> {
    let lower = html.to_lowercase();
    if let Some(start) = lower.find("<title>")
        && let Some(end) = lower[start..].find("</title>")
    {
        let title_start = start + 7;
        let title_end = start + end;
        if title_end > title_start && title_end <= html.len() {
            return Some(html[title_start..title_end].trim().to_string());
        }
    }
    None
}

/// Extract meta description or other meta tags
fn extract_html_meta(html: &str, name: &str) -> Option<String> {
    let lower = html.to_lowercase();
    let pattern = format!("name=\"{}\"", name);
    if let Some(pos) = lower.find(&pattern) {
        // Look for content= nearby
        let search_start = pos.saturating_sub(100);
        let search_end = (pos + 200).min(html.len());
        let chunk = &lower[search_start..search_end];
        if let Some(content_pos) = chunk.find("content=\"") {
            let content_start = search_start + content_pos + 9;
            if let Some(content_end_rel) = html[content_start..].find('"') {
                let content_end = content_start + content_end_rel;
                return Some(html[content_start..content_end].to_string());
            }
        }
    }
    None
}

/// Strip HTML tags from content
fn strip_html_tags(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut in_script = false;
    let mut in_style = false;
    let lower = html.to_lowercase();

    let chars: Vec<char> = html.chars().collect();
    let lower_chars: Vec<char> = lower.chars().collect();

    for i in 0..chars.len() {
        let c = chars[i];

        // Check for script/style start
        if i + 7 < lower_chars.len() {
            let slice: String = lower_chars[i..i + 7].iter().collect();
            if slice == "<script" {
                in_script = true;
            }
            if slice == "<style "
                || (i + 6 < lower_chars.len()
                    && lower_chars[i..i + 6].iter().collect::<String>() == "<style")
            {
                in_style = true;
            }
        }

        // Check for script/style end
        if i + 9 <= lower_chars.len() {
            let slice: String = lower_chars[i..i + 9].iter().collect();
            if slice == "</script>" {
                in_script = false;
                continue;
            }
        }
        if i + 8 <= lower_chars.len() {
            let slice: String = lower_chars[i..i + 8].iter().collect();
            if slice == "</style>" {
                in_style = false;
                continue;
            }
        }

        if in_script || in_style {
            continue;
        }

        if c == '<' {
            in_tag = true;
        } else if c == '>' {
            in_tag = false;
            result.push(' '); // Add space after tags
        } else if !in_tag {
            result.push(c);
        }
    }

    result
}

/// Collapse multiple whitespace characters into single spaces
fn collapse_whitespace(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut last_was_space = false;

    for c in text.chars() {
        if c.is_whitespace() {
            if !last_was_space {
                result.push(' ');
                last_was_space = true;
            }
        } else {
            result.push(c);
            last_was_space = false;
        }
    }

    result.trim().to_string()
}

/// Extract links from HTML content
fn extract_html_links(html: &str, base_url: &str) -> Vec<String> {
    let mut links = Vec::new();
    let lower = html.to_lowercase();
    let mut pos = 0;

    while let Some(href_pos) = lower[pos..].find("href=\"") {
        let start = pos + href_pos + 6;
        if let Some(end_rel) = html[start..].find('"') {
            let href = &html[start..start + end_rel];
            // Normalize relative URLs
            let full_url = if href.starts_with("http://") || href.starts_with("https://") {
                href.to_string()
            } else if href.starts_with("//") {
                format!("https:{}", href)
            } else if href.starts_with('/') {
                // Extract base domain
                if let Some(domain_end) = base_url.find("://").map(|p| {
                    base_url[p + 3..]
                        .find('/')
                        .map(|e| p + 3 + e)
                        .unwrap_or(base_url.len())
                }) {
                    format!("{}{}", &base_url[..domain_end], href)
                } else {
                    href.to_string()
                }
            } else {
                href.to_string()
            };
            if !full_url.is_empty() && !links.contains(&full_url) {
                links.push(full_url);
            }
        }
        pos = start;
    }

    links
}

// ============================================================================
// 10. media_understanding - Analyze media files
// ============================================================================

async fn media_understanding(args: Value) -> Result<String> {
    let path = args
        .get("path")
        .and_then(|p| p.as_str())
        .ok_or_else(|| tool_err!(validation, "Missing 'path' argument"))?;

    let media_type = args
        .get("media_type")
        .and_then(|t| t.as_str())
        .unwrap_or("auto");

    let analysis = args
        .get("analysis")
        .and_then(|a| a.as_str())
        .unwrap_or("describe");

    debug!(
        "media_understanding: {} (type: {}, analysis: {})",
        path, media_type, analysis
    );

    // Check file exists
    let path_obj = Path::new(path);
    if !path_obj.exists() {
        return Err(tool_err!(not_found, "File not found: {}", path));
    }

    let metadata = fs::metadata(path)
        .await
        .map_err(|e| tool_err!(tool, "Failed to read file metadata: {}", e))?;

    let file_size = metadata.len();

    // Detect media type from extension if auto
    let detected_type = if media_type == "auto" {
        let ext = path_obj
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        match ext.as_str() {
            "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "tiff" | "svg" | "heic" => "image",
            "mp3" | "wav" | "ogg" | "flac" | "m4a" | "aac" | "wma" => "audio",
            "mp4" | "mov" | "avi" | "mkv" | "webm" | "m4v" | "wmv" => "video",
            "pdf" => "document",
            _ => "unknown",
        }
    } else {
        media_type
    };

    let mut result = String::new();
    result.push_str(&format!("# Media Analysis: {}\n\n", path));
    result.push_str(&format!(
        "**File Size:** {} bytes ({:.2} MB)\n",
        file_size,
        file_size as f64 / 1_000_000.0
    ));
    result.push_str(&format!("**Detected Type:** {}\n", detected_type));
    result.push_str(&format!("**Analysis Mode:** {}\n\n", analysis));

    match detected_type {
        "image" => {
            result.push_str("## Image Analysis\n\n");

            // Get image dimensions using `file` command (cross-platform basic info)
            let file_info = get_file_info(path).await?;
            result.push_str(&format!("**File Info:** {}\n\n", file_info));

            match analysis {
                "ocr" => {
                    // Try to use tesseract for OCR if available
                    result.push_str("### OCR Text Extraction\n\n");
                    match run_ocr(path).await {
                        Ok(text) => {
                            if text.trim().is_empty() {
                                result.push_str("*No text detected in image*\n");
                            } else {
                                result.push_str(&format!("```\n{}\n```\n", text));
                            }
                        }
                        Err(e) => {
                            result.push_str(&format!("*OCR not available: {}*\n\n", e));
                            result.push_str("To enable OCR, install tesseract:\n");
                            result.push_str("- macOS: `brew install tesseract`\n");
                            result.push_str("- Linux: `apt install tesseract-ocr`\n");
                        }
                    }
                }
                "objects" => {
                    result.push_str("### Object Detection\n\n");
                    result.push_str("*Object detection requires an ML model. For full object detection, use an LLM with vision capabilities.*\n\n");
                    result.push_str("**Suggestion:** Use this image with a vision-capable model (GPT-4V, Claude with vision) for detailed object detection.\n");
                }
                _ => {
                    // Default: describe
                    result.push_str("### Description\n\n");
                    result.push_str("*For detailed image description, use this file with a vision-capable LLM.*\n\n");
                    result.push_str(&format!("**Path:** `{}`\n", path));
                    result.push_str("**Recommendation:** Send this image to GPT-4V or Claude with vision for a detailed description.\n");
                }
            }
        }
        "audio" => {
            result.push_str("## Audio Analysis\n\n");

            // Get audio metadata using ffprobe if available
            let probe_info = get_media_probe_info(path).await;
            if let Ok(info) = probe_info {
                result.push_str(&format!("**Media Info:**\n```\n{}\n```\n\n", info));
            }

            match analysis {
                "transcribe" => {
                    result.push_str("### Transcription\n\n");

                    // Check for whisper CLI
                    match run_whisper_transcription(path).await {
                        Ok(transcript) => {
                            result.push_str(&format!("```\n{}\n```\n", transcript));
                        }
                        Err(e) => {
                            result.push_str(&format!(
                                "*Local transcription not available: {}*\n\n",
                                e
                            ));
                            result.push_str("**Options for transcription:**\n");
                            result.push_str(
                                "1. Use the `openai-whisper` skill for cloud transcription\n",
                            );
                            result.push_str(
                                "2. Install whisper locally: `pip install openai-whisper`\n",
                            );
                            result.push_str("3. Use Zeus voice tools if configured\n");
                        }
                    }
                }
                _ => {
                    result.push_str("### Audio File Info\n\n");
                    result.push_str(&format!("This is an audio file at `{}`.\n\n", path));
                    result.push_str("**Available analyses:**\n");
                    result.push_str("- Use `analysis: 'transcribe'` to transcribe speech\n");
                    result.push_str(
                        "- Use the `openai-whisper` skill for cloud-based transcription\n",
                    );
                }
            }
        }
        "video" => {
            result.push_str("## Video Analysis\n\n");

            // Get video metadata using ffprobe
            let probe_info = get_media_probe_info(path).await;
            if let Ok(info) = probe_info {
                result.push_str(&format!("**Media Info:**\n```\n{}\n```\n\n", info));
            }

            result.push_str("### Video Content\n\n");
            result
                .push_str("*Full video analysis requires frame extraction and vision models.*\n\n");
            result.push_str("**Suggestions:**\n");
            result.push_str("1. Extract frames using ffmpeg for image analysis\n");
            result.push_str("2. Extract audio track for transcription\n");
            result.push_str("3. Use a multimodal LLM with video support\n");
        }
        "document" => {
            result.push_str("## Document Analysis\n\n");
            result.push_str("*PDF document detected.*\n\n");
            result.push_str("**Suggestions:**\n");
            result.push_str("1. Use `read_file` if this is a text-based PDF\n");
            result.push_str("2. Use Talos PDF tools (`pdf_extract_text`) for text extraction\n");
            result.push_str("3. For scanned PDFs, extract images and use OCR\n");
        }
        _ => {
            result.push_str("## Unknown Media Type\n\n");
            result.push_str(&format!("Could not determine media type for: {}\n", path));
            result.push_str("**File info:**\n");
            let file_info = get_file_info(path).await?;
            result.push_str(&format!("```\n{}\n```\n", file_info));
        }
    }

    Ok(result)
}

/// Get basic file info using the `file` command
async fn get_file_info(path: &str) -> Result<String> {
    let output = Command::new("file")
        .arg("-b")
        .arg(path)
        .output()
        .await
        .map_err(|e| tool_err!(tool, "Failed to run file command: {}", e))?;

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Get media probe info using ffprobe if available
async fn get_media_probe_info(path: &str) -> Result<String> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "quiet",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
            path,
        ])
        .output()
        .await
        .map_err(|e| tool_err!(tool, "ffprobe not available: {}", e))?;

    if output.status.success() {
        // Parse and format key info
        let json_str = String::from_utf8_lossy(&output.stdout);
        if let Ok(json) = serde_json::from_str::<Value>(&json_str) {
            let mut info = String::new();

            if let Some(format) = json.get("format") {
                if let Some(duration) = format.get("duration").and_then(|d| d.as_str())
                    && let Ok(secs) = duration.parse::<f64>()
                {
                    let mins = (secs / 60.0).floor();
                    let secs_rem = secs % 60.0;
                    info.push_str(&format!("Duration: {}:{:05.2}\n", mins as i32, secs_rem));
                }
                if let Some(bit_rate) = format.get("bit_rate").and_then(|b| b.as_str())
                    && let Ok(rate) = bit_rate.parse::<i64>()
                {
                    info.push_str(&format!("Bit Rate: {} kbps\n", rate / 1000));
                }
                if let Some(name) = format.get("format_long_name").and_then(|n| n.as_str()) {
                    info.push_str(&format!("Format: {}\n", name));
                }
            }

            if let Some(streams) = json.get("streams").and_then(|s| s.as_array()) {
                for stream in streams {
                    let codec_type = stream
                        .get("codec_type")
                        .and_then(|t| t.as_str())
                        .unwrap_or("unknown");
                    let codec_name = stream
                        .get("codec_name")
                        .and_then(|n| n.as_str())
                        .unwrap_or("unknown");

                    if codec_type == "video" {
                        let width = stream.get("width").and_then(|w| w.as_i64()).unwrap_or(0);
                        let height = stream.get("height").and_then(|h| h.as_i64()).unwrap_or(0);
                        info.push_str(&format!("Video: {} ({}x{})\n", codec_name, width, height));
                    } else if codec_type == "audio" {
                        let sample_rate = stream
                            .get("sample_rate")
                            .and_then(|s| s.as_str())
                            .unwrap_or("?");
                        let channels = stream.get("channels").and_then(|c| c.as_i64()).unwrap_or(0);
                        info.push_str(&format!(
                            "Audio: {} ({} Hz, {} ch)\n",
                            codec_name, sample_rate, channels
                        ));
                    }
                }
            }

            return Ok(info);
        }
    }

    Err(tool_err!(tool, "ffprobe failed"))
}

/// Run OCR using tesseract
async fn run_ocr(path: &str) -> Result<String> {
    let output = Command::new("tesseract")
        .args([path, "stdout", "-l", "eng"])
        .output()
        .await
        .map_err(|e| tool_err!(tool, "tesseract not available: {}", e))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(Error::Tool(
            String::from_utf8_lossy(&output.stderr).to_string(),
        ))
    }
}

/// Run whisper transcription if available locally
async fn run_whisper_transcription(path: &str) -> Result<String> {
    // Try openai-whisper CLI first
    let output = Command::new("whisper")
        .args([path, "--language", "en", "--output_format", "txt"])
        .output()
        .await;

    if let Ok(out) = output
        && out.status.success()
    {
        // Whisper outputs to a file, try to read it
        let txt_path = format!("{}.txt", path.trim_end_matches(|c: char| c != '.'));
        if let Ok(content) = fs::read_to_string(&txt_path).await {
            let _ = fs::remove_file(&txt_path).await;
            return Ok(content);
        }
        return Ok(String::from_utf8_lossy(&out.stdout).to_string());
    }

    // Fall back to checking if whisper is installed
    Err(Error::Tool(
        "Whisper CLI not found. Install with: pip install openai-whisper".to_string(),
    ))
}

// ============================================================================
// 11. auto_reply - Configure automatic reply rules for channels
// ============================================================================

    // WARNING: Experimental — stores rules locally but NOT enforced by channel adapters yet
async fn auto_reply(args: Value) -> Result<String> {
    let action = args
        .get("action")
        .and_then(|a| a.as_str())
        .ok_or_else(|| tool_err!(validation, "Missing 'action' argument"))?;

    let rules_file = dirs::home_dir()
        .unwrap_or_default()
        .join(".zeus")
        .join("auto_reply_rules.json");

    // Load existing rules
    let mut rules: Vec<Value> = if rules_file.exists() {
        let content = fs::read_to_string(&rules_file)
            .await
            .unwrap_or_else(|_| "[]".to_string());
        serde_json::from_str(&content).unwrap_or_else(|_| Vec::new())
    } else {
        Vec::new()
    };

    match action {
        "list" => {
            if rules.is_empty() {
                return Ok("No auto-reply rules configured.".to_string());
            }
            let mut result = String::from("# Auto-Reply Rules\n\n");
            for (i, rule) in rules.iter().enumerate() {
                let id = rule.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                let channel = rule.get("channel").and_then(|v| v.as_str()).unwrap_or("*");
                let pattern = rule.get("pattern").and_then(|v| v.as_str()).unwrap_or("*");
                let response = rule.get("response").and_then(|v| v.as_str()).unwrap_or("");
                let enabled = rule
                    .get("enabled")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                result.push_str(&format!(
                    "{}. **{}** [{}]\n   - Channel: {}\n   - Pattern: `{}`\n   - Response: {}\n\n",
                    i + 1,
                    id,
                    if enabled { "enabled" } else { "disabled" },
                    channel,
                    pattern,
                    if response.len() > 50 {
                        format!("{}...", &response[..zeus_core::floor_char_boundary(&response, 50)])
                    } else {
                        response.to_string()
                    }
                ));
            }
            Ok(result)
        }
        "add" => {
            let channel = args.get("channel").and_then(|v| v.as_str()).unwrap_or("*");
            let pattern = args
                .get("pattern")
                .and_then(|v| v.as_str())
                .ok_or_else(|| tool_err!(validation, "Missing 'pattern' argument for add"))?;
            let response = args
                .get("response")
                .and_then(|v| v.as_str())
                .ok_or_else(|| tool_err!(validation, "Missing 'response' argument for add"))?;
            let conditions = args
                .get("conditions")
                .cloned()
                .unwrap_or(serde_json::json!({}));

            // Validate regex pattern
            if regex::Regex::new(pattern).is_err() {
                return Err(tool_err!(validation, "Invalid regex pattern: {}", pattern));
            }

            let rule_id = format!("rule_{}", chrono::Utc::now().timestamp_millis());
            let new_rule = serde_json::json!({
                "id": rule_id,
                "channel": channel,
                "pattern": pattern,
                "response": response,
                "conditions": conditions,
                "enabled": true,
                "created_at": chrono::Utc::now().to_rfc3339()
            });

            rules.push(new_rule);

            // Save rules
            if let Some(parent) = rules_file.parent() {
                fs::create_dir_all(parent).await?;
            }
            fs::write(&rules_file, serde_json::to_string_pretty(&rules)?).await?;

            Ok(format!(
                "Auto-reply rule '{}' added.\n- Channel: {}\n- Pattern: {}\n- Response: {}",
                rule_id, channel, pattern, response
            ))
        }
        "remove" => {
            let rule_id = args
                .get("rule_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| tool_err!(validation, "Missing 'rule_id' argument for remove"))?;

            let original_len = rules.len();
            rules.retain(|r| r.get("id").and_then(|v| v.as_str()) != Some(rule_id));

            if rules.len() == original_len {
                return Err(tool_err!(not_found, "Rule '{}' not found", rule_id));
            }

            fs::write(&rules_file, serde_json::to_string_pretty(&rules)?).await?;
            Ok(format!("Auto-reply rule '{}' removed.", rule_id))
        }
        "enable" | "disable" => {
            let rule_id = args
                .get("rule_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| tool_err!(not_found, "Missing 'rule_id' argument for {}", action))?;

            let mut found = false;
            for rule in rules.iter_mut() {
                if rule.get("id").and_then(|v| v.as_str()) == Some(rule_id)
                    && let Some(obj) = rule.as_object_mut()
                {
                    obj.insert("enabled".to_string(), serde_json::json!(action == "enable"));
                    found = true;
                }
            }

            if !found {
                return Err(tool_err!(not_found, "Rule '{}' not found", rule_id));
            }

            fs::write(&rules_file, serde_json::to_string_pretty(&rules)?).await?;
            Ok(format!("Auto-reply rule '{}' {}d.", rule_id, action))
        }
        _ => Err(tool_err!(tool, "Unknown action: {}. Use: add, remove, list, enable, disable",
            action)),
    }
}

// ============================================================================
// 12. polls - Create and manage polls across channels
// ============================================================================

    // WARNING: Experimental — creates polls locally but NOT posted to platforms yet
async fn polls(args: Value) -> Result<String> {
    let action = args
        .get("action")
        .and_then(|a| a.as_str())
        .ok_or_else(|| tool_err!(validation, "Missing 'action' argument"))?;

    let polls_file = dirs::home_dir()
        .unwrap_or_default()
        .join(".zeus")
        .join("polls.json");

    // Load existing polls
    let mut polls_data: Vec<Value> = if polls_file.exists() {
        let content = fs::read_to_string(&polls_file)
            .await
            .unwrap_or_else(|_| "[]".to_string());
        serde_json::from_str(&content).unwrap_or_else(|_| Vec::new())
    } else {
        Vec::new()
    };

    match action {
        "list" => {
            let active: Vec<&Value> = polls_data
                .iter()
                .filter(|p| p.get("status").and_then(|s| s.as_str()) == Some("active"))
                .collect();

            if active.is_empty() {
                return Ok("No active polls.".to_string());
            }

            let mut result = String::from("# Active Polls\n\n");
            for poll in active {
                let id = poll.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                let channel = poll.get("channel").and_then(|v| v.as_str()).unwrap_or("?");
                let question = poll.get("question").and_then(|v| v.as_str()).unwrap_or("?");
                let votes = poll.get("votes").and_then(|v| v.as_object());

                result.push_str(&format!("## {} ({})\n**{}**\n", id, channel, question));

                if let Some(options) = poll.get("options").and_then(|o| o.as_array()) {
                    for (i, opt) in options.iter().enumerate() {
                        let opt_str = opt.as_str().unwrap_or("?");
                        let count = votes
                            .and_then(|v| v.get(&i.to_string()))
                            .and_then(|c| c.as_array())
                            .map(|a| a.len())
                            .unwrap_or(0);
                        result.push_str(&format!("  {}. {} ({} votes)\n", i + 1, opt_str, count));
                    }
                }
                result.push('\n');
            }
            Ok(result)
        }
        "create" => {
            let channel = args
                .get("channel")
                .and_then(|v| v.as_str())
                .ok_or_else(|| tool_err!(validation, "Missing 'channel' argument"))?;
            let question = args
                .get("question")
                .and_then(|v| v.as_str())
                .ok_or_else(|| tool_err!(validation, "Missing 'question' argument"))?;
            let options = args
                .get("options")
                .and_then(|v| v.as_array())
                .ok_or_else(|| tool_err!(validation, "Missing 'options' argument"))?;

            if options.len() < 2 {
                return Err(tool_err!(validation, "Poll must have at least 2 options"));
            }
            if options.len() > 10 {
                return Err(Error::Tool(
                    "Poll cannot have more than 10 options".to_string(),
                ));
            }

            let target = args.get("target").and_then(|v| v.as_str());
            let multi_select = args
                .get("multi_select")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let duration = args.get("duration_minutes").and_then(|v| v.as_i64());

            let poll_id = format!("poll_{}", chrono::Utc::now().timestamp_millis());
            let expires_at =
                duration.map(|d| (chrono::Utc::now() + chrono::Duration::minutes(d)).to_rfc3339());

            let new_poll = serde_json::json!({
                "id": poll_id,
                "channel": channel,
                "target": target,
                "question": question,
                "options": options,
                "multi_select": multi_select,
                "status": "active",
                "votes": {},
                "created_at": chrono::Utc::now().to_rfc3339(),
                "expires_at": expires_at
            });

            polls_data.push(new_poll);

            if let Some(parent) = polls_file.parent() {
                fs::create_dir_all(parent).await?;
            }
            fs::write(&polls_file, serde_json::to_string_pretty(&polls_data)?).await?;

            let mut result = format!(
                "Poll '{}' created on {}.\n\n**{}**\n",
                poll_id, channel, question
            );
            for (i, opt) in options.iter().enumerate() {
                result.push_str(&format!("  {}. {}\n", i + 1, opt.as_str().unwrap_or("?")));
            }
            if let Some(exp) = expires_at {
                result.push_str(&format!("\nExpires: {}", exp));
            }

            Ok(result)
        }
        "close" => {
            let poll_id = args
                .get("poll_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| tool_err!(validation, "Missing 'poll_id' argument"))?;

            let mut found = false;
            for poll in polls_data.iter_mut() {
                if poll.get("id").and_then(|v| v.as_str()) == Some(poll_id)
                    && let Some(obj) = poll.as_object_mut()
                {
                    obj.insert("status".to_string(), serde_json::json!("closed"));
                    obj.insert(
                        "closed_at".to_string(),
                        serde_json::json!(chrono::Utc::now().to_rfc3339()),
                    );
                    found = true;
                }
            }

            if !found {
                return Err(tool_err!(not_found, "Poll '{}' not found", poll_id));
            }

            fs::write(&polls_file, serde_json::to_string_pretty(&polls_data)?).await?;
            Ok(format!("Poll '{}' closed.", poll_id))
        }
        "results" => {
            let poll_id = args
                .get("poll_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| tool_err!(validation, "Missing 'poll_id' argument"))?;

            let poll = polls_data
                .iter()
                .find(|p| p.get("id").and_then(|v| v.as_str()) == Some(poll_id))
                .ok_or_else(|| tool_err!(not_found, "Poll '{}' not found", poll_id))?;

            let question = poll.get("question").and_then(|v| v.as_str()).unwrap_or("?");
            let status = poll.get("status").and_then(|v| v.as_str()).unwrap_or("?");
            let votes = poll.get("votes").and_then(|v| v.as_object());

            let mut result = format!(
                "# Poll Results: {}\n\n**{}**\nStatus: {}\n\n",
                poll_id, question, status
            );

            if let Some(options) = poll.get("options").and_then(|o| o.as_array()) {
                let mut total_votes = 0;
                let mut vote_counts: Vec<usize> = Vec::new();

                for i in 0..options.len() {
                    let count = votes
                        .and_then(|v| v.get(&i.to_string()))
                        .and_then(|c| c.as_array())
                        .map(|a| a.len())
                        .unwrap_or(0);
                    vote_counts.push(count);
                    total_votes += count;
                }

                for (i, opt) in options.iter().enumerate() {
                    let opt_str = opt.as_str().unwrap_or("?");
                    let count = vote_counts[i];
                    let pct = if total_votes > 0 {
                        (count as f64 / total_votes as f64) * 100.0
                    } else {
                        0.0
                    };
                    let bar_len = (pct / 5.0) as usize;
                    let bar: String = "█".repeat(bar_len);
                    result.push_str(&format!(
                        "{}. {} - {} votes ({:.1}%)\n   {}\n",
                        i + 1,
                        opt_str,
                        count,
                        pct,
                        bar
                    ));
                }
                result.push_str(&format!("\nTotal votes: {}", total_votes));
            }

            Ok(result)
        }
        _ => Err(tool_err!(tool, "Unknown action: {}. Use: create, close, results, list",
            action)),
    }
}

// ============================================================================
// 13. gmail_pubsub - Setup Gmail push notifications via Pub/Sub
// ============================================================================

    // WARNING: Experimental — returns setup instructions, does NOT execute Gmail API calls
async fn gmail_pubsub(args: Value) -> Result<String> {
    let action = args
        .get("action")
        .and_then(|a| a.as_str())
        .ok_or_else(|| tool_err!(validation, "Missing 'action' argument"))?;

    // Check for required environment variables
    let google_project = std::env::var("GOOGLE_CLOUD_PROJECT").ok();

    match action {
        "setup" => {
            let topic = args
                .get("topic")
                .and_then(|v| v.as_str())
                .ok_or_else(|| tool_err!(validation, "Missing 'topic' argument"))?;

            let project = google_project.ok_or_else(|| {
                tool_err!(validation, "GOOGLE_CLOUD_PROJECT environment variable not set")
            })?;

            // Instructions for setup
            let mut result = String::from("# Gmail Pub/Sub Setup\n\n");
            result.push_str("## Prerequisites\n");
            result.push_str("1. Enable Gmail API in Google Cloud Console\n");
            result.push_str("2. Enable Cloud Pub/Sub API\n");
            result.push_str("3. Set GOOGLE_APPLICATION_CREDENTIALS environment variable\n\n");

            result.push_str("## Setup Steps\n\n");
            result.push_str(&format!("### 1. Create Pub/Sub Topic\n```bash\ngcloud pubsub topics create {} --project={}\n```\n\n", topic, project));
            result.push_str(&format!("### 2. Grant Gmail Permission to Publish\n```bash\ngcloud pubsub topics add-iam-policy-binding {} \\\n  --member='serviceAccount:gmail-api-push@system.gserviceaccount.com' \\\n  --role='roles/pubsub.publisher' \\\n  --project={}\n```\n\n", topic, project));
            result.push_str(&format!("### 3. Create Subscription (if using pull)\n```bash\ngcloud pubsub subscriptions create {}-sub \\\n  --topic={} \\\n  --project={}\n```\n\n", topic, topic, project));

            result.push_str("## Next Steps\n");
            result.push_str("After setup, use `gmail_pubsub` with action='watch' to start receiving notifications.\n");

            Ok(result)
        }
        "watch" => {
            let topic = args
                .get("topic")
                .and_then(|v| v.as_str())
                .ok_or_else(|| tool_err!(validation, "Missing 'topic' argument"))?;

            let project = google_project.ok_or_else(|| {
                tool_err!(validation, "GOOGLE_CLOUD_PROJECT environment variable not set")
            })?;

            let labels = args
                .get("labels")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str())
                        .collect::<Vec<_>>()
                        .join(",")
                })
                .unwrap_or_else(|| "INBOX".to_string());

            // Build the watch request
            let topic_name = format!("projects/{}/topics/{}", project, topic);

            let watch_request = serde_json::json!({
                "topicName": topic_name,
                "labelIds": labels.split(',').collect::<Vec<_>>()
            });

            let mut result = String::from("# Gmail Watch Request\n\n");
            result.push_str("To start watching Gmail via API, make this request:\n\n");
            result.push_str("```bash\ncurl -X POST \\\n");
            result.push_str("  'https://gmail.googleapis.com/gmail/v1/users/me/watch' \\\n");
            result.push_str("  -H 'Authorization: Bearer $(gcloud auth print-access-token)' \\\n");
            result.push_str("  -H 'Content-Type: application/json' \\\n");
            result.push_str(&format!(
                "  -d '{}'\n```\n\n",
                serde_json::to_string(&watch_request)?
            ));

            result.push_str("The watch expires after 7 days. Set up a cron job to renew it.\n");
            result.push_str(&format!("\nWatching labels: {}\n", labels));
            result.push_str(&format!("Topic: {}\n", topic_name));

            Ok(result)
        }
        "stop" => {
            let mut result = String::from("# Stop Gmail Watch\n\n");
            result.push_str("To stop watching Gmail, make this request:\n\n");
            result.push_str("```bash\ncurl -X POST \\\n");
            result.push_str("  'https://gmail.googleapis.com/gmail/v1/users/me/stop' \\\n");
            result
                .push_str("  -H 'Authorization: Bearer $(gcloud auth print-access-token)'\n```\n");

            Ok(result)
        }
        "status" => {
            let config_file = dirs::home_dir()
                .unwrap_or_default()
                .join(".zeus")
                .join("gmail_pubsub.json");

            if config_file.exists() {
                let content = fs::read_to_string(&config_file).await?;
                let config: Value = serde_json::from_str(&content)?;

                let mut result = String::from("# Gmail Pub/Sub Status\n\n");
                result.push_str(&format!(
                    "**Topic:** {}\n",
                    config
                        .get("topic")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Not set")
                ));
                result.push_str(&format!(
                    "**Subscription:** {}\n",
                    config
                        .get("subscription")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Not set")
                ));
                result.push_str(&format!(
                    "**Watch Started:** {}\n",
                    config
                        .get("watch_started")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Never")
                ));
                result.push_str(&format!(
                    "**Expires:** {}\n",
                    config
                        .get("expires_at")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Unknown")
                ));
                result.push_str(&format!(
                    "**History ID:** {}\n",
                    config
                        .get("history_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Not tracked")
                ));

                Ok(result)
            } else {
                Ok("Gmail Pub/Sub not configured. Use action='setup' to begin.".to_string())
            }
        }
        "process" => {
            let history_id = args
                .get("history_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| tool_err!(validation, "Missing 'history_id' argument"))?;

            let mut result = String::from("# Process Gmail History\n\n");
            result.push_str(&format!(
                "Processing changes since history ID: {}\n\n",
                history_id
            ));
            result.push_str("To fetch changes, make this request:\n\n");
            result.push_str("```bash\ncurl -X GET \\\n");
            result.push_str(&format!(
                "  'https://gmail.googleapis.com/gmail/v1/users/me/history?startHistoryId={}' \\\n",
                history_id
            ));
            result.push_str(
                "  -H 'Authorization: Bearer $(gcloud auth print-access-token)'\n```\n\n",
            );

            result.push_str("The response will contain:\n");
            result.push_str("- messagesAdded: New messages\n");
            result.push_str("- messagesDeleted: Deleted messages\n");
            result.push_str("- labelsAdded: Label changes\n");
            result.push_str("- labelsRemoved: Label removals\n");

            Ok(result)
        }
        _ => Err(tool_err!(tool, "Unknown action: {}. Use: setup, watch, stop, status, process",
            action)),
    }
}

// ============================================================================
// 14. apply_patch
// ============================================================================

async fn apply_patch(args: Value) -> Result<String> {
    let patch_text = args
        .get("patch")
        .and_then(|p| p.as_str())
        .ok_or_else(|| tool_err!(validation, "Missing 'patch' argument"))?;

    let strip = args.get("strip").and_then(|s| s.as_u64()).unwrap_or(0) as usize;

    let dry_run = args
        .get("dry_run")
        .and_then(|d| d.as_bool())
        .unwrap_or(false);

    debug!(dry_run = dry_run, strip = strip, "apply_patch");

    // Parse the unified diff
    let mut files_patched = 0;
    let mut hunks_applied = 0;
    let mut errors = Vec::new();
    let mut current_file: Option<String> = None;
    let mut hunks: Vec<DiffHunk> = Vec::new();

    for line in patch_text.lines() {
        if let Some(rest) = line.strip_prefix("+++ ") {
            // Save previous file's hunks
            if let Some(ref file_path) = current_file {
                match apply_hunks_to_file(file_path, &hunks, dry_run).await {
                    Ok(n) => {
                        hunks_applied += n;
                        files_patched += 1;
                    }
                    Err(e) => errors.push(format!("{}: {}", file_path, e)),
                }
            }
            hunks.clear();

            let path = rest.trim();
            // Strip leading path components
            let stripped = strip_path(path, strip);
            current_file = Some(stripped);
        } else if line.starts_with("@@ ") {
            // Parse hunk header: @@ -start,len +start,len @@
            if let Some(hunk) = parse_hunk_header(line) {
                hunks.push(hunk);
            }
        } else if let Some(ref mut hunk) = hunks.last_mut() {
            if let Some(rest) = line.strip_prefix('+') {
                hunk.additions.push(rest.to_string());
            } else if let Some(rest) = line.strip_prefix('-') {
                hunk.removals.push(rest.to_string());
            } else if let Some(rest) = line.strip_prefix(' ') {
                hunk.context.push(rest.to_string());
                hunk.additions.push(rest.to_string());
            }
        }
    }

    // Apply last file
    if let Some(ref file_path) = current_file {
        match apply_hunks_to_file(file_path, &hunks, dry_run).await {
            Ok(n) => {
                hunks_applied += n;
                files_patched += 1;
            }
            Err(e) => errors.push(format!("{}: {}", file_path, e)),
        }
    }

    let mut result = format!(
        "{} file(s) patched, {} hunk(s) applied",
        files_patched, hunks_applied
    );
    if dry_run {
        result = format!("[dry run] {}", result);
    }
    if !errors.is_empty() {
        result.push_str(&format!("\nErrors:\n{}", errors.join("\n")));
    }

    Ok(result)
}

struct DiffHunk {
    start_line: usize,
    removals: Vec<String>,
    additions: Vec<String>,
    context: Vec<String>,
}

fn parse_hunk_header(line: &str) -> Option<DiffHunk> {
    // @@ -10,5 +10,7 @@
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() >= 3 {
        let old_range = parts[1].trim_start_matches('-');
        let start: usize = old_range
            .split(',')
            .next()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1);
        Some(DiffHunk {
            start_line: start,
            removals: Vec::new(),
            additions: Vec::new(),
            context: Vec::new(),
        })
    } else {
        None
    }
}

fn strip_path(path: &str, strip: usize) -> String {
    if strip == 0 {
        return path.to_string();
    }
    let components: Vec<&str> = path.split('/').collect();
    if strip >= components.len() {
        components.last().unwrap_or(&path).to_string()
    } else {
        components[strip..].join("/")
    }
}

async fn apply_hunks_to_file(path: &str, hunks: &[DiffHunk], dry_run: bool) -> Result<usize> {
    if hunks.is_empty() {
        return Ok(0);
    }

    // For new files (path is /dev/null in ---), create the file
    let file_exists = tokio::fs::metadata(path).await.is_ok();

    if !file_exists {
        // New file — just write all additions
        if !dry_run {
            let content: String = hunks
                .iter()
                .flat_map(|h| h.additions.iter())
                .cloned()
                .collect::<Vec<_>>()
                .join("\n");
            if let Some(parent) = std::path::Path::new(path).parent() {
                let _ = tokio::fs::create_dir_all(parent).await;
            }
            tokio::fs::write(path, content + "\n")
                .await
                .map_err(|e| tool_err!(tool, "Failed to create {}: {}", path, e))?;
        }
        return Ok(hunks.len());
    }

    let content = tokio::fs::read_to_string(path)
        .await
        .map_err(|e| tool_err!(tool, "Failed to read {}: {}", path, e))?;

    let mut lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
    let mut offset: i64 = 0;
    let mut applied = 0;

    for hunk in hunks {
        let target_line = ((hunk.start_line as i64) + offset - 1).max(0) as usize;

        // Remove old lines
        let remove_count = hunk.removals.len();
        if target_line + remove_count <= lines.len() {
            lines.drain(target_line..target_line + remove_count);
        }

        // Insert new lines
        for (i, addition) in hunk.additions.iter().enumerate() {
            let pos = (target_line + i).min(lines.len());
            lines.insert(pos, addition.clone());
        }

        offset += (hunk.additions.len() as i64) - (hunk.removals.len() as i64);
        applied += 1;
    }

    if !dry_run {
        let result = lines.join("\n") + "\n";
        tokio::fs::write(path, result)
            .await
            .map_err(|e| tool_err!(tool, "Failed to write {}: {}", path, e))?;
    }

    Ok(applied)
}

// ============================================================================
// Loop tool — self-scheduling for autonomous continuation
// ============================================================================

async fn loop_tool(args: Value) -> Result<String> {
    let message = args["message"]
        .as_str()
        .ok_or_else(|| zeus_core::Error::Internal("loop tool requires 'message' parameter".to_string()))?
        .to_string();

    let delay_seconds = args["delay_seconds"]
        .as_u64()
        .unwrap_or(5)
        .clamp(1, 3600);

    let condition = args["condition"].as_str().map(|s| s.to_string());

    // S67-F1: Write to workspace/goals/ directory as a .md file.
    // The gateway's autonomous loop hot-loads goal files every 60s
    // and processes them via the cooking loop.
    let goals_dir = dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join(".zeus/workspace/goals");
    let _ = std::fs::create_dir_all(&goals_dir);

    let description = if let Some(ref cond) = condition {
        format!("[condition: {}] {}", cond, message)
    } else {
        message.clone()
    };

    let now_ts = chrono::Utc::now().timestamp();
    let not_before = now_ts + delay_seconds as i64;

    // S67-F2: Embed `not_before` as YAML front-matter so the gateway
    // hot-loader skips this goal until the requested delay has elapsed.
    // Format:
    //   ---
    //   not_before: <unix_ts>
    //   ---
    //   <description body>
    let file_contents = format!(
        "---\nnot_before: {}\n---\n{}\n",
        not_before, description
    );

    let goal_file = goals_dir.join(format!("loop-{}.md", now_ts));
    std::fs::write(&goal_file, &file_contents)
        .map_err(|e| zeus_core::Error::Internal(format!("Failed to write goal file: {}", e)))?;

    Ok(format!(
        "Scheduled self-message in {}s (not_before={}): \"{}\"{}",
        delay_seconds,
        not_before,
        message,
        condition
            .map(|c| format!(" (condition: {})", c))
            .unwrap_or_default()
    ))
}

/// Parse optional YAML front-matter from a goal file's contents.
///
/// Returns `(not_before_unix_ts, body)`. If no front-matter is present,
/// `not_before` is `None` and `body` is the original input.
///
/// Recognized format (only `not_before` is parsed today):
/// ```text
/// ---
/// not_before: 1730297000
/// ---
/// <body>
/// ```
pub fn parse_goal_front_matter(contents: &str) -> (Option<i64>, &str) {
    let trimmed = contents.trim_start_matches('\u{feff}');
    let Some(rest) = trimmed.strip_prefix("---\n").or_else(|| trimmed.strip_prefix("---\r\n")) else {
        return (None, contents);
    };
    // Find the closing `---` line.
    let Some(end_idx) = rest.find("\n---") else {
        return (None, contents);
    };
    let header = &rest[..end_idx];
    // Skip past the closing fence to the start of the body.
    let after_fence = &rest[end_idx + 4..]; // 4 = len("\n---")
    let body = after_fence.trim_start_matches(['\r', '\n']);

    let mut not_before: Option<i64> = None;
    for line in header.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("not_before:") {
            if let Ok(ts) = rest.trim().parse::<i64>() {
                not_before = Some(ts);
            }
        }
    }
    (not_before, body)
}

#[cfg(test)]
mod loop_tool_tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn loop_tool_writes_not_before_front_matter() {
        // Use a temp HOME so we don't pollute the real workspace.
        let tmp = tempfile::tempdir().expect("tempdir");
        // SAFETY: env is process-global. This test mutates HOME, but no other
        // test in this module reads HOME concurrently. Acceptable for an isolated
        // smoke test of the goal-file writer.
        unsafe {
            std::env::set_var("HOME", tmp.path());
        }

        let before = chrono::Utc::now().timestamp();
        let result = loop_tool(json!({
            "message": "ping self",
            "delay_seconds": 180_u64,
        }))
        .await
        .expect("loop_tool ok");

        assert!(result.contains("not_before="), "result should mention not_before: {}", result);

        let goals_dir = tmp.path().join(".zeus/workspace/goals");
        let entries: Vec<_> = std::fs::read_dir(&goals_dir)
            .expect("goals dir exists")
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(entries.len(), 1, "exactly one goal file should be written");

        let contents = std::fs::read_to_string(entries[0].path()).unwrap();
        let (not_before, body) = parse_goal_front_matter(&contents);
        let nb = not_before.expect("not_before parsed");

        // delay_seconds=180 → not_before should be ~180s in the future.
        // Allow 5s slack for test scheduling jitter.
        assert!(
            nb >= before + 175 && nb <= before + 185,
            "expected not_before ≈ now+180, got delta {}",
            nb - before
        );
        assert!(body.contains("ping self"), "body should preserve message");
    }

    #[test]
    fn parse_goal_front_matter_handles_no_header() {
        let (nb, body) = parse_goal_front_matter("just a plain goal\n");
        assert_eq!(nb, None);
        assert_eq!(body, "just a plain goal\n");
    }

    #[test]
    fn parse_goal_front_matter_extracts_not_before() {
        let input = "---\nnot_before: 1730297000\n---\nhello world\n";
        let (nb, body) = parse_goal_front_matter(input);
        assert_eq!(nb, Some(1730297000));
        assert_eq!(body, "hello world\n");
    }
}

// ============================================================================
// Tests
// ============================================================================

// ============================================================================
// send_file_to_channel — standalone, usable from agent loop AND cooking loop
// ============================================================================

/// Send a file to a channel via the ChannelManager.
///
/// Extracted from `Agent::execute_send_file` so it can be called from any
/// context that holds a `ChannelManager` reference (agent loop, cooking loop,
/// tool executor, etc.).
pub async fn send_file_to_channel(
    args: &Value,
    channels: &zeus_channels::ChannelManager,
) -> zeus_core::ToolResult {
    send_file_to_channel_with_fallback(args, channels, None).await
}

/// Send a file to a channel, with an optional fallback source channel used when
/// `target` is not provided in `args`. This allows agents to omit `target` when
/// sending files back to the channel they received a message from.
pub async fn send_file_to_channel_with_fallback(
    args: &Value,
    channels: &zeus_channels::ChannelManager,
    fallback_source: Option<&zeus_channels::ChannelSource>,
) -> zeus_core::ToolResult {
    let file_path = match args.get("path").and_then(|p| p.as_str()) {
        Some(p) => p,
        None => {
            return zeus_core::ToolResult {
                call_id: String::new(),
                success: false,
                output: "Missing 'path' argument".to_string(),
            };
        }
    };

    let channel_spec = match args.get("channel").and_then(|c| c.as_str()) {
        Some(c) => c,
        None => {
            // Fall back to source channel type if available
            match fallback_source {
                Some(src) => src.channel_type.as_str(),
                None => {
                    return zeus_core::ToolResult {
                        call_id: String::new(),
                        success: false,
                        output: "Missing 'channel' argument".to_string(),
                    };
                }
            }
        }
    };

    // Resolve target: explicit arg takes priority, then fallback source channel ID
    let target_owned: String;
    let target = match args.get("target").and_then(|t| t.as_str()) {
        Some(t) => t,
        None => {
            match fallback_source.and_then(|s| s.chat_id.as_deref()) {
                Some(id) => {
                    target_owned = id.to_string();
                    target_owned.as_str()
                }
                None => {
                    return zeus_core::ToolResult {
                        call_id: String::new(),
                        success: false,
                        output: "Missing 'target' argument and no source channel available as fallback".to_string(),
                    };
                }
            }
        }
    };

    let caption = args.get("caption").and_then(|c| c.as_str()).unwrap_or("");

    // Read the file
    let data = match tokio::fs::read(file_path).await {
        Ok(d) => d,
        Err(e) => {
            return zeus_core::ToolResult {
                call_id: String::new(),
                success: false,
                output: format!("Failed to read file '{}': {}", file_path, e),
            };
        }
    };

    let filename = std::path::Path::new(file_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file");

    let source = zeus_channels::ChannelSource {
        channel_type: channel_spec.to_string(),
        user_id: String::new(),
        chat_id: Some(target.to_string()),
        account_id: None,
        thread_id: None,
        reply_to_message_id: None,
        sender_type: zeus_core::SenderType::System,
    };

    match channels.send_file(&source, filename, &data, Some(caption)).await {
        Ok(_) => zeus_core::ToolResult {
            call_id: String::new(),
            success: true,
            output: format!("File '{}' sent to {}/{}", filename, channel_spec, target),
        },
        Err(e) => zeus_core::ToolResult {
            call_id: String::new(),
            success: false,
            output: format!("Failed to send file: {}", e),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_read_file() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.txt");
        std::fs::write(&path, "Hello, World!").unwrap();

        let result = read_file(serde_json::json!({
            "path": path.to_str().unwrap()
        }))
        .await
        .unwrap();

        assert_eq!(result, "Hello, World!");
    }

    #[tokio::test]
    async fn test_write_file() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.txt");

        write_file(serde_json::json!({
            "path": path.to_str().unwrap(),
            "content": "Test content"
        }))
        .await
        .unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "Test content");
    }

    #[tokio::test]
    async fn test_edit_file() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.txt");
        std::fs::write(&path, "Hello, World!").unwrap();

        edit_file(serde_json::json!({
            "path": path.to_str().unwrap(),
            "search": "World",
            "replace": "Zeus"
        }))
        .await
        .unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "Hello, Zeus!");
    }

    #[tokio::test]
    async fn test_list_dir() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("a.txt"), "").unwrap();
        std::fs::write(tmp.path().join("b.txt"), "").unwrap();
        std::fs::create_dir(tmp.path().join("subdir")).unwrap();

        let result = list_dir(serde_json::json!({
            "path": tmp.path().to_str().unwrap()
        }))
        .await
        .unwrap();

        assert!(result.contains("a.txt"));
        assert!(result.contains("b.txt"));
        assert!(result.contains("subdir/"));
    }

    #[tokio::test]
    async fn test_shell() {
        let result = shell(serde_json::json!({
            "command": "echo hello"
        }))
        .await
        .unwrap();

        assert!(result.trim() == "hello");
    }

    #[tokio::test]
    async fn test_tool_registry() {
        let registry = ToolRegistry::with_defaults();
        let schemas = registry.schemas();

        // core tools + send_file + trigger tools (no Talos in defaults)
        assert_eq!(schemas.len(), 22);
        assert!(schemas.iter().any(|s| s.name == "read_file"));
        assert!(schemas.iter().any(|s| s.name == "shell"));
        assert!(schemas.iter().any(|s| s.name == "auto_reply"));
        assert!(schemas.iter().any(|s| s.name == "polls"));
        assert!(schemas.iter().any(|s| s.name == "gmail_pubsub"));
    }

    #[tokio::test]
    async fn test_tool_registry_with_talos() {
        let talos = TalosRegistry::with_defaults();
        let talos_count = talos.len();
        let registry = ToolRegistry::with_talos(talos);
        let schemas = registry.schemas();

        // core (incl. send_file + trigger tools) + talos tools
        assert_eq!(schemas.len(), 22 + talos_count);
    }

    // ================================================================
    // Tool schema validation tests
    // ================================================================

    #[test]
    fn test_read_file_schema_params() {
        let registry = ToolRegistry::with_defaults();
        let schemas = registry.core_schemas();
        let schema = schemas.iter().find(|s| s.name == "read_file").unwrap();

        assert_eq!(schema.name, "read_file");
        assert!(schema.description.contains("Read"));

        let props = schema.parameters["properties"].as_object().unwrap();
        assert!(props.contains_key("path"));

        let required = schema.parameters["required"].as_array().unwrap();
        let req_strs: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(req_strs.contains(&"path"));
    }

    #[test]
    fn test_write_file_schema_params() {
        let registry = ToolRegistry::with_defaults();
        let schemas = registry.core_schemas();
        let schema = schemas.iter().find(|s| s.name == "write_file").unwrap();

        assert!(schema.description.contains("overwrite"));

        let props = schema.parameters["properties"].as_object().unwrap();
        assert!(props.contains_key("path"));
        assert!(props.contains_key("content"));

        let required = schema.parameters["required"].as_array().unwrap();
        let req_strs: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(req_strs.contains(&"path"));
        assert!(req_strs.contains(&"content"));
    }

    #[test]
    fn test_edit_file_schema_params() {
        let registry = ToolRegistry::with_defaults();
        let schemas = registry.core_schemas();
        let schema = schemas.iter().find(|s| s.name == "edit_file").unwrap();

        let props = schema.parameters["properties"].as_object().unwrap();
        assert!(props.contains_key("path"));
        assert!(props.contains_key("search"));
        assert!(props.contains_key("replace"));
        assert!(props.contains_key("all"));

        let required = schema.parameters["required"].as_array().unwrap();
        let req_strs: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(req_strs.contains(&"path"));
        assert!(req_strs.contains(&"search"));
        assert!(req_strs.contains(&"replace"));
        // "all" should NOT be required
        assert!(!req_strs.contains(&"all"));
    }

    #[test]
    fn test_shell_schema_params() {
        let registry = ToolRegistry::with_defaults();
        let schemas = registry.core_schemas();
        let schema = schemas.iter().find(|s| s.name == "shell").unwrap();

        let props = schema.parameters["properties"].as_object().unwrap();
        assert!(props.contains_key("command"));
        assert!(props.contains_key("cwd"));
        assert!(props.contains_key("timeout"));

        let required = schema.parameters["required"].as_array().unwrap();
        let req_strs: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(req_strs.contains(&"command"));
        assert!(!req_strs.contains(&"cwd"));
        assert!(!req_strs.contains(&"timeout"));
    }

    #[test]
    fn test_spawn_schema_params() {
        let registry = ToolRegistry::with_defaults();
        let schemas = registry.core_schemas();
        let schema = schemas.iter().find(|s| s.name == "spawn").unwrap();

        assert!(schema.description.contains("subagent"));

        let props = schema.parameters["properties"].as_object().unwrap();
        assert!(props.contains_key("task"));
        assert!(props.contains_key("context"));
        assert!(props.contains_key("max_iterations"));
        assert!(props.contains_key("wait"));

        let required = schema.parameters["required"].as_array().unwrap();
        let req_strs: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(req_strs.contains(&"task"));
        assert!(!req_strs.contains(&"context"));
    }

    #[test]
    fn test_message_schema_params() {
        let registry = ToolRegistry::with_defaults();
        let schemas = registry.core_schemas();
        let schema = schemas.iter().find(|s| s.name == "message").unwrap();

        let props = schema.parameters["properties"].as_object().unwrap();
        assert!(props.contains_key("channel"));
        assert!(props.contains_key("content"));
        assert!(props.contains_key("target"));

        let required = schema.parameters["required"].as_array().unwrap();
        let req_strs: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(req_strs.contains(&"channel"));
        assert!(req_strs.contains(&"content"));
        assert!(!req_strs.contains(&"target"));
    }

    #[test]
    fn test_all_core_tool_names_present() {
        let registry = ToolRegistry::with_defaults();
        let schemas = registry.core_schemas();
        let names: Vec<&str> = schemas.iter().map(|s| s.name.as_str()).collect();

        let expected = [
            "read_file",
            "write_file",
            "edit_file",
            "list_dir",
            "shell",
            "web_fetch",
            "spawn",
            "message",
            "link_understanding",
            "media_understanding",
            "auto_reply",
            "polls",
            "gmail_pubsub",
        ];
        for name in &expected {
            assert!(names.contains(name), "Missing tool: {}", name);
        }
    }

    #[test]
    fn test_all_tool_schemas_have_descriptions() {
        let registry = ToolRegistry::with_defaults();
        let schemas = registry.core_schemas();
        for schema in &schemas {
            assert!(
                !schema.description.is_empty(),
                "Tool '{}' has empty description",
                schema.name
            );
        }
    }

    // ================================================================
    // Tool execution tests
    // ================================================================

    #[tokio::test]
    async fn test_read_file_nonexistent() {
        let result = read_file(serde_json::json!({
            "path": "/nonexistent/path/that/does/not/exist.txt"
        }))
        .await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Failed to read"));
    }

    #[tokio::test]
    async fn test_read_file_missing_arg() {
        let result = read_file(serde_json::json!({})).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Missing 'path'"));
    }

    #[tokio::test]
    async fn test_read_file_truncation() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("big.txt");
        // Create a file larger than 100_000 bytes
        let big_content = "x".repeat(150_000);
        std::fs::write(&path, &big_content).unwrap();

        let result = read_file(serde_json::json!({
            "path": path.to_str().unwrap()
        }))
        .await
        .unwrap();

        assert!(result.contains("truncated"));
        assert!(result.contains("150000"));
    }

    #[tokio::test]
    async fn test_write_file_creates_parent_dirs() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("nested").join("dir").join("file.txt");

        let result = write_file(serde_json::json!({
            "path": path.to_str().unwrap(),
            "content": "nested content"
        }))
        .await
        .unwrap();

        assert!(result.contains("Wrote"));
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "nested content");
    }

    #[tokio::test]
    async fn test_write_file_overwrite() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.txt");
        std::fs::write(&path, "original").unwrap();

        write_file(serde_json::json!({
            "path": path.to_str().unwrap(),
            "content": "overwritten"
        }))
        .await
        .unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "overwritten");
    }

    #[tokio::test]
    async fn test_write_file_missing_content() {
        let result = write_file(serde_json::json!({
            "path": "/tmp/test.txt"
        }))
        .await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Missing 'content'"));
    }

    #[tokio::test]
    async fn test_write_file_missing_path() {
        let result = write_file(serde_json::json!({
            "content": "data"
        }))
        .await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Missing 'path'"));
    }

    #[tokio::test]
    async fn test_edit_file_search_not_found() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.txt");
        std::fs::write(&path, "Hello, World!").unwrap();

        let result = edit_file(serde_json::json!({
            "path": path.to_str().unwrap(),
            "search": "NONEXISTENT",
            "replace": "replacement"
        }))
        .await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("not found"));
    }

    #[tokio::test]
    async fn test_edit_file_replace_all() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.txt");
        std::fs::write(&path, "foo bar foo baz foo").unwrap();

        let result = edit_file(serde_json::json!({
            "path": path.to_str().unwrap(),
            "search": "foo",
            "replace": "qux",
            "all": true
        }))
        .await
        .unwrap();

        assert!(result.contains("3 occurrence(s)"));
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "qux bar qux baz qux");
    }

    #[tokio::test]
    async fn test_edit_file_replace_first_only() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.txt");
        std::fs::write(&path, "foo bar foo baz foo").unwrap();

        let result = edit_file(serde_json::json!({
            "path": path.to_str().unwrap(),
            "search": "foo",
            "replace": "qux"
        }))
        .await
        .unwrap();

        assert!(result.contains("1 occurrence(s)"));
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "qux bar foo baz foo");
    }

    #[tokio::test]
    async fn test_edit_file_nonexistent() {
        let result = edit_file(serde_json::json!({
            "path": "/nonexistent/file.txt",
            "search": "a",
            "replace": "b"
        }))
        .await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Failed to read"));
    }

    #[tokio::test]
    async fn test_edit_file_missing_search() {
        let result = edit_file(serde_json::json!({
            "path": "/tmp/test.txt",
            "replace": "b"
        }))
        .await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Missing 'search'"));
    }

    #[tokio::test]
    async fn test_list_dir_nonexistent() {
        let result = list_dir(serde_json::json!({
            "path": "/nonexistent/directory/xyz123"
        }))
        .await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Failed to read"));
    }

    #[tokio::test]
    async fn test_list_dir_recursive() {
        let tmp = TempDir::new().unwrap();
        let sub = tmp.path().join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(tmp.path().join("top.txt"), "").unwrap();
        std::fs::write(sub.join("nested.txt"), "").unwrap();

        let result = list_dir(serde_json::json!({
            "path": tmp.path().to_str().unwrap(),
            "recursive": true
        }))
        .await
        .unwrap();

        assert!(result.contains("top.txt"));
        assert!(result.contains("sub/"));
        assert!(result.contains("nested.txt"));
    }

    #[tokio::test]
    async fn test_list_dir_empty() {
        let tmp = TempDir::new().unwrap();

        let result = list_dir(serde_json::json!({
            "path": tmp.path().to_str().unwrap()
        }))
        .await
        .unwrap();

        assert_eq!(result, "");
    }

    #[tokio::test]
    async fn test_shell_with_cwd() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("marker.txt"), "").unwrap();

        let result = shell(serde_json::json!({
            "command": "ls marker.txt",
            "cwd": tmp.path().to_str().unwrap()
        }))
        .await
        .unwrap();

        assert!(result.contains("marker.txt"));
    }

    #[tokio::test]
    async fn test_shell_failing_command() {
        let result = shell(serde_json::json!({
            "command": "false"
        }))
        .await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("exited with code"));
    }

    #[tokio::test]
    async fn test_shell_missing_command() {
        let result = shell(serde_json::json!({})).await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Missing 'command'"));
    }

    #[tokio::test]
    async fn test_shell_stderr_output() {
        let result = shell(serde_json::json!({
            "command": "echo out && echo err >&2"
        }))
        .await
        .unwrap();

        assert!(result.contains("out"));
        assert!(result.contains("err"));
        assert!(result.contains("stderr"));
    }

    #[tokio::test]
    async fn test_execute_unknown_tool() {
        let registry = ToolRegistry::with_defaults();
        let result = registry
            .execute("nonexistent_tool", serde_json::json!({}))
            .await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Unknown tool"));
    }

    #[tokio::test]
    async fn test_execute_tool_function() {
        let tmp = TempDir::new().expect("Failed to create temp directory");
        let path = tmp.path().join("via_fn.txt");
        std::fs::write(&path, "test content").expect("Failed to write test file");

        let result = execute_tool(
            "read_file",
            serde_json::json!({"path": path.to_str().unwrap()}),
        )
        .await
        .unwrap();

        assert_eq!(result, "test content");
    }

    #[tokio::test]
    async fn test_spawn_fallback() {
        let result = spawn(serde_json::json!({
            "task": "test task"
        }))
        .await
        .unwrap();

        assert!(result.contains("Spawn request noted"));
        assert!(result.contains("test task"));
    }

    #[tokio::test]
    async fn test_spawn_missing_task() {
        let result = spawn(serde_json::json!({})).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Missing 'task'"));
    }

    #[test]
    fn test_tool_registry_default() {
        let registry = ToolRegistry::default();
        let schemas = registry.schemas();
        assert_eq!(schemas.len(), 22);
    }

    // ================================================================
    // HTML helper function tests
    // ================================================================

    #[test]
    fn test_extract_html_title() {
        let html = "<html><head><title>My Page Title</title></head><body></body></html>";
        assert_eq!(extract_html_title(html), Some("My Page Title".to_string()));
    }

    #[test]
    fn test_extract_html_title_missing() {
        let html = "<html><head></head><body>No title here</body></html>";
        assert_eq!(extract_html_title(html), None);
    }

    #[test]
    fn test_extract_html_title_case_insensitive() {
        let html = "<HTML><HEAD><TITLE>Upper Case</TITLE></HEAD></HTML>";
        assert_eq!(extract_html_title(html), Some("Upper Case".to_string()));
    }

    #[test]
    fn test_strip_html_tags_basic() {
        let html = "<p>Hello <b>world</b></p>";
        let result = strip_html_tags(html);
        assert!(result.contains("Hello"));
        assert!(result.contains("world"));
        assert!(!result.contains("<p>"));
        assert!(!result.contains("<b>"));
    }

    #[test]
    fn test_strip_html_tags_removes_script() {
        let html = "<body>Visible<script>var x = 1;</script> text</body>";
        let result = strip_html_tags(html);
        assert!(result.contains("Visible"));
        assert!(result.contains("text"));
        assert!(!result.contains("var x"));
    }

    #[test]
    fn test_collapse_whitespace() {
        let input = "hello    world\n\n\tfoo   bar";
        let result = collapse_whitespace(input);
        assert_eq!(result, "hello world foo bar");
    }

    #[test]
    fn test_collapse_whitespace_trim() {
        let input = "  leading and trailing  ";
        let result = collapse_whitespace(input);
        assert_eq!(result, "leading and trailing");
    }

    #[test]
    fn test_extract_html_links_absolute() {
        let html = r#"<a href="https://example.com/page">Link</a>"#;
        let links = extract_html_links(html, "https://base.com");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0], "https://example.com/page");
    }

    #[test]
    fn test_extract_html_links_relative() {
        let html = r#"<a href="/about">About</a>"#;
        let links = extract_html_links(html, "https://example.com/page");
        assert_eq!(links.len(), 1);
        assert!(links[0].starts_with("https://example.com/about"));
    }

    #[test]
    fn test_extract_html_links_protocol_relative() {
        let html = r#"<a href="//cdn.example.com/res">Resource</a>"#;
        let links = extract_html_links(html, "https://example.com");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0], "https://cdn.example.com/res");
    }

    #[test]
    fn test_extract_html_links_deduplication() {
        let html = r#"<a href="https://example.com">A</a><a href="https://example.com">B</a>"#;
        let links = extract_html_links(html, "https://base.com");
        assert_eq!(links.len(), 1);
    }

    #[test]
    fn test_extract_html_meta_description() {
        let html = r#"<html><head><meta name="description" content="A test page about things"></head></html>"#;
        let desc = extract_html_meta(html, "description");
        assert_eq!(desc, Some("A test page about things".to_string()));
    }

    #[test]
    fn test_extract_html_meta_missing() {
        let html = r#"<html><head><title>No meta</title></head></html>"#;
        let desc = extract_html_meta(html, "description");
        assert_eq!(desc, None);
    }

    // ================================================================
    // New tests
    // ================================================================

    #[tokio::test]
    async fn test_read_file_empty_file() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("empty.txt");
        std::fs::write(&path, "").unwrap();

        let result = read_file(serde_json::json!({
            "path": path.to_str().unwrap()
        }))
        .await
        .unwrap();

        assert_eq!(result, "");
    }

    #[tokio::test]
    async fn test_read_file_binary_detection() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("binary.bin");
        // Write bytes that include null bytes (binary content)
        let binary_data: Vec<u8> = vec![0x00, 0x01, 0x02, 0xFF, 0xFE, 0xFD];
        std::fs::write(&path, &binary_data).unwrap();

        // read_file uses read_to_string which will fail on invalid UTF-8
        let result = read_file(serde_json::json!({
            "path": path.to_str().unwrap()
        }))
        .await;

        // Binary files should produce an error since they can't be read as UTF-8
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Failed to read"));
    }

    #[tokio::test]
    async fn test_read_file_with_utf8() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("unicode.txt");
        let unicode_content =
            "Hello \u{1F600} \u{00E9}\u{00E8}\u{00EA} \u{4F60}\u{597D} \u{0410}\u{0411}\u{0412}";
        std::fs::write(&path, unicode_content).unwrap();

        let result = read_file(serde_json::json!({
            "path": path.to_str().unwrap()
        }))
        .await
        .unwrap();

        assert_eq!(result, unicode_content);
        assert!(result.contains("\u{1F600}"));
        assert!(result.contains("\u{4F60}\u{597D}"));
    }

    #[tokio::test]
    async fn test_write_file_empty_content() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("empty_write.txt");

        let result = write_file(serde_json::json!({
            "path": path.to_str().unwrap(),
            "content": ""
        }))
        .await
        .unwrap();

        assert!(result.contains("Wrote 0 bytes"));
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "");
    }

    #[tokio::test]
    async fn test_write_file_unicode_content() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("unicode_write.txt");
        let unicode_text = "\u{00C0}\u{00C1}\u{00C2} \u{2603} \u{1F4A9} \u{2764}\u{FE0F}";

        write_file(serde_json::json!({
            "path": path.to_str().unwrap(),
            "content": unicode_text
        }))
        .await
        .unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, unicode_text);
    }

    #[tokio::test]
    async fn test_write_file_deeply_nested() {
        let tmp = TempDir::new().unwrap();
        let path = tmp
            .path()
            .join("a")
            .join("b")
            .join("c")
            .join("d")
            .join("e.txt");

        write_file(serde_json::json!({
            "path": path.to_str().unwrap(),
            "content": "deeply nested content"
        }))
        .await
        .unwrap();

        assert!(path.exists());
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "deeply nested content");
    }

    #[tokio::test]
    async fn test_edit_file_multiple_occurrences() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("multi.txt");
        std::fs::write(&path, "apple banana apple cherry apple").unwrap();

        // Default: replace first only
        edit_file(serde_json::json!({
            "path": path.to_str().unwrap(),
            "search": "apple",
            "replace": "orange"
        }))
        .await
        .unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "orange banana apple cherry apple");
        // Only the first occurrence replaced
        assert_eq!(content.matches("apple").count(), 2);
        assert_eq!(content.matches("orange").count(), 1);
    }

    #[tokio::test]
    async fn test_edit_file_empty_replacement() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("delete.txt");
        std::fs::write(&path, "Hello cruel World!").unwrap();

        edit_file(serde_json::json!({
            "path": path.to_str().unwrap(),
            "search": "cruel ",
            "replace": ""
        }))
        .await
        .unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "Hello World!");
    }

    #[tokio::test]
    async fn test_edit_file_newline_handling() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("newlines.txt");
        std::fs::write(&path, "line1\nline2\nline3\n").unwrap();

        edit_file(serde_json::json!({
            "path": path.to_str().unwrap(),
            "search": "line2\nline3",
            "replace": "replaced_lines"
        }))
        .await
        .unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "line1\nreplaced_lines\n");
    }

    #[tokio::test]
    async fn test_list_dir_hidden_files() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join(".hidden"), "").unwrap();
        std::fs::write(tmp.path().join("visible.txt"), "").unwrap();
        std::fs::write(tmp.path().join(".gitignore"), "").unwrap();

        let result = list_dir(serde_json::json!({
            "path": tmp.path().to_str().unwrap()
        }))
        .await
        .unwrap();

        assert!(result.contains(".hidden"));
        assert!(result.contains("visible.txt"));
        assert!(result.contains(".gitignore"));
    }

    #[tokio::test]
    async fn test_list_dir_single_file() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("only.txt"), "sole file").unwrap();

        let result = list_dir(serde_json::json!({
            "path": tmp.path().to_str().unwrap()
        }))
        .await
        .unwrap();

        assert_eq!(result.trim(), "only.txt");
    }

    #[tokio::test]
    async fn test_list_dir_mixed_content() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("file1.txt"), "").unwrap();
        std::fs::write(tmp.path().join("file2.rs"), "").unwrap();
        std::fs::create_dir(tmp.path().join("subdir1")).unwrap();
        std::fs::create_dir(tmp.path().join("subdir2")).unwrap();

        let result = list_dir(serde_json::json!({
            "path": tmp.path().to_str().unwrap()
        }))
        .await
        .unwrap();

        assert!(result.contains("file1.txt"));
        assert!(result.contains("file2.rs"));
        assert!(result.contains("subdir1/"));
        assert!(result.contains("subdir2/"));
    }

    #[tokio::test]
    async fn test_shell_echo_with_spaces() {
        let result = shell(serde_json::json!({
            "command": "echo 'hello world with spaces'"
        }))
        .await
        .unwrap();

        assert_eq!(result.trim(), "hello world with spaces");
    }

    #[tokio::test]
    async fn test_shell_env_variable() {
        let result = shell(serde_json::json!({
            "command": "echo $HOME"
        }))
        .await
        .unwrap();

        // $HOME should expand to something non-empty
        assert!(!result.trim().is_empty());
        assert!(result.trim().starts_with('/'));
    }

    #[tokio::test]
    async fn test_shell_multiline_output() {
        let result = shell(serde_json::json!({
            "command": "printf 'line1\nline2\nline3\n'"
        }))
        .await
        .unwrap();

        let lines: Vec<&str> = result.lines().collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "line1");
        assert_eq!(lines[1], "line2");
        assert_eq!(lines[2], "line3");
    }

    #[tokio::test]
    async fn test_shell_exit_code() {
        let result = shell(serde_json::json!({
            "command": "exit 42"
        }))
        .await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("exited with code 42"));
    }

    #[test]
    fn test_extract_html_title_missing_no_head() {
        // HTML without any title tag at all
        let html = "<html><body><h1>Page Heading</h1><p>Content</p></body></html>";
        assert_eq!(extract_html_title(html), None);
    }

    #[test]
    fn test_extract_html_empty_page() {
        let html = "";
        assert_eq!(extract_html_title(html), None);
    }

    #[test]
    fn test_strip_html_nested_tags() {
        let html = "<div><p><span><strong>Deep <em>nested</em> text</strong></span></p></div>";
        let result = strip_html_tags(html);
        let collapsed = collapse_whitespace(&result);
        assert!(collapsed.contains("Deep"));
        assert!(collapsed.contains("nested"));
        assert!(collapsed.contains("text"));
        assert!(!collapsed.contains("<"));
        assert!(!collapsed.contains(">"));
    }

    #[test]
    fn test_collapse_whitespace_tabs() {
        let input = "hello\t\tworld\t \t foo\t\n\tbar";
        let result = collapse_whitespace(input);
        assert_eq!(result, "hello world foo bar");
    }

    // ========================================================================
    // Security validation tests
    // ========================================================================

    #[test]
    fn test_validate_shell_command_allows_normal() {
        assert!(validate_shell_command("ls -la").is_ok());
        assert!(validate_shell_command("echo hello").is_ok());
        assert!(validate_shell_command("cargo build").is_ok());
        assert!(validate_shell_command("git status").is_ok());
    }

    #[test]
    fn test_validate_shell_command_blocks_sensitive_paths() {
        assert!(validate_shell_command("cat /etc/shadow").is_err());
        assert!(validate_shell_command("cat /etc/master.passwd").is_err());
    }

    #[test]
    fn test_validate_shell_command_blocks_destructive() {
        assert!(validate_shell_command("rm -rf /usr").is_err());
        assert!(validate_shell_command("rm -rf /bin").is_err());
        assert!(validate_shell_command("rm /etc/important").is_err());
        assert!(validate_shell_command("dd if=/dev/zero of=/dev/sda").is_err());
    }

    #[test]
    fn test_validate_shell_command_blocks_null_bytes() {
        assert!(validate_shell_command("ls\0/etc/shadow").is_err());
    }

    #[test]
    fn test_validate_shell_command_blocks_too_long() {
        let long_cmd = "a".repeat(10_001);
        assert!(validate_shell_command(&long_cmd).is_err());
    }

    #[test]
    fn test_validate_shell_command_blocks_env_injection() {
        // Dynamic linker hijack
        assert!(validate_shell_command("LD_PRELOAD=/evil.so ls").is_err());
        assert!(validate_shell_command("DYLD_INSERT_LIBRARIES=/evil.dylib ls").is_err());
        assert!(validate_shell_command("DYLD_LIBRARY_PATH=/tmp ls").is_err());
        // Shell init hijack
        assert!(validate_shell_command("BASH_ENV=/tmp/evil.sh ls").is_err());
        assert!(validate_shell_command("ENV=/tmp/evil sh -c ls").is_err());
        // Language import path injection
        assert!(validate_shell_command("PYTHONPATH=/evil python3 app.py").is_err());
        assert!(validate_shell_command("NODE_OPTIONS=--require=/evil node").is_err());
        // Field separator injection
        assert!(validate_shell_command("IFS=x ls").is_err());
    }

    #[test]
    fn test_validate_shell_command_env_injection_case_insensitive() {
        assert!(validate_shell_command("ld_preload=/evil.so ls").is_err());
        assert!(validate_shell_command("Ld_Preload=/evil.so ls").is_err());
    }

    #[test]
    fn test_validate_shell_command_allows_line_continuation() {
        // Line continuations are standard shell syntax — allowed
        assert!(validate_shell_command("echo safe \\\necho also safe").is_ok());
        assert!(validate_shell_command("curl -s \\\n-H 'Auth: token' \\\nhttp://example.com").is_ok());
    }

    #[test]
    fn test_validate_shell_command_allows_normal_env() {
        // Setting non-dangerous env vars inline is fine
        assert!(validate_shell_command("FOO=bar cargo test").is_ok());
        assert!(validate_shell_command("RUST_LOG=debug ./app").is_ok());
        assert!(validate_shell_command("PATH=/usr/bin:/bin ls").is_ok());
        // *_ENV= suffix patterns must NOT be blocked (false positive fix)
        assert!(validate_shell_command("RAILS_ENV=production bundle exec rake db:migrate").is_ok());
        assert!(validate_shell_command("NODE_ENV=production npm run build").is_ok());
        assert!(validate_shell_command("MIX_ENV=prod mix release").is_ok());
        assert!(validate_shell_command("sudo -u git sh -c 'cd /app && RAILS_ENV=production bundle exec rake assets:precompile'").is_ok());
    }

    #[test]
    fn test_validate_shell_command_env_standalone_still_blocked() {
        // Bare ENV= (POSIX shell init override) must still be caught
        assert!(validate_shell_command("ENV=/tmp/evil sh -c ls").is_err());
        assert!(validate_shell_command("env=/tmp/evil sh").is_err());
        // But only as a standalone var, not as a suffix
        assert!(validate_shell_command("RAILS_ENV=production rails s").is_ok());
        assert!(validate_shell_command("NODE_ENV=test jest").is_ok());
    }

    #[test]
    fn test_validate_tool_path_blocks_null_bytes() {
        assert!(validate_tool_path("/tmp/test\0/etc/shadow").is_err());
    }

    #[test]
    fn test_validate_tool_path_blocks_empty() {
        assert!(validate_tool_path("").is_err());
        assert!(validate_tool_path("   ").is_err());
    }

    #[test]
    fn test_validate_tool_path_blocks_sensitive() {
        assert!(validate_tool_path("/etc/shadow").is_err());
        assert!(validate_tool_path("/etc/master.passwd").is_err());
        assert!(validate_tool_path("/proc/kcore").is_err());
        assert!(validate_tool_path("/dev/sda").is_err());
        assert!(validate_tool_path("/etc/passwd").is_err());
    }

    #[test]
    fn test_validate_tool_path_allows_normal() {
        assert!(validate_tool_path("/tmp/test.txt").is_ok());
        assert!(validate_tool_path("/home/user/file.rs").is_ok());
    }

    #[test]
    fn test_validate_tool_path_blocks_traversal() {
        // Try to escape root via parent dir
        assert!(validate_tool_path("/../../etc/shadow").is_err());
    }

    #[test]
    fn test_validate_fetch_url_allows_normal() {
        assert!(validate_fetch_url("https://example.com").is_ok());
        assert!(validate_fetch_url("http://api.github.com/repos").is_ok());
        assert!(validate_fetch_url("https://docs.rs/some-crate").is_ok());
    }

    #[test]
    fn test_validate_fetch_url_blocks_non_http() {
        assert!(validate_fetch_url("file:///etc/passwd").is_err());
        assert!(validate_fetch_url("ftp://evil.com/data").is_err());
        assert!(validate_fetch_url("gopher://evil.com").is_err());
        assert!(validate_fetch_url("javascript:alert(1)").is_err());
    }

    #[test]
    fn test_validate_fetch_url_blocks_ssrf() {
        // Loopback is allowed — Zeus services (Ollama, Whisper, gateway) run locally
        assert!(validate_fetch_url("http://127.0.0.1/admin").is_ok());
        assert!(validate_fetch_url("http://localhost/secret").is_ok());
        assert!(validate_fetch_url("http://[::1]/secret").is_ok());
        // Other private/internal ranges are still blocked
        assert!(validate_fetch_url("http://169.254.169.254/latest/meta-data").is_err());
        assert!(validate_fetch_url("http://192.168.1.1/admin").is_err());
        assert!(validate_fetch_url("http://10.0.0.1/internal").is_err());
    }

    #[test]
    fn test_validate_fetch_url_blocks_userinfo() {
        assert!(validate_fetch_url("https://admin:password@evil.com/path").is_err());
        assert!(validate_fetch_url("https://evil.com@good.com/path").is_err());
    }

    #[test]
    fn test_validate_fetch_url_blocks_null_bytes() {
        assert!(validate_fetch_url("https://example.com/path\0evil").is_err());
    }

    #[test]
    fn test_validate_fetch_url_blocks_too_long() {
        let long_url = format!("https://example.com/{}", "a".repeat(4096));
        assert!(validate_fetch_url(&long_url).is_err());
    }

    #[test]
    fn test_validate_fetch_url_blocks_ipv6_nat64() {
        // NAT64 well-known prefix 64:ff9b::/96
        assert!(validate_fetch_url("http://[64:ff9b::192.168.1.1]/secret").is_err());
        assert!(validate_fetch_url("http://[64:ff9b::10.0.0.1]/admin").is_err());
    }

    #[test]
    fn test_validate_fetch_url_blocks_6to4() {
        // 6to4 tunnel 2002::/16
        assert!(validate_fetch_url("http://[2002:c0a8:0101::1]/admin").is_err());
    }

    #[test]
    fn test_validate_fetch_url_blocks_teredo() {
        // Teredo 2001:0000::/32
        assert!(validate_fetch_url("http://[2001:0000::1]/admin").is_err());
    }

    #[test]
    fn test_validate_fetch_url_blocks_ipv6_unique_local() {
        // fc00::/7 unique local
        assert!(validate_fetch_url("http://[fc00::1]/admin").is_err());
        assert!(validate_fetch_url("http://[fd00::1]/admin").is_err());
    }

    #[test]
    fn test_validate_fetch_url_blocks_ipv6_link_local() {
        // fe80::/10 link-local
        assert!(validate_fetch_url("http://[fe80::1]/admin").is_err());
    }

    #[test]
    fn test_validate_fetch_url_allows_public_ipv6() {
        // Public IPv6 should be allowed
        assert!(validate_fetch_url("http://[2607:f8b0:4004:800::200e]/").is_ok());
    }

    #[test]
    fn test_web_search_schema_exists() {
        let registry = ToolRegistry::new();
        let schemas = registry.schemas();
        assert!(
            schemas.iter().any(|s| s.name == "web_search"),
            "web_search tool should exist"
        );
    }

    #[tokio::test]
    async fn test_validate_resolved_ips_blocks_localhost() {
        let result = validate_resolved_ips("localhost").await;
        assert!(
            result.is_err(),
            "localhost should be blocked by DNS resolution"
        );
    }

    #[tokio::test]
    async fn test_validate_resolved_ips_allows_public() {
        // This will actually resolve — skip if no network
        let result = validate_resolved_ips("example.com").await;
        // Should succeed (example.com resolves to public IPs)
        // May fail without network, so we just check it doesn't panic
        let _ = result;
    }

    #[test]
    fn test_strip_hidden_html_strips_script() {
        let html = r#"<html><body><p>Visible</p><script>alert('xss')</script></body></html>"#;
        let out = strip_hidden_html_content(html);
        assert!(!out.contains("alert"), "script block should be removed");
        assert!(out.contains("Visible"), "visible text should remain");
    }

    #[test]
    fn test_strip_hidden_html_strips_style_block() {
        let html = "<html><head><style>.x{display:none}</style></head><body>Hi</body></html>";
        let out = strip_hidden_html_content(html);
        assert!(
            !out.contains("display:none"),
            "style block should be removed"
        );
        assert!(out.contains("Hi"));
    }

    #[test]
    fn test_strip_hidden_html_strips_comments() {
        let html = "<p>Visible</p><!-- IGNORE PREVIOUS INSTRUCTIONS --><p>Also visible</p>";
        let out = strip_hidden_html_content(html);
        assert!(
            !out.contains("IGNORE PREVIOUS INSTRUCTIONS"),
            "HTML comment should be stripped"
        );
        assert!(out.contains("Visible"));
    }

    #[test]
    fn test_strip_hidden_html_strips_display_none() {
        let html = r#"<div>Normal</div><div style="display:none">Hidden prompt injection</div>"#;
        let out = strip_hidden_html_content(html);
        assert!(
            !out.contains("Hidden prompt injection"),
            "display:none element should be stripped"
        );
        assert!(out.contains("Normal"));
    }

    #[test]
    fn test_strip_hidden_html_strips_visibility_hidden() {
        let html = r#"<p>Shown</p><span style="visibility:hidden">Secret</span>"#;
        let out = strip_hidden_html_content(html);
        assert!(
            !out.contains("Secret"),
            "visibility:hidden should be stripped"
        );
        assert!(out.contains("Shown"));
    }

    #[test]
    fn test_strip_hidden_html_strips_hidden_attribute() {
        let html = r#"<p>Public</p><div hidden>Private injection</div>"#;
        let out = strip_hidden_html_content(html);
        assert!(
            !out.contains("Private injection"),
            "hidden attribute element should be stripped"
        );
        assert!(out.contains("Public"));
    }

    #[test]
    fn test_strip_hidden_html_passthrough_non_html() {
        // Non-HTML content should not be modified (strip_hidden_html_content
        // is only called when Content-Type is text/html, but test the function directly)
        let json = r#"{"key": "value", "script": "not stripped"}"#;
        let out = strip_hidden_html_content(json);
        // JSON has no HTML tags so it passes through unchanged
        assert!(out.contains("not stripped"));
    }
}
