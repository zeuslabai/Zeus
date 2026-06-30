//! Multi-level sandboxing
//!
//! Provides a unified interface for platform-specific sandboxing:
//! - **macOS**: Seatbelt profiles via sandbox-exec
//! - **Linux**: seccomp-bpf syscall filtering
//!
//! ## Usage
//!
//! ```no_run
//! use zeus_aegis::sandbox::{Sandbox, SandboxLevel};
//!
//! let mut sandbox = Sandbox::new(SandboxLevel::Standard);
//! sandbox.allow_path("/home/user/.zeus");
//! sandbox.allow_host("api.anthropic.com");
//! sandbox.apply().expect("Failed to apply sandbox");
//! ```

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use zeus_core::Result;

/// Sandbox security levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SandboxLevel {
    /// No sandboxing (development only)
    None = 0,
    /// Basic sandboxing - block dangerous operations
    Basic = 1,
    /// Standard sandboxing - limited filesystem
    #[default]
    Standard = 2,
    /// Strict sandboxing - network allowlist
    Strict = 3,
    /// Paranoid sandboxing - minimal permissions
    Paranoid = 4,
}

impl SandboxLevel {
    /// Check if filesystem access is restricted
    pub fn restricts_filesystem(&self) -> bool {
        *self >= SandboxLevel::Strict
    }

    /// Check if network access is restricted
    pub fn restricts_network(&self) -> bool {
        *self >= SandboxLevel::Strict
    }

    /// Check if this level requires allowlisting
    pub fn requires_allowlist(&self) -> bool {
        *self >= SandboxLevel::Strict
    }

    /// Get a description of this sandbox level
    pub fn description(&self) -> &'static str {
        match self {
            Self::None => "No restrictions (development only)",
            Self::Basic => "Block dangerous operations (ptrace, kexec, etc.)",
            Self::Standard => "Restricted filesystem access",
            Self::Strict => "Network allowlist + restricted filesystem",
            Self::Paranoid => "Minimal permissions only",
        }
    }
}

impl std::str::FromStr for SandboxLevel {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_str() {
            "none" => Self::None,
            "basic" => Self::Basic,
            "standard" => Self::Standard,
            "strict" => Self::Strict,
            "paranoid" => Self::Paranoid,
            _ => Self::None,
        })
    }
}

impl std::fmt::Display for SandboxLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => write!(f, "none"),
            Self::Basic => write!(f, "basic"),
            Self::Standard => write!(f, "standard"),
            Self::Strict => write!(f, "strict"),
            Self::Paranoid => write!(f, "paranoid"),
        }
    }
}

/// Sandbox enforcement
///
/// Provides a unified interface for platform-specific sandboxing.
/// The sandbox can be configured with allowed paths and hosts,
/// then applied once to restrict the process.
pub struct Sandbox {
    level: SandboxLevel,
    filesystem_paths: Vec<PathBuf>,
    network_hosts: Vec<String>,
    workspace_path: Option<PathBuf>,
    applied: bool,
}

impl Sandbox {
    /// Create a new sandbox with the specified level
    pub fn new(level: SandboxLevel) -> Self {
        let mut filesystem_paths = Vec::new();
        // Allow Telegram file download directory
        filesystem_paths.push(PathBuf::from("/tmp/telegram_files"));

        Self {
            level,
            filesystem_paths,
            network_hosts: Vec::new(),
            workspace_path: None,
            applied: false,
        }
    }

    /// Get the current sandbox level
    pub fn level(&self) -> SandboxLevel {
        self.level
    }

    /// Check if the sandbox has been applied
    pub fn is_applied(&self) -> bool {
        self.applied
    }

    /// Allow access to a filesystem path
    pub fn allow_path(&mut self, path: impl Into<String>) {
        self.filesystem_paths.push(PathBuf::from(path.into()));
    }

    /// Allow access to a network host
    pub fn allow_host(&mut self, host: impl Into<String>) {
        self.network_hosts.push(host.into());
    }

    /// Set the workspace path (primary read-write area)
    pub fn set_workspace(&mut self, path: impl Into<PathBuf>) {
        self.workspace_path = Some(path.into());
    }

    /// Get allowed filesystem paths
    pub fn allowed_paths(&self) -> &[PathBuf] {
        &self.filesystem_paths
    }

    /// Get allowed network hosts
    pub fn allowed_hosts(&self) -> &[String] {
        &self.network_hosts
    }

    /// Check if a filesystem path is allowed
    pub fn is_path_allowed(&self, path: &str) -> bool {
        if !self.level.restricts_filesystem() {
            return true;
        }

        // Standard Unix devices are always safe — never block /dev/null, /dev/zero, etc.
        if path.starts_with("/dev/null") || path.starts_with("/dev/zero") || path.starts_with("/dev/stdin") || path.starts_with("/dev/stdout") || path.starts_with("/dev/stderr") {
            return true;
        }

        self.filesystem_paths.iter().any(|allowed| {
            let allowed_str = allowed.to_string_lossy();
            path.starts_with(allowed_str.as_ref()) || allowed_str == "*"
        })
    }

    /// Check if a network host is allowed
    pub fn is_host_allowed(&self, host: &str) -> bool {
        if !self.level.restricts_network() {
            return true;
        }

        self.network_hosts
            .iter()
            .any(|allowed| host == allowed || allowed == "*")
    }

    /// Apply sandbox restrictions (macOS)
    ///
    /// Uses Seatbelt profiles via sandbox-exec.
    #[cfg(target_os = "macos")]
    pub fn apply(&mut self) -> Result<()> {
        use crate::macos::seatbelt::SeatbeltSandbox;

        if self.applied {
            return Err(zeus_core::Error::Security("Sandbox already applied".into()));
        }

        let mut seatbelt = SeatbeltSandbox::new(self.level);

        // Add allowed paths
        for path in &self.filesystem_paths {
            seatbelt.allow_path(path);
        }

        // Add allowed hosts
        for host in &self.network_hosts {
            seatbelt.allow_host(host);
        }

        // Set workspace
        if let Some(workspace) = &self.workspace_path {
            seatbelt.workspace(workspace);
        }

        // Apply the sandbox
        seatbelt.apply()?;

        self.applied = true;
        tracing::info!(
            level = %self.level,
            paths = self.filesystem_paths.len(),
            hosts = self.network_hosts.len(),
            "Sandbox applied (macOS Seatbelt)"
        );

        Ok(())
    }

    /// Apply sandbox restrictions (Linux)
    ///
    /// Uses seccomp-bpf for syscall filtering.
    #[cfg(target_os = "linux")]
    pub fn apply(&mut self) -> Result<()> {
        use crate::linux::seccomp::SeccompSandbox;

        if self.applied {
            return Err(zeus_core::Error::Security("Sandbox already applied".into()));
        }

        let mut seccomp = SeccompSandbox::new(self.level);

        // Note: seccomp doesn't filter by path/host directly
        // It only filters syscalls. Path/host filtering is done
        // at the application level using is_path_allowed/is_host_allowed.

        // Apply the seccomp filter
        seccomp.apply()?;

        self.applied = true;
        tracing::info!(
            level = %self.level,
            "Sandbox applied (Linux seccomp)"
        );

        Ok(())
    }

    /// Wrap a shell command with sandbox-exec for macOS Seatbelt enforcement.
    ///
    /// Generates a Seatbelt profile appropriate for the current sandbox level
    /// and wraps the command so it runs inside the sandbox. The profile restricts
    /// filesystem access to the workspace directory and temp files.
    ///
    /// - At `SandboxLevel::None`, returns the command unchanged.
    /// - On non-macOS platforms, returns the command unchanged.
    /// - At `Basic` and above, wraps with `sandbox-exec -p '<profile>' /bin/sh -c '<command>'`.
    pub fn sandbox_command(&self, command: &str) -> String {
        if self.level == SandboxLevel::None {
            return command.to_string();
        }

        // Seatbelt sandboxing is macOS-only; on other platforms return the command unchanged.
        // Linux uses seccomp at the process level (see apply()), not per-command wrapping.
        #[cfg(target_os = "macos")]
        {
            let profile = self.generate_command_profile();

            // Escape single quotes in both profile and command for shell embedding
            let escaped_profile = profile.replace('\'', "'\\''");
            let escaped_command = command.replace('\'', "'\\''");

            format!(
                "sandbox-exec -p '{}' /bin/sh -c '{}'",
                escaped_profile, escaped_command
            )
        }

        #[cfg(not(target_os = "macos"))]
        {
            command.to_string()
        }
    }

    /// Generate an SBPL profile suitable for wrapping external shell commands.
    ///
    /// This differs from the process-level profiles in `SeatbeltProfile` because
    /// it must allow process execution (fork + exec) for `/bin/sh` and common
    /// system binaries, while still restricting filesystem and network access
    /// according to the sandbox level.
    #[cfg(target_os = "macos")]
    fn generate_command_profile(&self) -> String {
        match self.level {
            SandboxLevel::None => "(version 1)\n(allow default)".to_string(),
            SandboxLevel::Basic => self.generate_command_profile_basic(),
            SandboxLevel::Standard => self.generate_command_profile_standard(),
            SandboxLevel::Strict => self.generate_command_profile_strict(),
            SandboxLevel::Paranoid => self.generate_command_profile_paranoid(),
        }
    }

    /// Basic command profile — block dangerous ops but allow most execution
    #[cfg(target_os = "macos")]
    fn generate_command_profile_basic(&self) -> String {
        r#"(version 1)
(allow default)

; Block dangerous system operations
(deny system-privilege)
(deny system-kext*)
(deny nvram*)
(deny iokit-set-properties)"#
            .to_string()
    }

    /// Standard command profile — restrict filesystem to workspace + temp + system
    #[cfg(target_os = "macos")]
    fn generate_command_profile_standard(&self) -> String {
        let mut profile = String::from(
            r#"(version 1)
(deny default)

; Process execution (required for shell commands)
(allow process-fork)
(allow process-exec*)

; Essential IPC and system access
(allow signal (target self))
(allow sysctl-read)
(allow mach-lookup)
(allow mach-register)
(allow ipc-posix-shm-read-data)
(allow ipc-posix-shm-write-data)
(allow ipc-posix-shm-write-create)
(allow iokit-open)

; System binaries and libraries (read-only)
(allow file-read* (subpath "/usr"))
(allow file-read* (subpath "/bin"))
(allow file-read* (subpath "/sbin"))
(allow file-read* (subpath "/System"))
(allow file-read* (subpath "/Library/Frameworks"))
(allow file-read* (subpath "/private/var/db"))
(allow file-read* (subpath "/private/etc"))
(allow file-read* (literal "/dev/null"))
(allow file-read* (literal "/dev/random"))
(allow file-read* (literal "/dev/urandom"))
(allow file-read* (literal "/dev/tty"))
(allow file-read* (literal "/dev/fd"))
(allow file-write* (literal "/dev/null"))

; Temp directories (read-write)
(allow file-read* (subpath "/private/tmp"))
(allow file-write* (subpath "/private/tmp"))
(allow file-read* (subpath "/tmp"))
(allow file-write* (subpath "/tmp"))
(allow file-read* (subpath "/var/folders"))
(allow file-write* (subpath "/var/folders"))

; Allow network (unrestricted at Standard level)
(allow network*)
"#,
        );

        // Workspace path (read-write)
        if let Some(workspace) = &self.workspace_path {
            let ws = workspace.display();
            profile.push_str(&format!(
                "\n; Workspace (read-write)\n(allow file-read* (subpath \"{ws}\"))\n(allow file-write* (subpath \"{ws}\"))\n"
            ));
        }

        // Additional allowed paths (read-write)
        for path in &self.filesystem_paths {
            let p = path.display();
            profile.push_str(&format!(
                "\n; Allowed path\n(allow file-read* (subpath \"{p}\"))\n(allow file-write* (subpath \"{p}\"))\n"
            ));
        }

        profile
    }

    /// Strict command profile — filesystem restricted + network allowlisted
    #[cfg(target_os = "macos")]
    fn generate_command_profile_strict(&self) -> String {
        let mut profile = String::from(
            r#"(version 1)
(deny default)

; Process execution (required for shell commands)
(allow process-fork)
(allow process-exec*)

; Essential IPC and system access
(allow signal (target self))
(allow sysctl-read)
(allow mach-lookup)
(allow mach-register)
(allow ipc-posix-shm-read-data)
(allow ipc-posix-shm-write-data)
(allow ipc-posix-shm-write-create)
(allow iokit-open)

; System binaries and libraries (read-only)
(allow file-read* (subpath "/usr"))
(allow file-read* (subpath "/bin"))
(allow file-read* (subpath "/sbin"))
(allow file-read* (subpath "/System"))
(allow file-read* (subpath "/Library/Frameworks"))
(allow file-read* (subpath "/private/var/db"))
(allow file-read* (literal "/private/etc/resolv.conf"))
(allow file-read* (literal "/private/etc/hosts"))
(allow file-read* (literal "/dev/null"))
(allow file-read* (literal "/dev/random"))
(allow file-read* (literal "/dev/urandom"))
(allow file-read* (literal "/dev/tty"))
(allow file-write* (literal "/dev/null"))

; Temp directories (read-write)
(allow file-read* (subpath "/private/tmp"))
(allow file-write* (subpath "/private/tmp"))
(allow file-read* (subpath "/tmp"))
(allow file-write* (subpath "/tmp"))
(allow file-read* (subpath "/var/folders"))
(allow file-write* (subpath "/var/folders"))

; Deny network by default
(deny network*)
"#,
        );

        // Workspace path (read-write)
        if let Some(workspace) = &self.workspace_path {
            let ws = workspace.display();
            profile.push_str(&format!(
                "\n; Workspace (read-write)\n(allow file-read* (subpath \"{ws}\"))\n(allow file-write* (subpath \"{ws}\"))\n"
            ));
        }

        // Additional allowed paths
        for path in &self.filesystem_paths {
            let p = path.display();
            profile.push_str(&format!(
                "\n; Allowed path\n(allow file-read* (subpath \"{p}\"))\n(allow file-write* (subpath \"{p}\"))\n"
            ));
        }

        // Network allowlist
        if self.network_hosts.is_empty() || self.network_hosts.contains(&"*".to_string()) {
            profile.push_str("\n; Network: all allowed\n(allow network*)\n");
        } else {
            // Allow DNS resolution and outbound TCP (Seatbelt can't filter by hostname)
            profile.push_str("\n; Network: allowlisted hosts (enforced at application layer)\n");
            profile.push_str("(allow network-outbound (remote tcp))\n");
            profile.push_str(
                "(allow network-outbound (remote udp (to name \"localhost\" port 53)))\n",
            );
            profile.push_str("(allow network-inbound (local tcp))\n");
            for host in &self.network_hosts {
                profile.push_str(&format!("; Allowed host: {}\n", host));
            }
        }

        profile
    }

    /// Paranoid command profile — minimal permissions, no network, no arbitrary exec
    #[cfg(target_os = "macos")]
    fn generate_command_profile_paranoid(&self) -> String {
        let mut profile = String::from(
            r#"(version 1)
(deny default)

; Minimal process execution — only /bin/sh and /usr/bin/env
(allow process-fork)
(allow process-exec (literal "/bin/sh"))
(allow process-exec (literal "/usr/bin/env"))
(allow process-exec (literal "/bin/cat"))
(allow process-exec (literal "/bin/ls"))
(allow process-exec (literal "/usr/bin/head"))
(allow process-exec (literal "/usr/bin/tail"))

; Essential IPC
(allow signal (target self))
(allow sysctl-read)
(allow mach-lookup)

; System libraries (read-only, minimal)
(allow file-read* (subpath "/usr/lib"))
(allow file-read* (subpath "/System/Library"))
(allow file-read* (literal "/dev/null"))
(allow file-read* (literal "/dev/urandom"))
(allow file-write* (literal "/dev/null"))

; Temp (read-write)
(allow file-read* (subpath "/private/tmp"))
(allow file-write* (subpath "/private/tmp"))
(allow file-read* (subpath "/var/folders"))
(allow file-write* (subpath "/var/folders"))

; Deny all network
(deny network*)
"#,
        );

        // Workspace path (read-write)
        if let Some(workspace) = &self.workspace_path {
            let ws = workspace.display();
            profile.push_str(&format!(
                "\n; Workspace (read-write)\n(allow file-read* (subpath \"{ws}\"))\n(allow file-write* (subpath \"{ws}\"))\n"
            ));
        }

        // Additional paths (read-only in paranoid mode)
        for path in &self.filesystem_paths {
            let p = path.display();
            profile.push_str(&format!(
                "\n; Read-only path\n(allow file-read* (subpath \"{p}\"))\n"
            ));
        }

        profile
    }

    /// Fallback for other platforms
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    pub fn apply(&mut self) -> Result<()> {
        if self.applied {
            return Err(zeus_core::Error::Security("Sandbox already applied".into()));
        }

        tracing::warn!(
            level = %self.level,
            "Sandbox enforcement not available on this platform"
        );

        self.applied = true;
        Ok(())
    }
}

/// Check if sandbox enforcement is available on this platform
pub fn is_available() -> bool {
    #[cfg(target_os = "macos")]
    {
        crate::macos::seatbelt::is_available()
    }
    #[cfg(target_os = "linux")]
    {
        crate::linux::seccomp::is_available()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        false
    }
}

/// Get sandbox status information
#[derive(Debug, Clone)]
pub struct SandboxStatus {
    /// Whether sandboxing is available
    pub available: bool,
    /// Current platform
    pub platform: &'static str,
    /// Enforcement mechanism
    pub mechanism: &'static str,
}

/// Get current sandbox status
pub fn status() -> SandboxStatus {
    #[cfg(target_os = "macos")]
    {
        SandboxStatus {
            available: true,
            platform: "macOS",
            mechanism: "Seatbelt (sandbox-exec)",
        }
    }
    #[cfg(target_os = "linux")]
    {
        SandboxStatus {
            available: crate::linux::seccomp::is_available(),
            platform: "Linux",
            mechanism: "seccomp-bpf",
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        SandboxStatus {
            available: false,
            platform: "unknown",
            mechanism: "none",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sandbox_levels() {
        assert!(SandboxLevel::Paranoid > SandboxLevel::Strict);
        assert!(SandboxLevel::Strict > SandboxLevel::Standard);
        assert!(SandboxLevel::Standard > SandboxLevel::Basic);
        assert!(SandboxLevel::Basic > SandboxLevel::None);
    }

    #[test]
    fn test_path_allowlist() {
        let mut sandbox = Sandbox::new(SandboxLevel::Strict);
        sandbox.allow_path("/home/user/.zeus");

        assert!(sandbox.is_path_allowed("/home/user/.zeus/config.toml"));
        assert!(!sandbox.is_path_allowed("/etc/passwd"));
    }

    #[test]
    fn test_host_allowlist() {
        let mut sandbox = Sandbox::new(SandboxLevel::Strict);
        sandbox.allow_host("api.anthropic.com");

        assert!(sandbox.is_host_allowed("api.anthropic.com"));
        assert!(!sandbox.is_host_allowed("evil.com"));
    }

    // =========================================================================
    // sandbox_command() tests
    // =========================================================================

    #[test]
    fn test_sandbox_command_none_returns_unchanged() {
        let sandbox = Sandbox::new(SandboxLevel::None);
        let cmd = sandbox.sandbox_command("ls -la /tmp");
        assert_eq!(cmd, "ls -la /tmp");
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn test_sandbox_command_basic_wraps_with_sandbox_exec() {
        let sandbox = Sandbox::new(SandboxLevel::Basic);
        let cmd = sandbox.sandbox_command("echo hello");
        assert!(cmd.starts_with("sandbox-exec -p '"));
        assert!(cmd.contains("/bin/sh -c '"));
        assert!(cmd.contains("echo hello"));
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn test_sandbox_command_standard_contains_deny_default() {
        let sandbox = Sandbox::new(SandboxLevel::Standard);
        let cmd = sandbox.sandbox_command("cat /etc/passwd");
        assert!(cmd.contains("(deny default)"));
        assert!(cmd.contains("(allow process-fork)"));
        assert!(cmd.contains("(allow process-exec*)"));
        assert!(cmd.contains("cat /etc/passwd"));
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn test_sandbox_command_standard_includes_workspace() {
        let mut sandbox = Sandbox::new(SandboxLevel::Standard);
        sandbox.set_workspace("/Users/test/.zeus");
        let cmd = sandbox.sandbox_command("ls");
        assert!(cmd.contains("/Users/test/.zeus"));
        assert!(cmd.contains("Workspace"));
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn test_sandbox_command_standard_includes_allowed_paths() {
        let mut sandbox = Sandbox::new(SandboxLevel::Standard);
        sandbox.allow_path("/home/user/projects");
        let cmd = sandbox.sandbox_command("git status");
        assert!(cmd.contains("/home/user/projects"));
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn test_sandbox_command_standard_allows_temp() {
        let sandbox = Sandbox::new(SandboxLevel::Standard);
        let cmd = sandbox.sandbox_command("echo test");
        assert!(cmd.contains("/private/tmp"));
        assert!(cmd.contains("/var/folders"));
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn test_sandbox_command_strict_denies_network() {
        let sandbox = Sandbox::new(SandboxLevel::Strict);
        let cmd = sandbox.sandbox_command("curl example.com");
        assert!(cmd.contains("(deny network*)"));
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn test_sandbox_command_strict_with_allowed_hosts() {
        let mut sandbox = Sandbox::new(SandboxLevel::Strict);
        sandbox.allow_host("api.anthropic.com");
        let cmd = sandbox.sandbox_command("curl api.anthropic.com");
        assert!(cmd.contains("api.anthropic.com"));
        assert!(cmd.contains("network-outbound"));
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn test_sandbox_command_strict_wildcard_host_allows_all() {
        let mut sandbox = Sandbox::new(SandboxLevel::Strict);
        sandbox.allow_host("*");
        let cmd = sandbox.sandbox_command("curl example.com");
        assert!(cmd.contains("(allow network*)"));
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn test_sandbox_command_paranoid_restricts_exec() {
        let sandbox = Sandbox::new(SandboxLevel::Paranoid);
        let cmd = sandbox.sandbox_command("ls");
        // Paranoid only allows specific binaries
        assert!(cmd.contains("(allow process-exec (literal \"/bin/sh\"))"));
        assert!(cmd.contains("(deny network*)"));
        // Should NOT contain (allow process-exec*)
        assert!(!cmd.contains("(allow process-exec*)"));
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn test_sandbox_command_paranoid_with_workspace() {
        let mut sandbox = Sandbox::new(SandboxLevel::Paranoid);
        sandbox.set_workspace("/Users/test/workspace");
        let cmd = sandbox.sandbox_command("cat file.txt");
        assert!(cmd.contains("/Users/test/workspace"));
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn test_sandbox_command_escapes_single_quotes() {
        let sandbox = Sandbox::new(SandboxLevel::Standard);
        let cmd = sandbox.sandbox_command("echo 'hello world'");
        // The command should be escaped for shell embedding
        assert!(cmd.contains("echo"));
        assert!(cmd.contains("hello world"));
        // Should not have unmatched quotes that break the shell
        let quote_count = cmd.matches('\'').count();
        assert!(
            quote_count % 2 == 0 || cmd.contains("'\\''"),
            "quotes should be balanced"
        );
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn test_sandbox_command_profile_allows_system_binaries() {
        let sandbox = Sandbox::new(SandboxLevel::Standard);
        let cmd = sandbox.sandbox_command("ls");
        // Standard profile must allow reading system paths
        assert!(cmd.contains("(allow file-read* (subpath \"/usr\"))"));
        assert!(cmd.contains("(allow file-read* (subpath \"/bin\"))"));
        assert!(cmd.contains("(allow file-read* (subpath \"/sbin\"))"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_generate_command_profile_none() {
        let sandbox = Sandbox::new(SandboxLevel::None);
        let profile = sandbox.generate_command_profile();
        assert!(profile.contains("(allow default)"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_generate_command_profile_basic() {
        let sandbox = Sandbox::new(SandboxLevel::Basic);
        let profile = sandbox.generate_command_profile();
        assert!(profile.contains("(allow default)"));
        assert!(profile.contains("(deny system-privilege)"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_generate_command_profile_standard_structure() {
        let mut sandbox = Sandbox::new(SandboxLevel::Standard);
        sandbox.set_workspace("/ws");
        sandbox.allow_path("/extra");
        let profile = sandbox.generate_command_profile();

        // Must have deny default
        assert!(profile.contains("(deny default)"));
        // Must allow process execution
        assert!(profile.contains("(allow process-fork)"));
        assert!(profile.contains("(allow process-exec*)"));
        // Must allow workspace
        assert!(profile.contains("/ws"));
        // Must allow extra path
        assert!(profile.contains("/extra"));
        // Must allow network at standard
        assert!(profile.contains("(allow network*)"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_generate_command_profile_strict_structure() {
        let mut sandbox = Sandbox::new(SandboxLevel::Strict);
        sandbox.set_workspace("/ws");
        sandbox.allow_host("example.com");
        let profile = sandbox.generate_command_profile();

        assert!(profile.contains("(deny default)"));
        assert!(profile.contains("(deny network*)"));
        assert!(profile.contains("example.com"));
        assert!(profile.contains("/ws"));
    }
}
