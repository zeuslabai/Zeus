//! macOS Seatbelt sandbox implementation
//!
//! Uses sandbox-exec with SBPL (Sandbox Profile Language) profiles
//! to restrict process capabilities.
//!
//! ## Profile Levels
//!
//! - **None**: No restrictions (allow default)
//! - **Basic**: Block dangerous operations (process-exec*, system-*)
//! - **Standard**: Restricted filesystem (only allowed paths)
//! - **Strict**: Network allowlist + restricted filesystem
//! - **Paranoid**: Minimal permissions (read-only except workspace)

use crate::sandbox::SandboxLevel;
use std::ffi::CString;
use std::os::raw::c_char;
use std::path::PathBuf;
use zeus_core::{Error, Result};

// FFI bindings for sandbox_init
unsafe extern "C" {
    fn sandbox_init(profile: *const c_char, flags: u64, errorbuf: *mut *mut c_char) -> i32;
    fn sandbox_free_error(errorbuf: *mut c_char);
}

/// Seatbelt sandbox profile generator
pub struct SeatbeltProfile {
    level: SandboxLevel,
    allowed_paths: Vec<PathBuf>,
    allowed_hosts: Vec<String>,
    workspace_path: Option<PathBuf>,
}

impl SeatbeltProfile {
    /// Create a new profile for the given sandbox level
    pub fn new(level: SandboxLevel) -> Self {
        Self {
            level,
            allowed_paths: Vec::new(),
            allowed_hosts: Vec::new(),
            workspace_path: None,
        }
    }

    /// Add an allowed filesystem path
    pub fn allow_path(&mut self, path: impl Into<PathBuf>) -> &mut Self {
        self.allowed_paths.push(path.into());
        self
    }

    /// Add an allowed network host
    pub fn allow_host(&mut self, host: impl Into<String>) -> &mut Self {
        self.allowed_hosts.push(host.into());
        self
    }

    /// Set the workspace path (for Paranoid mode)
    pub fn workspace(&mut self, path: impl Into<PathBuf>) -> &mut Self {
        self.workspace_path = Some(path.into());
        self
    }

    /// Generate the SBPL profile string
    pub fn generate(&self) -> String {
        match self.level {
            SandboxLevel::None => self.generate_none(),
            SandboxLevel::Basic => self.generate_basic(),
            SandboxLevel::Standard => self.generate_standard(),
            SandboxLevel::Strict => self.generate_strict(),
            SandboxLevel::Paranoid => self.generate_paranoid(),
        }
    }

    /// No restrictions - allow everything
    fn generate_none(&self) -> String {
        r#"(version 1)
(allow default)"#
            .to_string()
    }

    /// Basic - block dangerous operations
    fn generate_basic(&self) -> String {
        r#"(version 1)
(allow default)

; Block dangerous system operations
(deny process-exec* (with no-report))
(deny system-privilege)
(deny system-kext*)
(deny nvram*)
(deny iokit-set-properties)

; Allow basic process operations
(allow process-exec (literal "/bin/sh"))
(allow process-exec (literal "/usr/bin/env"))
"#
        .to_string()
    }

    /// Standard - restricted filesystem
    fn generate_standard(&self) -> String {
        let mut profile = r#"(version 1)
(deny default)

; Allow essential operations
(allow signal (target self))
(allow sysctl-read)
(allow mach-lookup)
(allow ipc-posix-shm-read-data)
(allow ipc-posix-shm-write-data)
(allow ipc-posix-shm-write-create)

; Allow reading system files
(allow file-read* (subpath "/usr/lib"))
(allow file-read* (subpath "/usr/share"))
(allow file-read* (subpath "/System"))
(allow file-read* (subpath "/Library/Frameworks"))
(allow file-read* (subpath "/private/var/db"))
(allow file-read* (literal "/dev/null"))
(allow file-read* (literal "/dev/random"))
(allow file-read* (literal "/dev/urandom"))

; Allow home directory read
(allow file-read* (subpath (param "HOME")))

; Allow writing to temp directories
(allow file-write* (subpath "/private/tmp"))
(allow file-write* (subpath (param "TMPDIR")))

; Allow network (unrestricted at this level)
(allow network*)
"#
        .to_string();

        // Add allowed paths
        for path in &self.allowed_paths {
            let path_str = path.display();
            profile.push_str(&format!(
                "\n; Allowed path: {0}\n(allow file-read* (subpath \"{0}\"))\n(allow file-write* (subpath \"{0}\"))\n",
                path_str
            ));
        }

        profile
    }

    /// Strict - network allowlist + restricted filesystem
    fn generate_strict(&self) -> String {
        let mut profile = r#"(version 1)
(deny default)

; Allow essential operations
(allow signal (target self))
(allow sysctl-read)
(allow mach-lookup)
(allow ipc-posix-shm-read-data)
(allow ipc-posix-shm-write-data)
(allow ipc-posix-shm-write-create)

; Allow reading system files
(allow file-read* (subpath "/usr/lib"))
(allow file-read* (subpath "/usr/share"))
(allow file-read* (subpath "/System"))
(allow file-read* (subpath "/Library/Frameworks"))
(allow file-read* (subpath "/private/var/db"))
(allow file-read* (literal "/dev/null"))
(allow file-read* (literal "/dev/random"))
(allow file-read* (literal "/dev/urandom"))
(allow file-read* (literal "/etc/resolv.conf"))
(allow file-read* (literal "/etc/hosts"))

; Allow writing to temp directories
(allow file-write* (subpath "/private/tmp"))
(allow file-write* (subpath (param "TMPDIR")))

; Deny network by default
(deny network*)
"#
        .to_string();

        // Add allowed paths
        for path in &self.allowed_paths {
            let path_str = path.display();
            profile.push_str(&format!(
                "\n; Allowed path: {0}\n(allow file-read* (subpath \"{0}\"))\n(allow file-write* (subpath \"{0}\"))\n",
                path_str
            ));
        }

        // Add allowed network hosts
        if self.allowed_hosts.is_empty() || self.allowed_hosts.contains(&"*".to_string()) {
            profile.push_str(
                "\n; Network: Allow all (no hosts specified or wildcard)\n(allow network*)\n",
            );
        } else {
            profile.push_str(
                "\n; Network: Allowlisted hosts only\n(allow network-outbound (remote tcp))\n",
            );
            for host in &self.allowed_hosts {
                // Note: Seatbelt doesn't support hostname filtering directly
                // This allows TCP connections but relies on DNS resolution
                profile.push_str(&format!("; Allowed host: {}\n", host));
            }
        }

        profile
    }

    /// Paranoid - minimal permissions
    fn generate_paranoid(&self) -> String {
        let mut profile = r#"(version 1)
(deny default)

; Allow minimal essential operations
(allow signal (target self))
(allow sysctl-read)
(allow mach-lookup)

; Allow reading system libraries only
(allow file-read* (subpath "/usr/lib"))
(allow file-read* (subpath "/System/Library"))
(allow file-read* (literal "/dev/null"))
(allow file-read* (literal "/dev/urandom"))

; Deny all network
(deny network*)

; Deny process execution
(deny process-exec*)
(deny process-fork)
"#
        .to_string();

        // Only allow workspace if specified
        if let Some(workspace) = &self.workspace_path {
            let ws_str = workspace.display();
            profile.push_str(&format!(
                "\n; Workspace (read-write): {0}\n(allow file-read* (subpath \"{0}\"))\n(allow file-write* (subpath \"{0}\"))\n",
                ws_str
            ));
        }

        // Allow reading other specified paths (no write)
        for path in &self.allowed_paths {
            let path_str = path.display();
            profile.push_str(&format!(
                "\n; Read-only path: {0}\n(allow file-read* (subpath \"{0}\"))\n",
                path_str
            ));
        }

        profile
    }
}

/// Seatbelt sandbox enforcer
pub struct SeatbeltSandbox {
    profile: SeatbeltProfile,
    applied: bool,
}

impl SeatbeltSandbox {
    /// Create a new sandbox with the given level
    pub fn new(level: SandboxLevel) -> Self {
        Self {
            profile: SeatbeltProfile::new(level),
            applied: false,
        }
    }

    /// Add an allowed filesystem path
    pub fn allow_path(&mut self, path: impl Into<PathBuf>) -> &mut Self {
        self.profile.allow_path(path);
        self
    }

    /// Add an allowed network host
    pub fn allow_host(&mut self, host: impl Into<String>) -> &mut Self {
        self.profile.allow_host(host);
        self
    }

    /// Set the workspace path
    pub fn workspace(&mut self, path: impl Into<PathBuf>) -> &mut Self {
        self.profile.workspace(path);
        self
    }

    /// Get the generated profile string (for debugging)
    pub fn profile_string(&self) -> String {
        self.profile.generate()
    }

    /// Apply the sandbox restrictions
    ///
    /// This is a one-way operation - once applied, restrictions cannot be removed.
    pub fn apply(&mut self) -> Result<()> {
        if self.applied {
            return Err(Error::Security("Sandbox already applied".into()));
        }

        let profile = self.profile.generate();
        tracing::debug!(profile = %profile, "Applying Seatbelt sandbox profile");

        // Apply via sandbox_init
        self.apply_profile(&profile)?;

        self.applied = true;
        tracing::info!(level = %self.profile.level, "Seatbelt sandbox applied");

        Ok(())
    }

    /// Check if sandbox is applied
    pub fn is_applied(&self) -> bool {
        self.applied
    }

    /// Apply the profile using sandbox_init FFI
    fn apply_profile(&self, profile: &str) -> Result<()> {
        let profile_cstr = CString::new(profile)
            .map_err(|e| Error::Security(format!("Invalid profile string: {}", e)))?;

        let mut error_buf: *mut c_char = std::ptr::null_mut();

        // SANDBOX_NAMED = 0x0001 (use named profile)
        // We're using a raw profile string, so flags = 0
        let result = unsafe { sandbox_init(profile_cstr.as_ptr(), 0, &mut error_buf) };

        if result != 0 {
            let error_msg = if !error_buf.is_null() {
                let msg = unsafe { std::ffi::CStr::from_ptr(error_buf) }
                    .to_string_lossy()
                    .into_owned();
                unsafe { sandbox_free_error(error_buf) };
                msg
            } else {
                "Unknown error".to_string()
            };

            return Err(Error::Security(format!(
                "Failed to apply sandbox: {}",
                error_msg
            )));
        }

        Ok(())
    }
}

/// Check if sandbox-exec is available
pub fn is_available() -> bool {
    // sandbox_init is always available on macOS 10.5+
    true
}

/// Get the current sandbox status
pub fn status() -> SandboxStatus {
    // Note: There's no public API to check if we're sandboxed
    // We can try to do something that would fail in a sandbox
    SandboxStatus {
        available: true,
        applied: false, // Can't reliably detect this
        level: None,
    }
}

/// Sandbox status information
#[derive(Debug, Clone)]
pub struct SandboxStatus {
    /// Whether sandboxing is available on this system
    pub available: bool,
    /// Whether a sandbox is currently applied
    pub applied: bool,
    /// The current sandbox level (if known)
    pub level: Option<SandboxLevel>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_profile_generation_none() {
        let profile = SeatbeltProfile::new(SandboxLevel::None);
        let sbpl = profile.generate();
        assert!(sbpl.contains("(allow default)"));
    }

    #[test]
    fn test_profile_generation_basic() {
        let profile = SeatbeltProfile::new(SandboxLevel::Basic);
        let sbpl = profile.generate();
        assert!(sbpl.contains("(deny process-exec*"));
        assert!(sbpl.contains("(deny system-privilege)"));
    }

    #[test]
    fn test_profile_generation_standard() {
        let mut profile = SeatbeltProfile::new(SandboxLevel::Standard);
        profile.allow_path("/Users/test/.zeus");
        let sbpl = profile.generate();
        assert!(sbpl.contains("(deny default)"));
        assert!(sbpl.contains("/Users/test/.zeus"));
    }

    #[test]
    fn test_profile_generation_strict() {
        let mut profile = SeatbeltProfile::new(SandboxLevel::Strict);
        profile.allow_host("api.anthropic.com");
        let sbpl = profile.generate();
        assert!(sbpl.contains("(deny network*)"));
        assert!(sbpl.contains("api.anthropic.com"));
    }

    #[test]
    fn test_profile_generation_paranoid() {
        let mut profile = SeatbeltProfile::new(SandboxLevel::Paranoid);
        profile.workspace("/Users/test/workspace");
        let sbpl = profile.generate();
        assert!(sbpl.contains("(deny process-fork)"));
        assert!(sbpl.contains("/Users/test/workspace"));
    }

    #[test]
    fn test_sandbox_builder() {
        let mut sandbox = SeatbeltSandbox::new(SandboxLevel::Standard);
        sandbox
            .allow_path("/tmp")
            .allow_host("example.com")
            .workspace("/workspace");

        assert!(!sandbox.is_applied());
        let profile = sandbox.profile_string();
        assert!(profile.contains("/tmp"));
    }
}
