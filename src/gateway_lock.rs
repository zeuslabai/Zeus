//! Gateway PID Lock — prevents multiple gateway instances.
//!
//! Writes PID to `~/.zeus/gateway.pid` on acquire; removes on Drop (even on panic).
//! Also verifies the target port is not already bound.

use anyhow::Result;
use tracing::{info, warn};
use crate::zeus_paths;

/// Prevents multiple gateway instances from running simultaneously.
pub struct GatewayLock {
    pid_path: std::path::PathBuf,
}

impl GatewayLock {
    /// Acquire the gateway lock. Fails if another instance is already running
    /// or if the target port is in use.
    pub fn acquire(port: u16) -> Result<Self> {
        let pid_path = zeus_paths::zeus_pid_path();

        // Check existing PID file
        if let Ok(pid_str) = std::fs::read_to_string(&pid_path)
            && let Ok(pid) = pid_str.trim().parse::<u32>() {
                // #248: under FreeBSD's daemon(8) with `-p` (child pidfile),
                // the supervisor writes OUR OWN pid into this file before we
                // get here. That's us, not a rival instance.
                if pid == std::process::id() {
                    info!("PID file already holds our PID {} (daemon(8) child pidfile) — lock is ours", pid);
                    return Ok(Self { pid_path });
                }
                if Self::process_is_alive(pid) {
                    anyhow::bail!(
                        "Gateway already running (PID {}). Kill it first or remove {}",
                        pid,
                        pid_path.display()
                    );
                }
                // Stale PID file — clean it up
                let _ = std::fs::remove_file(&pid_path);
                info!("Removed stale PID file (PID {} no longer running)", pid);
            }

        // Check port availability
        if Self::port_in_use(port) {
            anyhow::bail!(
                "Port {} is already in use. Another process may be bound to it.",
                port
            );
        }

        // Write our PID.
        //
        // #248: under daemon(8) the process may start with no $HOME and no
        // $ZEUS_HOME, so `zeus_home()` can resolve somewhere unwritable
        // (worst case the shared `/tmp/.zeus` fallback owned by another
        // user). A bare io::Error here gives the operator nothing to act
        // on — name the resolved path, the uid, and the remediation.
        if let Some(parent) = pid_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                anyhow::anyhow!(
                    "Cannot create Zeus home dir {} (uid {}): {}. \
                     The gateway resolves its home via $ZEUS_HOME → $HOME/.zeus. \
                     When run under daemon(8)/a service manager, export ZEUS_HOME \
                     explicitly (the rc.d script does this via /usr/bin/env) or \
                     fix ownership: chown -R <user> {}",
                    parent.display(),
                    unsafe { libc::getuid() },
                    e,
                    parent.display()
                )
            })?;
        }
        std::fs::write(&pid_path, std::process::id().to_string()).map_err(|e| {
            anyhow::anyhow!(
                "Cannot write gateway PID lock {} (uid {}): {}. \
                 Check ownership of the directory, or set ZEUS_HOME to a \
                 writable location before starting the gateway.",
                pid_path.display(),
                unsafe { libc::getuid() },
                e
            )
        })?;
        info!("Gateway lock acquired (PID {}, port {})", std::process::id(), port);

        Ok(Self { pid_path })
    }

    fn process_is_alive(pid: u32) -> bool {
        // OBSERVABILITY-ONLY: existing logic preserved, behavior unchanged.
        // Diagnostic warn-level logs cover each return-branch so we can
        // root-cause the .112 false-positive "Gateway already running" bail.
        // Also captures `ps -o pid,comm,args` of the offending PID to detect
        // PID recycling (unrelated process holding the slot after gateway death).

        // kill(pid, 0) checks if process exists without sending a signal
        let kill_rc = unsafe { libc::kill(pid as i32, 0) };
        if kill_rc != 0 {
            warn!(target: "gateway_lock", "process_is_alive(pid={}) → false (kill -0 rc={}, errno suggests dead)", pid, kill_rc);
            return false;
        }
        warn!(target: "gateway_lock", "process_is_alive(pid={}) kill -0 → alive; verifying it's a zeus binary", pid);

        // Diagnostic: capture full ps output so we can see what process actually has this PID
        if let Ok(diag) = std::process::Command::new("ps")
            .args(["-o", "pid,comm,args", "-p", &pid.to_string()])
            .output()
        {
            warn!(target: "gateway_lock",
                "process_is_alive(pid={}) ps diag: stdout={:?} stderr={:?} status={}",
                pid,
                String::from_utf8_lossy(&diag.stdout).trim(),
                String::from_utf8_lossy(&diag.stderr).trim(),
                diag.status,
            );
        }

        // Verify it's actually a zeus process (not a recycled PID)
        #[cfg(target_os = "macos")]
        {
            if let Ok(output) = std::process::Command::new("ps")
                .args(["-p", &pid.to_string(), "-o", "comm="])
                .output()
            {
                let comm = String::from_utf8_lossy(&output.stdout);
                let trimmed = comm.trim();
                let is_zeus = trimmed.contains("zeus");
                warn!(target: "gateway_lock",
                    "process_is_alive(pid={}) macOS ps comm= → {:?} (contains 'zeus'={}); returning {}",
                    pid, trimmed, is_zeus, is_zeus
                );
                return is_zeus;
            }
            warn!(target: "gateway_lock", "process_is_alive(pid={}) macOS ps comm= command failed; falling through to fallback=true", pid);
        }
        #[cfg(target_os = "freebsd")]
        {
            if let Ok(output) = std::process::Command::new("ps")
                .args(["-p", &pid.to_string(), "-o", "comm="])
                .output()
            {
                let comm = String::from_utf8_lossy(&output.stdout);
                return comm.trim().contains("zeus");
            }
        }
        #[cfg(target_os = "linux")]
        {
            if let Ok(cmdline) = std::fs::read_to_string(format!("/proc/{}/cmdline", pid)) {
                let is_zeus = cmdline.contains("zeus");
                warn!(target: "gateway_lock",
                    "process_is_alive(pid={}) Linux /proc/cmdline → {:?} (contains 'zeus'={}); returning {}",
                    pid, cmdline, is_zeus, is_zeus
                );
                return is_zeus;
            }
            warn!(target: "gateway_lock", "process_is_alive(pid={}) Linux /proc/cmdline read failed; falling through to fallback=true", pid);
        }
        warn!(target: "gateway_lock", "process_is_alive(pid={}) → true (FALLBACK: kill -0 alive but ps/proc verification unavailable)", pid);
        true // fallback: assume alive if we can't verify
    }

    fn port_in_use(port: u16) -> bool {
        std::net::TcpListener::bind(("0.0.0.0", port)).is_err()
            && std::net::TcpListener::bind(("127.0.0.1", port)).is_err()
    }
}

impl Drop for GatewayLock {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_file(&self.pid_path) {
            // Only warn if file existed — it might have been manually removed
            if e.kind() != std::io::ErrorKind::NotFound {
                eprintln!("Warning: failed to remove PID file {}: {}", self.pid_path.display(), e);
            }
        }
    }
}
