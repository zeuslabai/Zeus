//! Zeus Aegis - Security, sandboxing, and credential management
//!
//! This crate provides:
//! - OS keychain integration (macOS Keychain, Linux Secret Service)
//! - Multi-level sandboxing (seccomp on Linux, Seatbelt on macOS)
//! - Network allowlisting and filtering
//! - Tamper-evident audit logging
//!
//! ## Sandboxing
//!
//! Aegis provides platform-specific sandboxing:
//!
//! - **macOS**: Uses sandbox-exec with Seatbelt profiles (SBPL)
//! - **Linux**: Uses seccomp-bpf for syscall filtering
//!
//! Sandbox levels (from least to most restrictive):
//! - `None`: No restrictions (development only)
//! - `Basic`: Block dangerous operations (ptrace, kexec, etc.)
//! - `Standard`: Restricted filesystem access
//! - `Strict`: Network allowlist + restricted filesystem
//! - `Paranoid`: Minimal permissions only

pub mod approvals;
pub mod audit;
pub mod audit_jsonl;
pub mod credential_vault;
pub mod keychain;
pub mod manifest;
pub mod merkle;
pub mod network;
pub mod permissions;
pub mod rate_limiter;
pub mod sandbox;
pub mod taint;
pub mod tool_replay;
pub use tool_replay::{
    DiffResult, Recording, ReplayConfig, ReplayEntry, ReplayFilter, ReplayReport, ReplayStats,
    ToolReplayRecorder,
};

// Platform-specific modules
#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(target_os = "linux")]
pub mod linux;

pub use approvals::{
    ApprovalManager, ApprovalOutcome, ApprovalRequest, ApprovalResult, ApprovalSink, QueuedApproval,
};
pub use audit::{AuditLog, AuditQuery, PatternConfig, Severity, SuspiciousAlert};
pub use manifest::{
    AgentManifest, SignedManifest, generate_agent_keypair, load_agent_keypair,
    load_or_generate_keypair, sign_manifest, verify_manifest, verify_manifest_with_key,
};
pub use merkle::{MerkleAuditLog, MerkleProof, MerkleState, MerkleTree, verify_proof};
pub use audit_jsonl::{AuditJsonlWriter, ToolAuditEntry};
pub use credential_vault::CredentialVault;
pub use keychain::Keychain;
pub use network::{NetworkFilter, NetworkStats};
pub use permissions::PermissionSet;
pub use rate_limiter::{
    ClientUsage, EndpointLimit, RateLimitConfig, RateLimitResult, RateLimitStats, RateLimiter,
};
pub use sandbox::{Sandbox, SandboxLevel};
pub use taint::{SinkMode, SinkPolicy, TaintTracker, TaintViolation};
pub use risk::{RiskLevel, tool_risk};

pub mod risk;

// Re-export platform-specific sandboxing
#[cfg(target_os = "macos")]
pub use macos::seatbelt::{SeatbeltProfile, SeatbeltSandbox};

#[cfg(target_os = "linux")]
pub use linux::seccomp::{SeccompFilter, SeccompSandbox};

use zeus_core::Result;

/// Permission mode for Aegis approval decisions
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AegisMode {
    /// Always prompt the user for approval (default)
    #[default]
    Prompt,
    /// Use LLM classifier to auto-approve/deny; fall back to prompt on error
    Auto,
    /// Allow all tool calls without approval
    Bypass,
}

/// Main security manager
pub struct Aegis {
    keychain: Keychain,
    sandbox: Sandbox,
    audit: AuditLog,
    permissions: PermissionSet,
    network: NetworkFilter,
    approval_manager: ApprovalManager,
    auto_allow: Vec<String>,
    mode: AegisMode,
    /// API key for the LLM classifier (used when mode = Auto)
    classifier_api_key: Option<String>,
    /// Base URL for the classifier (default: Anthropic)
    classifier_api_url: Option<String>,
}

impl Aegis {
    /// Create a new Aegis instance
    pub async fn new(config: AegisConfig) -> Result<Self> {
        let keychain = Keychain::new(&config.keychain_service)?;
        let mut sandbox = Sandbox::new(config.sandbox_level);

        // Configure sandbox with network allowlist
        for host in &config.network_allowlist {
            sandbox.allow_host(host);
        }

        // Add explicitly allowed write paths from config
        for path in &config.allowed_write_paths {
            sandbox.allow_path(path);
        }

        // Grant access to standard system paths for autonomous deployments.
        // OS-level permissions remain the actual enforcement layer.
        if config.allow_system_paths {
            for path in system_paths() {
                sandbox.allow_path(*path);
            }
        }

        let audit = AuditLog::new(&config.audit_path).await?;

        // Verify audit log chain integrity on startup
        if config.audit_path.exists() {
            match audit.verify().await {
                Ok(true) => {
                    tracing::debug!(
                        "Audit log chain verified OK ({} entries)",
                        audit.entry_count()
                    );
                }
                Ok(false) => {
                    tracing::error!(
                        "⚠️  AUDIT LOG TAMPER DETECTED — hash chain broken at {}",
                        config.audit_path.display()
                    );
                }
                Err(e) => {
                    tracing::warn!("Audit log verification failed: {} — continuing", e);
                }
            }

            // Run suspicious pattern detection on startup
            match audit.detect_suspicious_patterns().await {
                Ok(alerts) if !alerts.is_empty() => {
                    for alert in &alerts {
                        tracing::warn!(
                            "🚨 Security alert on startup: {} (severity: {}, {} related entries)",
                            alert.description,
                            alert.severity,
                            alert.related_sequences.len()
                        );
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!("Suspicious pattern scan failed: {}", e);
                }
            }
        }

        let permissions = PermissionSet::new(config.permissions);

        // Configure network filter
        let mut network = if config.sandbox_level >= SandboxLevel::Strict {
            NetworkFilter::default_allowlist()
        } else {
            NetworkFilter::allow_all()
        };

        // Add custom allowlist entries
        for host in &config.network_allowlist {
            if host != "*" {
                network.allow_host(host);
            }
        }

        if config.sandbox_level >= SandboxLevel::Strict {
            network.enable();
        }

        // Build approval manager from config
        let approval_manager = ApprovalManager::new(
            config.dangerous_patterns.clone(),
            config.tools_requiring_approval.clone(),
        )
        .with_timeout(config.approval_timeout_secs);

        // Apply OS-level sandboxing based on configured level.
        // Minimal = no sandbox, Standard = path restrictions, Strict = full isolation.
        // Skip in test builds — sandbox_init() persists for the process lifetime,
        // which locks down all subsequent tests that need temp dirs or file writes.
        #[cfg(not(test))]
        if config.sandbox_level > SandboxLevel::None {
            match sandbox.apply() {
                Ok(()) => {
                    tracing::info!(
                        "Aegis sandbox applied (level: {:?})",
                        config.sandbox_level
                    );
                }
                Err(e) => {
                    // Don't fail startup on sandbox errors — log and continue.
                    // Sandbox may not be available on all platforms.
                    tracing::warn!(
                        "Aegis sandbox apply failed (level: {:?}): {} — running unsandboxed",
                        config.sandbox_level, e
                    );
                }
            }
        }

        Ok(Self {
            keychain,
            sandbox,
            audit,
            permissions,
            network,
            approval_manager,
            auto_allow: config.auto_allow.clone(),
            mode: config.mode.clone(),
            classifier_api_key: config.classifier_api_key.clone(),
            classifier_api_url: config.classifier_api_url.clone(),
        })
    }

    /// Get a secret from the keychain
    pub async fn get_secret(&self, key: &str) -> Result<Option<String>> {
        self.keychain.get(key).await
    }

    /// Store a secret in the keychain
    pub async fn set_secret(&self, key: &str, value: &str) -> Result<()> {
        self.keychain.set(key, value).await
    }

    /// Delete a secret from the keychain
    pub async fn delete_secret(&self, key: &str) -> Result<()> {
        self.keychain.delete(key).await
    }

    /// Check if an operation is permitted
    pub fn is_permitted(&self, operation: &str) -> bool {
        self.permissions.is_allowed(operation)
    }

    /// Check if a network host is allowed
    pub fn check_network_host(&self, host: &str) -> Result<()> {
        self.network.check_host(host)
    }

    /// Check if a URL is allowed
    pub fn check_network_url(&self, url: &str) -> Result<()> {
        self.network.check_url(url)
    }

    /// Log a security event
    pub async fn log_event(&mut self, event: audit::AuditEvent) -> Result<()> {
        self.audit.log(event).await
    }

    /// Get current sandbox level
    pub fn sandbox_level(&self) -> SandboxLevel {
        self.sandbox.level()
    }

    /// Get network filter
    pub fn network_filter(&self) -> &NetworkFilter {
        &self.network
    }

    /// Get mutable network filter
    pub fn network_filter_mut(&mut self) -> &mut NetworkFilter {
        &mut self.network
    }

    /// Check if the sandbox restricts filesystem access
    pub fn restricts_filesystem(&self) -> bool {
        self.sandbox.level().restricts_filesystem()
    }

    /// Check if the sandbox restricts network access
    pub fn restricts_network(&self) -> bool {
        self.sandbox.level().restricts_network()
    }

    /// Check if a filesystem path is allowed
    pub fn is_path_allowed(&self, path: &str) -> bool {
        self.sandbox.is_path_allowed(path)
    }

    /// Wrap a shell command with sandbox enforcement.
    ///
    /// On macOS, wraps the command with `sandbox-exec -p '<profile>'` using a
    /// Seatbelt profile that restricts filesystem access to the workspace directory
    /// and temp files, based on the configured sandbox level.
    ///
    /// At `SandboxLevel::None`, returns the command unchanged.
    /// On non-macOS platforms, returns the command unchanged.
    pub fn sandbox_command(&self, command: &str) -> String {
        self.sandbox.sandbox_command(command)
    }

    /// Validate a shell command against security policies.
    ///
    /// This performs deeper analysis than just checking the command string as a
    /// path. It:
    /// - Extracts absolute paths referenced in the command and checks each one
    /// - Blocks commands that contain known-dangerous binaries
    /// - Detects command chaining used to bypass single-command analysis
    pub fn validate_shell_command(&self, command: &str) -> Result<()> {
        // S65: "none" means truly none — skip ALL validation.
        // Path extraction was corrupting/rejecting valid commands even
        // when the user explicitly set sandbox_level = "none".
        if self.sandbox.level() == SandboxLevel::None {
            return Ok(());
        }

        // Block command substitution injection patterns regardless of sandbox level.
        // $(...) and backticks allow arbitrary command execution hidden inside arguments.
        if command.contains("$(") || command.contains('`') {
            return Err(zeus_core::Error::Security(
                "Command contains substitution injection ($() or backticks) — use explicit arguments instead".to_string(),
            ));
        }

        // Blocked command prefixes: dangerous operations that should require explicit permission
        let blocked_commands = [
            "shutdown",
            "reboot",
            "halt",
            "poweroff",
            "init 0",
            "init 6",
            "launchctl unload",
            "systemctl disable",
        ];

        // Split on shell separators and check EACH segment independently.
        // This catches injection through chaining: `ls; shutdown`, `echo ok && reboot`,
        // `cat file | shutdown`. The old prefix-only check on the full string missed these.
        let segments: Vec<&str> = command
            .split(|c| c == ';' || c == '|')
            .flat_map(|s| s.split("&&"))
            .flat_map(|s| s.split("||"))
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();

        for segment in &segments {
            for blocked in &blocked_commands {
                if segment.starts_with(blocked) {
                    return Err(zeus_core::Error::Security(format!(
                        "Command '{}' is blocked by security policy",
                        blocked
                    )));
                }
            }
        }

        // If filesystem is restricted, extract and validate all absolute paths in the command
        if self.restricts_filesystem() {
            // Split on common shell separators to find path-like tokens
            let tokens: Vec<&str> = command
                .split(|c: char| {
                    c.is_whitespace()
                        || c == ';'
                        || c == '|'
                        || c == '&'
                        || c == '>'
                        || c == '<'
                        || c == '`'
                        || c == '$'
                        || c == '('
                        || c == ')'
                })
                .filter(|t| t.starts_with('/'))
                .collect();

            for token in &tokens {
                // Clean up quotes around the path
                let path = token.trim_matches(|c: char| c == '\'' || c == '"');
                if !self.sandbox.is_path_allowed(path) {
                    return Err(zeus_core::Error::Security(format!(
                        "Command references restricted path: {}",
                        path
                    )));
                }
            }
        }

        Ok(())
    }

    /// Connect an external approval sink (e.g., the API approval queue).
    ///
    /// When connected, `queue_for_approval()` forwards sensitive tool
    /// calls through this channel to the API layer, which creates
    /// `PendingApproval` entries visible via REST and WebSocket.
    pub fn set_approval_sink(&mut self, sink: ApprovalSink) {
        self.approval_manager.set_approval_sink(sink);
    }

    /// Queue a tool call for approval before execution.
    ///
    /// Checks whether the given tool/args combination triggers an
    /// approval requirement (dangerous pattern match or tool in the
    /// `tools_requiring_approval` list). If so, submits a
    /// `QueuedApproval` to the external approval queue (API) or the
    /// internal oneshot system, and waits for the resolution.
    ///
    /// Returns `ApprovalOutcome::NotRequired` when the tool is safe,
    /// `Approved` when approved, `Denied` when rejected, or `Expired`
    /// on timeout.
    pub async fn queue_for_approval(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
        agent_id: Option<String>,
    ) -> ApprovalOutcome {
        self.approval_manager
            .queue_for_approval(tool_name, args, agent_id)
            .await
    }

    /// Check if a tool call would require approval (without queueing).
    ///
    /// In `Bypass` mode: always returns false.
    /// In `Prompt` mode: checks auto_allow patterns + deny list.
    /// In `Auto` mode: same sync check (LLM classifier runs in `needs_approval_async`).
    pub fn needs_approval(&self, tool_name: &str, args: &serde_json::Value) -> bool {
        match self.mode {
            AegisMode::Bypass => return false,
            AegisMode::Prompt | AegisMode::Auto => {}
        }
        // S101 #18: check auto-allow patterns first
        if self.is_auto_allowed(tool_name, args) {
            return false;
        }
        self.approval_manager.needs_approval(tool_name, args)
    }

    /// Async approval check — races all three resolvers in `Auto` mode.
    ///
    /// Resolver priority (first to answer wins):
    /// 1. `auto_allow` glob patterns  → allow immediately
    /// 2. `tools_requiring_approval` deny list → require approval
    /// 3. LLM classifier (Haiku-class) → allow or require approval
    ///
    /// Falls back to `needs_approval()` (sync) in `Prompt` / `Bypass` mode.
    pub async fn needs_approval_async(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> bool {
        match self.mode {
            AegisMode::Bypass => return false,
            AegisMode::Prompt => return self.needs_approval(tool_name, args),
            AegisMode::Auto => {}
        }

        // Resolver 1: auto_allow glob — instant allow
        if self.is_auto_allowed(tool_name, args) {
            tracing::debug!(tool = %tool_name, "auto-allow pattern matched — no approval needed");
            return false;
        }

        // Resolver 2: deny list — instant require approval
        if self.approval_manager.needs_approval(tool_name, args) {
            tracing::debug!(tool = %tool_name, "deny list matched — approval required");
            return true;
        }

        // Resolver 3: LLM classifier
        match self.run_llm_classifier(tool_name, args).await {
            Ok(allow) => {
                tracing::debug!(tool = %tool_name, allow, "LLM classifier result");
                !allow
            }
            Err(e) => {
                tracing::warn!(tool = %tool_name, error = %e, "LLM classifier failed — defaulting to require approval");
                true
            }
        }
    }

    /// Run a fast LLM classifier to decide allow/deny for a tool call.
    ///
    /// Sends tool name + args to a Haiku-class model and parses the response.
    /// Returns `Ok(true)` = allow, `Ok(false)` = deny.
    async fn run_llm_classifier(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> std::result::Result<bool, String> {
        let api_key = self
            .classifier_api_key
            .as_deref()
            .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok().as_deref().map(|_| ""))
            .ok_or_else(|| "No API key for LLM classifier".to_string())?;

        // Resolve the key from env if not set in config
        let api_key = if api_key.is_empty() {
            std::env::var("ANTHROPIC_API_KEY")
                .map_err(|_| "ANTHROPIC_API_KEY not set".to_string())?
        } else {
            api_key.to_string()
        };

        let base_url = self
            .classifier_api_url
            .as_deref()
            .unwrap_or("https://api.anthropic.com");

        let args_summary = match args {
            serde_json::Value::Object(m) => m
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join(", "),
            other => other.to_string(),
        };

        let prompt = format!(
            "You are a security classifier for an AI agent. \
             Decide whether the following tool call is safe to execute without human approval.\n\
             Tool: {tool_name}\n\
             Args: {args_summary}\n\n\
             Reply with exactly one word: ALLOW or DENY.\n\
             ALLOW = safe, routine operation (read files, list dirs, web searches, run tests, etc.)\n\
             DENY = destructive, irreversible, or high-risk (delete data, system changes, credential access, etc.)"
        );

        let body = serde_json::json!({
            "model": "claude-haiku-4-5",
            "max_tokens": 10,
            "messages": [{"role": "user", "content": prompt}]
        });

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(8))
            .build()
            .map_err(|e| format!("HTTP client error: {e}"))?;

        let resp = client
            .post(format!("{base_url}/v1/messages"))
            .header("x-api-key", &api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Classifier request failed: {e}"))?;

        if !resp.status().is_success() {
            return Err(format!("Classifier HTTP {}", resp.status()));
        }

        let resp_json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("Failed to parse classifier response: {e}"))?;

        let text = resp_json["content"][0]["text"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_uppercase();

        Ok(text.starts_with("ALLOW"))
    }

    /// Check if a tool call matches any auto-allow glob pattern.
    /// Patterns are "ToolName(glob)" e.g. "shell(git *)", "read_file(src/**)".
    fn is_auto_allowed(&self, tool_name: &str, args: &serde_json::Value) -> bool {
        let arg_str = match args {
            serde_json::Value::Object(map) => {
                // Use the first string arg (typically "command", "path", etc.)
                map.values()
                    .find_map(|v| v.as_str())
                    .unwrap_or("")
            }
            serde_json::Value::String(s) => s.as_str(),
            _ => "",
        };
        for pattern in &self.auto_allow {
            // Parse "ToolName(glob)" format
            if let Some(paren) = pattern.find('(') {
                let pat_tool = &pattern[..paren];
                let pat_glob = pattern[paren + 1..].trim_end_matches(')');
                // Tool name match (case-insensitive)
                if pat_tool.eq_ignore_ascii_case(tool_name) || pat_tool == "*" {
                    // Glob match on the argument
                    if pat_glob == "*" || pat_glob == "**" {
                        return true;
                    }
                    if arg_str.starts_with(pat_glob.trim_end_matches('*')) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Get a reference to the internal approval manager.
    pub fn approval_manager(&self) -> &ApprovalManager {
        &self.approval_manager
    }

    /// Get a mutable reference to the internal approval manager.
    pub fn approval_manager_mut(&mut self) -> &mut ApprovalManager {
        &mut self.approval_manager
    }

    /// Verify audit log integrity
    pub async fn verify_audit_log(&self) -> Result<bool> {
        self.audit.verify().await
    }
}

/// Aegis configuration
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct AegisConfig {
    /// Keychain service name
    #[serde(default = "default_keychain_service")]
    pub keychain_service: String,

    /// Sandbox level
    #[serde(default)]
    pub sandbox_level: SandboxLevel,

    /// Audit log path
    #[serde(default = "default_audit_path")]
    pub audit_path: std::path::PathBuf,

    /// Allowed operations
    #[serde(default)]
    pub permissions: Vec<String>,

    /// Network allowlist
    #[serde(default)]
    pub network_allowlist: Vec<String>,

    /// Shell command patterns that trigger approval (e.g., "rm -rf", "sudo", "DROP TABLE")
    #[serde(default)]
    pub dangerous_patterns: Vec<String>,

    /// Tools that always require approval before execution
    #[serde(default)]
    pub tools_requiring_approval: Vec<String>,

    /// Glob patterns for auto-allowed tool operations. Matching operations
    /// skip the approval prompt. Examples: "Bash(git *)", "Edit(src/**)",
    /// "Read(*)", "shell(cargo test *)". Format: "ToolName(glob_pattern)".
    #[serde(default)]
    pub auto_allow: Vec<String>,

    /// Approval mode: "prompt" (always ask), "auto" (LLM classifier), "bypass" (allow all)
    #[serde(default)]
    pub mode: AegisMode,

    /// API key for the LLM classifier (auto mode). Defaults to ANTHROPIC_API_KEY env var.
    #[serde(default)]
    pub classifier_api_key: Option<String>,

    /// Base URL for the LLM classifier API (auto mode). Defaults to https://api.anthropic.com
    #[serde(default)]
    pub classifier_api_url: Option<String>,

    /// Timeout for approval requests in seconds (default 300 = 5 minutes)
    #[serde(default = "default_approval_timeout_secs")]
    pub approval_timeout_secs: u64,

    /// Additional filesystem paths that Zeus may write to, beyond the workspace.
    /// Useful for DevOps deployments that need to edit config files in system
    /// directories (e.g. "/usr/local/etc/", "/usr/local/bin/").
    /// Each entry is treated as a prefix — subdirectories are included.
    ///
    /// Example in config.toml:
    /// ```toml
    /// [aegis]
    /// allowed_write_paths = ["/usr/local/etc/", "/usr/local/bin/"]
    /// ```
    #[serde(default)]
    pub allowed_write_paths: Vec<String>,

    /// Grant autonomous agents write access to standard system paths:
    /// `/usr/local/`, `/usr/local/bin/`, `/usr/local/sbin/`,
    /// `/usr/local/etc/`, `/usr/local/lib/`, `/usr/local/share/`, `/etc/`.
    ///
    /// OS-level permissions still apply — Zeus cannot write to paths the
    /// running user doesn't own. This flag only removes the Aegis-layer
    /// sandbox restriction so the OS can give its own answer.
    ///
    /// Set to `true` for autonomous DevOps deployments (install binaries,
    /// write config files, manage services). Leave `false` on shared or
    /// untrusted machines.
    ///
    /// ```toml
    /// [aegis]
    /// allow_system_paths = true
    /// ```
    #[serde(default)]
    pub allow_system_paths: bool,
}

fn default_keychain_service() -> String {
    "zeus".to_string()
}

/// Standard system paths granted when `allow_system_paths = true`.
///
/// These cover the common locations where autonomous agents install
/// binaries, write config files, and manage service data on
/// Unix-like systems (Linux, macOS, FreeBSD).
pub fn system_paths() -> &'static [&'static str] {
    &[
        "/usr/local/",
        "/usr/local/bin/",
        "/usr/local/sbin/",
        "/usr/local/etc/",
        "/usr/local/lib/",
        "/usr/local/share/",
        "/etc/",
    ]
}

fn default_audit_path() -> std::path::PathBuf {
    zeus_core::default_config_dir().join("audit.log")
}

fn default_approval_timeout_secs() -> u64 {
    300
}

impl Default for AegisConfig {
    fn default() -> Self {
        Self {
            keychain_service: default_keychain_service(),
            sandbox_level: SandboxLevel::default(),
            audit_path: default_audit_path(),
            // Default to explicit permission categories instead of wildcard.
            // Users who want unrestricted access can set ["*"] in config.toml.
            permissions: vec![
                "fs.read".to_string(),
                "fs.write".to_string(),
                "fs.execute".to_string(),
                "net.http".to_string(),
                "net.dns".to_string(),
                "sys.shell".to_string(),
                "sys.env".to_string(),
                "tool.mcp".to_string(),
                "tool.skill".to_string(),
                "channel.send".to_string(),
                "channel.receive".to_string(),
            ],
            // Default to common safe domains instead of wildcard.
            // Users who want unrestricted network can set ["*"] in config.toml.
            network_allowlist: vec![
                "*.anthropic.com".to_string(),
                "*.openai.com".to_string(),
                "*.googleapis.com".to_string(),
                "*.groq.com".to_string(),
                "*.mistral.ai".to_string(),
                "*.together.xyz".to_string(),
                "*.fireworks.ai".to_string(),
                "api.telegram.org".to_string(),
                "discord.com".to_string(),
                "*.discord.com".to_string(),
                "*.slack.com".to_string(),
                "*.github.com".to_string(),
                "localhost".to_string(),
                "127.0.0.1".to_string(),
            ],
            dangerous_patterns: vec![
                "rm -rf /".to_string(),
                "rm -rf ~".to_string(),
                "mkfs.".to_string(),
                "dd if=/dev/zero".to_string(),
                ":(){ :|:& };:".to_string(),
                "> /dev/sda".to_string(),
                "chmod -R 777 /".to_string(),
                "sudo rm".to_string(),
            ],
            tools_requiring_approval: Vec::new(),
            approval_timeout_secs: default_approval_timeout_secs(),
            auto_allow: Vec::new(),
            allowed_write_paths: Vec::new(),
            allow_system_paths: false,
            mode: AegisMode::Prompt,
            classifier_api_key: None,
            classifier_api_url: None,
        }
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;

    /// Helper to create an Aegis instance for testing with custom permissions
    async fn aegis_with_permissions(
        permissions: Vec<String>,
        sandbox_level: SandboxLevel,
        network_allowlist: Vec<String>,
    ) -> Aegis {
        let tmp = tempfile::tempdir().expect("should create temp dir");
        let config = AegisConfig {
            keychain_service: "zeus-test".to_string(),
            sandbox_level,
            audit_path: tmp.path().join("audit.log"),
            permissions,
            network_allowlist,
            ..Default::default()
        };
        Aegis::new(config).await.expect("Aegis::new should succeed")
    }

    #[tokio::test]
    async fn test_full_security_check_read_file() {
        let aegis =
            aegis_with_permissions(vec!["fs.read".to_string()], SandboxLevel::Standard, vec![])
                .await;

        // fs.read should be permitted
        assert!(aegis.is_permitted("fs.read"));
        // fs.write should not
        assert!(!aegis.is_permitted("fs.write"));
        // shell should not
        assert!(!aegis.is_permitted("sys.shell"));
    }

    #[tokio::test]
    async fn test_full_security_check_wildcard_fs() {
        let aegis =
            aegis_with_permissions(vec!["fs.*".to_string()], SandboxLevel::Standard, vec![]).await;

        assert!(aegis.is_permitted("fs.read"));
        assert!(aegis.is_permitted("fs.write"));
        assert!(aegis.is_permitted("fs.delete"));
        assert!(!aegis.is_permitted("net.http"));
        assert!(!aegis.is_permitted("sys.shell"));
    }

    #[tokio::test]
    async fn test_network_filter_strict_mode() {
        let aegis = aegis_with_permissions(
            vec!["*".to_string()],
            SandboxLevel::Strict,
            vec!["api.anthropic.com".to_string()],
        )
        .await;

        // Allowed host should pass
        assert!(aegis.check_network_host("api.anthropic.com").is_ok());
        // Unknown host should be blocked in strict mode
        assert!(aegis.check_network_host("evil.example.com").is_err());
    }

    #[tokio::test]
    async fn test_network_url_check_rejects_userinfo_bypass() {
        let aegis = aegis_with_permissions(
            vec!["*".to_string()],
            SandboxLevel::Strict,
            vec!["api.anthropic.com".to_string()],
        )
        .await;

        // Attempt userinfo bypass: should extract real host, not the fake one
        assert!(
            aegis
                .check_network_url("https://api.anthropic.com@evil.com/steal")
                .is_err()
        );
    }

    #[tokio::test]
    async fn test_path_traversal_blocked_in_sandbox() {
        let aegis = aegis_with_permissions(
            vec!["*".to_string()],
            SandboxLevel::Strict,
            vec!["*".to_string()],
        )
        .await;

        // Strict sandbox restricts filesystem paths
        assert!(aegis.restricts_filesystem());
        // Paths outside allowed dirs should be blocked
        assert!(!aegis.is_path_allowed("/etc/shadow"));
    }

    #[tokio::test]
    async fn test_sandbox_none_allows_everything() {
        let aegis = aegis_with_permissions(
            vec!["*".to_string()],
            SandboxLevel::None,
            vec!["*".to_string()],
        )
        .await;

        assert!(!aegis.restricts_filesystem());
        assert!(!aegis.restricts_network());
    }

    #[tokio::test]
    async fn test_deny_all_permissions() {
        let aegis = aegis_with_permissions(vec![], SandboxLevel::Standard, vec![]).await;

        assert!(!aegis.is_permitted("fs.read"));
        assert!(!aegis.is_permitted("fs.write"));
        assert!(!aegis.is_permitted("sys.shell"));
        assert!(!aegis.is_permitted("net.http"));
    }

    #[tokio::test]
    async fn test_audit_log_integrity() {
        let tmp = tempfile::tempdir().expect("should create temp dir");
        let config = AegisConfig {
            keychain_service: "zeus-test".to_string(),
            sandbox_level: SandboxLevel::None,
            audit_path: tmp.path().join("audit.log"),
            permissions: vec!["*".to_string()],
            network_allowlist: vec!["*".to_string()],
            ..Default::default()
        };
        let mut aegis = Aegis::new(config).await.expect("Aegis::new should succeed");

        // Write some audit events first
        aegis
            .log_event(audit::AuditEvent::System {
                event: "test".to_string(),
                details: None,
            })
            .await
            .expect("async operation should succeed");

        aegis
            .log_event(audit::AuditEvent::PermissionCheck {
                operation: "fs.read".to_string(),
                allowed: true,
            })
            .await
            .expect("async operation should succeed");

        // Now verify the audit log integrity
        let result = aegis.verify_audit_log().await;
        assert!(result.is_ok());
        assert!(
            result.expect("operation should succeed"),
            "Audit log should verify as intact"
        );
    }

    #[tokio::test]
    async fn test_validate_shell_command_blocks_dangerous() {
        let aegis = aegis_with_permissions(
            vec!["*".to_string()],
            SandboxLevel::Standard,
            vec!["*".to_string()],
        )
        .await;

        // Blocked commands
        assert!(aegis.validate_shell_command("shutdown -h now").is_err());
        assert!(aegis.validate_shell_command("reboot").is_err());
        assert!(aegis.validate_shell_command("halt").is_err());
    }

    #[tokio::test]
    async fn test_validate_shell_command_checks_paths() {
        let aegis = aegis_with_permissions(
            vec!["*".to_string()],
            SandboxLevel::Strict,
            vec!["*".to_string()],
        )
        .await;

        // Strict sandbox restricts filesystem, and /etc/shadow is not an allowed path
        assert!(aegis.restricts_filesystem());
        assert!(aegis.validate_shell_command("cat /etc/shadow").is_err());
    }

    #[tokio::test]
    async fn test_allowed_write_paths_grants_access() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let config = AegisConfig {
            keychain_service: "zeus-test".to_string(),
            sandbox_level: SandboxLevel::Standard,
            audit_path: tmp.path().join("audit.log"),
            permissions: vec!["*".to_string()],
            network_allowlist: vec!["*".to_string()],
            allowed_write_paths: vec!["/usr/local/etc/".to_string()],
            ..Default::default()
        };
        let aegis = Aegis::new(config).await.expect("Aegis::new");

        // /usr/local/etc/ is in the whitelist — writes should be allowed
        assert!(aegis.validate_shell_command("echo 'bind 127.0.0.1' >> /usr/local/etc/redis.conf").is_ok());
        assert!(aegis.validate_shell_command("sed -i '' 's/old/new/' /usr/local/etc/nginx/nginx.conf").is_ok());
    }

    #[tokio::test]
    async fn test_allowed_write_paths_does_not_affect_other_paths() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let config = AegisConfig {
            keychain_service: "zeus-test".to_string(),
            sandbox_level: SandboxLevel::Strict,
            audit_path: tmp.path().join("audit.log"),
            permissions: vec!["*".to_string()],
            network_allowlist: vec!["*".to_string()],
            allowed_write_paths: vec!["/usr/local/etc/".to_string()],
            ..Default::default()
        };
        let aegis = Aegis::new(config).await.expect("Aegis::new");

        // Sensitive paths not in the whitelist must still be blocked
        assert!(aegis.validate_shell_command("cat /etc/shadow").is_err());
    }

    #[tokio::test]
    async fn test_allowed_write_paths_empty_preserves_default_behavior() {
        // Empty list → no change from default Strict sandbox behavior
        let aegis = aegis_with_permissions(
            vec!["*".to_string()],
            SandboxLevel::Strict,
            vec!["*".to_string()],
        )
        .await;

        assert!(aegis.restricts_filesystem());
        // Without explicit paths, /usr/local/etc/ is still restricted
        assert!(aegis.validate_shell_command("echo test >> /usr/local/etc/redis.conf").is_err());
    }

    #[tokio::test]
    async fn test_validate_shell_command_allows_normal() {
        let aegis = aegis_with_permissions(
            vec!["*".to_string()],
            SandboxLevel::None,
            vec!["*".to_string()],
        )
        .await;

        // With no filesystem restrictions, normal commands should pass
        assert!(aegis.validate_shell_command("ls -la").is_ok());
        assert!(aegis.validate_shell_command("echo hello").is_ok());
    }

    #[tokio::test]
    async fn test_validate_shell_command_blocks_chained() {
        let aegis = aegis_with_permissions(
            vec!["*".to_string()],
            SandboxLevel::Standard,
            vec!["*".to_string()],
        )
        .await;

        // Chained commands that include blocked operations
        assert!(
            aegis
                .validate_shell_command("echo ok && shutdown -h now")
                .is_err()
        );
    }

    // ========================================================================
    // Aegis::queue_for_approval integration tests
    // ========================================================================

    /// Helper to create an Aegis instance with approval config
    async fn aegis_with_approvals(
        dangerous_patterns: Vec<String>,
        tools_requiring_approval: Vec<String>,
        timeout_secs: u64,
    ) -> Aegis {
        let tmp = tempfile::tempdir().expect("should create temp dir");
        let config = AegisConfig {
            keychain_service: "zeus-test".to_string(),
            sandbox_level: SandboxLevel::None,
            audit_path: tmp.path().join("audit.log"),
            permissions: vec!["*".to_string()],
            network_allowlist: vec!["*".to_string()],
            dangerous_patterns,
            tools_requiring_approval,
            approval_timeout_secs: timeout_secs,
            allowed_write_paths: Vec::new(),
            allow_system_paths: false,
            auto_allow: Vec::new(),
            mode: AegisMode::Prompt,
            classifier_api_key: None,
            classifier_api_url: None,
        };
        Aegis::new(config).await.expect("Aegis::new should succeed")
    }

    #[tokio::test]
    async fn test_aegis_queue_not_required() {
        let aegis = aegis_with_approvals(vec![], vec!["shell".to_string()], 5).await;

        // read_file not in approval list
        let outcome = aegis
            .queue_for_approval("read_file", &serde_json::json!({"path": "/tmp/test"}), None)
            .await;
        assert_eq!(outcome, approvals::ApprovalOutcome::NotRequired);
    }

    #[tokio::test]
    async fn test_aegis_needs_approval_check() {
        let aegis = aegis_with_approvals(
            vec!["sudo".to_string()],
            vec!["write_file".to_string()],
            300,
        )
        .await;

        // write_file always needs approval
        assert!(aegis.needs_approval("write_file", &serde_json::json!({})));

        // shell with sudo pattern needs approval
        assert!(aegis.needs_approval("shell", &serde_json::json!({"command": "sudo rm -rf /"})));

        // shell without pattern doesn't
        assert!(!aegis.needs_approval("shell", &serde_json::json!({"command": "ls -la"})));

        // read_file doesn't
        assert!(!aegis.needs_approval("read_file", &serde_json::json!({"path": "/tmp/test"})));
    }

    #[tokio::test]
    async fn test_aegis_queue_with_sink_approved() {
        use tokio::sync::mpsc;

        let mut aegis = aegis_with_approvals(vec![], vec!["shell".to_string()], 5).await;

        let (sink_tx, mut sink_rx) = mpsc::channel::<(
            approvals::QueuedApproval,
            tokio::sync::oneshot::Sender<approvals::ApprovalOutcome>,
        )>(16);
        aegis.set_approval_sink(sink_tx);

        // Auto-approve in background
        tokio::spawn(async move {
            if let Some((_queued, tx)) = sink_rx.recv().await {
                tx.send(approvals::ApprovalOutcome::Approved).ok();
            }
        });

        let outcome = aegis
            .queue_for_approval("shell", &serde_json::json!({"command": "echo test"}), None)
            .await;
        assert_eq!(outcome, approvals::ApprovalOutcome::Approved);
    }

    #[tokio::test]
    async fn test_aegis_queue_with_sink_denied() {
        use tokio::sync::mpsc;

        let mut aegis = aegis_with_approvals(vec![], vec!["shell".to_string()], 5).await;

        let (sink_tx, mut sink_rx) = mpsc::channel::<(
            approvals::QueuedApproval,
            tokio::sync::oneshot::Sender<approvals::ApprovalOutcome>,
        )>(16);
        aegis.set_approval_sink(sink_tx);

        tokio::spawn(async move {
            if let Some((_queued, tx)) = sink_rx.recv().await {
                tx.send(approvals::ApprovalOutcome::Denied {
                    reason: Some("not allowed".to_string()),
                })
                .ok();
            }
        });

        let outcome = aegis
            .queue_for_approval(
                "shell",
                &serde_json::json!({"command": "rm /important"}),
                None,
            )
            .await;
        assert!(matches!(
            outcome,
            approvals::ApprovalOutcome::Denied { reason: Some(r) } if r == "not allowed"
        ));
    }

    #[tokio::test]
    async fn test_aegis_approval_manager_accessor() {
        let aegis = aegis_with_approvals(
            vec!["DROP TABLE".to_string()],
            vec!["shell".to_string()],
            600,
        )
        .await;

        let mgr = aegis.approval_manager();
        assert_eq!(mgr.timeout_secs(), 600);
        assert_eq!(mgr.patterns(), &["DROP TABLE".to_string()]);
        assert_eq!(mgr.tools_requiring_approval(), &["shell".to_string()]);
    }

    #[tokio::test]
    async fn test_aegis_default_blocks_dangerous_commands() {
        let tmp = tempfile::tempdir().expect("should create temp dir");
        let config = AegisConfig {
            audit_path: tmp.path().join("audit.log"),
            ..Default::default()
        };
        let aegis = Aegis::new(config).await.expect("Aegis::new should succeed");

        // Default config flags dangerous patterns for approval
        assert!(aegis.needs_approval("shell", &serde_json::json!({"command": "rm -rf /"})));

        // Safe commands don't need approval
        assert!(!aegis.needs_approval("shell", &serde_json::json!({"command": "ls -la"})));
        assert!(!aegis.needs_approval("shell", &serde_json::json!({"command": "cargo build"})));
    }

    // =========================================================================
    // sandbox_command() integration tests
    // =========================================================================

    #[tokio::test]
    async fn test_aegis_sandbox_command_none() {
        let aegis = aegis_with_permissions(
            vec!["*".to_string()],
            SandboxLevel::None,
            vec!["*".to_string()],
        )
        .await;

        let cmd = aegis.sandbox_command("ls -la");
        assert_eq!(cmd, "ls -la", "None level should return command unchanged");
    }

    #[tokio::test]
    #[cfg(target_os = "macos")]
    async fn test_aegis_sandbox_command_standard() {
        let aegis = aegis_with_permissions(
            vec!["*".to_string()],
            SandboxLevel::Standard,
            vec!["*".to_string()],
        )
        .await;

        let cmd = aegis.sandbox_command("echo hello");
        assert!(
            cmd.starts_with("sandbox-exec -p '"),
            "Standard level should wrap with sandbox-exec"
        );
        assert!(cmd.contains("echo hello"));
        assert!(cmd.contains("(deny default)"));
    }

    #[tokio::test]
    #[cfg(target_os = "macos")]
    async fn test_aegis_sandbox_command_strict() {
        let aegis = aegis_with_permissions(
            vec!["*".to_string()],
            SandboxLevel::Strict,
            vec!["api.anthropic.com".to_string()],
        )
        .await;

        let cmd = aegis.sandbox_command("curl api.anthropic.com");
        assert!(cmd.contains("sandbox-exec"));
        assert!(cmd.contains("(deny network*)"));
    }

    #[tokio::test]
    #[cfg(target_os = "macos")]
    async fn test_aegis_sandbox_command_paranoid() {
        let aegis =
            aegis_with_permissions(vec!["*".to_string()], SandboxLevel::Paranoid, vec![]).await;

        let cmd = aegis.sandbox_command("cat file.txt");
        assert!(cmd.contains("sandbox-exec"));
        // Paranoid restricts exec to specific binaries
        assert!(cmd.contains("(allow process-exec (literal \"/bin/sh\"))"));
        assert!(!cmd.contains("(allow process-exec*)"));
    }

    #[tokio::test]
    #[cfg(not(target_os = "macos"))]
    async fn test_aegis_sandbox_command_passthrough_on_linux() {
        let aegis = aegis_with_permissions(
            vec!["*".to_string()],
            SandboxLevel::Standard,
            vec!["*".to_string()],
        )
        .await;

        // On non-macOS, sandbox_command returns the command unchanged
        // (Linux uses seccomp at process level, not per-command wrapping)
        let cmd = aegis.sandbox_command("echo hello");
        assert_eq!(cmd, "echo hello");
    }

    // =========================================================================
    // allow_system_paths tests — S13-1
    // =========================================================================

    #[tokio::test]
    async fn test_allow_system_paths_disabled_blocks_usr_local() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let config = AegisConfig {
            keychain_service: "zeus-test".to_string(),
            sandbox_level: SandboxLevel::Strict,
            audit_path: tmp.path().join("audit.log"),
            permissions: vec!["*".to_string()],
            network_allowlist: vec!["*".to_string()],
            allow_system_paths: false, // explicit: keep Aegis-layer restriction
            ..Default::default()
        };
        let aegis = Aegis::new(config).await.expect("Aegis::new");

        // Strict sandbox + no system paths → /usr/local/bin is blocked
        assert!(aegis.restricts_filesystem());
        assert!(!aegis.is_path_allowed("/usr/local/bin/zeus"));
        assert!(
            aegis
                .validate_shell_command("cp ./zeus /usr/local/bin/zeus")
                .is_err()
        );
    }

    #[tokio::test]
    async fn test_allow_system_paths_enabled_allows_usr_local() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let config = AegisConfig {
            keychain_service: "zeus-test".to_string(),
            sandbox_level: SandboxLevel::Standard,
            audit_path: tmp.path().join("audit.log"),
            permissions: vec!["*".to_string()],
            network_allowlist: vec!["*".to_string()],
            allow_system_paths: true, // removes Aegis-layer restriction
            ..Default::default()
        };
        let aegis = Aegis::new(config).await.expect("Aegis::new");

        // All 7 system prefixes should now be accessible
        assert!(aegis.is_path_allowed("/usr/local/bin/zeus"));
        assert!(aegis.is_path_allowed("/usr/local/sbin/myservice"));
        assert!(aegis.is_path_allowed("/usr/local/etc/nginx/nginx.conf"));
        assert!(aegis.is_path_allowed("/usr/local/lib/libfoo.so"));
        assert!(aegis.is_path_allowed("/usr/local/share/man/man1/zeus.1"));
        assert!(aegis.is_path_allowed("/etc/hosts"));

        // Validate shell commands referencing these paths also pass
        assert!(
            aegis
                .validate_shell_command("cp ./zeus /usr/local/bin/zeus")
                .is_ok()
        );
        assert!(
            aegis
                .validate_shell_command("install -m 755 zeus /usr/local/bin/zeus")
                .is_ok()
        );
    }

    #[test]
    fn test_system_paths_covers_expected_directories() {
        let paths = system_paths();
        assert!(paths.contains(&"/usr/local/"), "missing /usr/local/");
        assert!(paths.contains(&"/usr/local/bin/"), "missing /usr/local/bin/");
        assert!(
            paths.contains(&"/usr/local/sbin/"),
            "missing /usr/local/sbin/"
        );
        assert!(
            paths.contains(&"/usr/local/etc/"),
            "missing /usr/local/etc/"
        );
        assert!(
            paths.contains(&"/usr/local/lib/"),
            "missing /usr/local/lib/"
        );
        assert!(
            paths.contains(&"/usr/local/share/"),
            "missing /usr/local/share/"
        );
        assert!(paths.contains(&"/etc/"), "missing /etc/");
        assert_eq!(paths.len(), 7, "expected exactly 7 system paths");
    }
}
