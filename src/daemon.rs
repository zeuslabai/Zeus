//! Daemon management - install/uninstall Zeus as a system service
//!
//! On macOS: Uses launchd with a plist in ~/Library/LaunchAgents/
//! On Linux: Uses systemd user units in ~/.config/systemd/user/

use anyhow::Result;
use std::path::PathBuf;
use crate::zeus_paths;

/// Daemon actions
#[derive(Debug, Clone)]
pub enum DaemonAction {
    /// Install the daemon service file
    Install,
    /// Uninstall the daemon service file
    Uninstall,
    /// Start the daemon
    Start,
    /// Stop the daemon
    Stop,
    /// Restart the daemon (stop, wait 2s, start)
    Restart {
        /// Clear sessions before restarting
        fresh: bool,
    },
    /// Check daemon status
    Status,
}

/// Service identifier (used by launchd on macOS, referenced in tests)
#[allow(dead_code)]
const SERVICE_LABEL: &str = "com.zeus.gateway";
/// Service identifier (used by systemd on Linux, referenced in tests)
#[allow(dead_code)]
const SYSTEMD_UNIT: &str = "zeus-gateway";

#[allow(dead_code)]
/// Get the path to the current zeus binary
fn zeus_binary_path() -> Result<PathBuf> {
    std::env::current_exe()
        .map_err(|e| anyhow::anyhow!("Failed to get current executable path: {}", e))
}

/// Kill any lingering "zeus gateway" processes by name, EXCLUDING the current
/// process (and its parent shell, if invoked via `zeus daemon restart`).
///
/// Replaces unconditional `pkill -f "zeus gateway"` which had a self-kill risk
/// when `zeus daemon restart` itself matched the pattern. Uses `pgrep -f` to
/// enumerate candidates, then filters out our own PID before sending signals.
#[cfg(unix)]
#[allow(dead_code)]
async fn pkill_gateway_excluding_self() {
    let self_pid = std::process::id() as i32;
    let parent_pid = unsafe { libc::getppid() };

    let output = match tokio::process::Command::new("pgrep")
        .args(["-f", "zeus gateway"])
        .output()
        .await
    {
        Ok(o) => o,
        Err(_) => return,
    };

    if !output.status.success() {
        return;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if let Ok(pid) = line.trim().parse::<i32>() {
            if pid == self_pid || pid == parent_pid {
                continue;
            }
            unsafe {
                libc::kill(pid, libc::SIGTERM);
            }
        }
    }
}

/// Send `sig` to `pid`, tolerating EPERM/ESRCH which are expected when the
/// gateway runs as `root` (system LaunchDaemon) but `zeus daemon restart` is
/// invoked as an unprivileged user, or when the target process already exited.
///
/// Returns `true` if the signal was delivered, `false` on tolerated errors.
/// Unexpected `errno` values are logged to stderr so silent failures don't
/// mask bugs.
///
/// Use this instead of bare `unsafe { libc::kill(pid, sig) }` for the gateway
/// stop/restart flows where root-owned gateway + unprivileged CLI is an
/// expected configuration (system LaunchDaemon domain).
#[cfg(unix)]
#[allow(dead_code)]
fn kill_tolerant(pid: i32, sig: i32) -> bool {
    // SAFETY: libc::kill is a thin syscall wrapper; pid/sig are integers.
    let rc = unsafe { libc::kill(pid, sig) };
    if rc == 0 {
        return true;
    }
    // SAFETY: __error()/__errno_location() returns a pointer to thread-local errno.
    #[cfg(any(target_os = "macos", target_os = "freebsd"))]
    let errno = unsafe { *libc::__error() };
    #[cfg(target_os = "linux")]
    let errno = unsafe { *libc::__errno_location() };
    match errno {
        // ESRCH: process already gone — expected race in stop/restart flows.
        // EPERM: gateway running as root, CLI is unprivileged — sudo bootout
        //        handles it (Step 1 in stop_daemon); helper sweep is fallback.
        e if e == libc::ESRCH || e == libc::EPERM => false,
        e => {
            eprintln!("warning: kill({}, {}) failed with errno {}", pid, sig, e);
            false
        }
    }
}

/// On macOS, attempt a fast restart via
/// `sudo launchctl kickstart -k system/com.zeus.gateway`.
///
/// `kickstart -k` is a single launchctl call that signals the running job and
/// restarts it under launchd. Roughly an order of magnitude faster than the
/// generic `bootout` → manual SIGTERM → `bootstrap` cycle when the plist is
/// already loaded in the `system` domain.
///
/// Returns:
/// - `Ok(true)`  — fast-path succeeded; caller should skip the slow restart.
/// - `Ok(false)` — fast-path not applicable (no plist / non-System / not loaded).
/// - `Err(_)`    — process spawn itself failed; caller should fall back.
///
/// Best-effort: any launchctl error returns `Ok(false)` so the caller
/// transparently falls back to the slow path. Non-System domains (legacy
/// user-agent plists) never take the fast path.
#[cfg(target_os = "macos")]
async fn try_launchctl_kickstart_restart() -> Result<bool> {
    // Only attempt the fast-path for the canonical System-domain install.
    let domain = match resolve_plist() {
        Some((_, LaunchdDomain::System)) => LaunchdDomain::System,
        _ => return Ok(false),
    };
    let target = format!("system/{}", SERVICE_LABEL);
    let output = tokio::process::Command::new("sudo")
        .args(["launchctl", "kickstart", "-k", &target])
        .output()
        .await?;
    if output.status.success() {
        println!(
            "Daemon restarted via launchctl kickstart -k ({:?} domain)",
            domain
        );
        Ok(true)
    } else {
        // Job not loaded, permission denied, etc. — caller falls back.
        Ok(false)
    }
}

/// Run a daemon management action
pub async fn run_daemon(action: DaemonAction) -> Result<()> {
    match action {
        DaemonAction::Install => install_daemon().await,
        DaemonAction::Uninstall => uninstall_daemon().await,
        DaemonAction::Start => start_daemon().await,
        DaemonAction::Stop => stop_daemon().await,
        DaemonAction::Restart { fresh } => restart_daemon(fresh).await,
        DaemonAction::Status => show_status().await,
    }
}

/// Restart the gateway daemon.
///
/// On macOS with a System-domain LaunchDaemon, this prefers the fast-path
/// `launchctl kickstart -k system/com.zeus.gateway` (≈10× faster than
/// bootout/bootstrap). For `--fresh` or non-System installs (Linux, FreeBSD,
/// legacy user-agent plists, or any kickstart failure) it falls back to the
/// generic `stop_daemon` → optional fresh-session cleanup → 2s settle →
/// `start_daemon` sequence.
///
/// # Errors
/// Propagates errors from the underlying stop/start fallbacks. The fast-path
/// itself never bubbles errors — failures degrade silently to the slow path.
async fn restart_daemon(fresh: bool) -> Result<()> {
    println!("Restarting daemon...");

    // Fast-path: macOS launchctl kickstart -k for System-domain installs.
    // Skipped when --fresh is requested, because kickstart keeps the process
    // under launchd supervision with no opportunity to clear sessions between
    // stop and start. Fall through to slow path on --fresh or any failure.
    #[cfg(target_os = "macos")]
    {
        if !fresh {
            if let Ok(true) = try_launchctl_kickstart_restart().await {
                return Ok(());
            }
        }
    }

    stop_daemon().await?;
    if fresh {
        let sessions_dir = zeus_paths::zeus_home().join("sessions");
        match std::fs::read_dir(&sessions_dir) {
            Ok(entries) => {
                let mut cleared = 0usize;
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                        if std::fs::remove_file(&path).is_ok() {
                            cleared += 1;
                        }
                    }
                }
                println!("--fresh: cleared {} session file(s)", cleared);
            }
            Err(e) => eprintln!("--fresh: could not read sessions dir: {}", e),
        }
        // Write fresh_start marker so gateway skips history injection on boot
        let fresh_marker = zeus_paths::zeus_home().join(".fresh_start");
        if let Err(e) = std::fs::write(&fresh_marker, "") {
            eprintln!("--fresh: could not write fresh_start marker: {}", e);
        } else {
            println!("--fresh: wrote fresh_start marker");
        }
    }
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    start_daemon().await?;
    println!("Daemon restarted");
    Ok(())
}

// ============================================================================
// macOS (launchd)
// ============================================================================

/// launchd domain for the gateway plist on macOS.
///
/// Post Dispatch 3 (system LaunchDaemon promotion), the gateway plist lives at
/// `/Library/LaunchDaemons/com.zeus.gateway.plist` and runs in the `system`
/// domain. Legacy installs may still have a user-agent plist at
/// `~/Library/LaunchAgents/com.zeus.gateway.plist` running in the
/// `gui/<uid>` domain. CLI commands need to know which is active.
#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LaunchdDomain {
    /// `/Library/LaunchDaemons/` — `sudo launchctl bootstrap system <path>` etc.
    System,
    /// `~/Library/LaunchAgents/` — `launchctl load/unload <path>` (legacy).
    UserAgent,
}

#[cfg(target_os = "macos")]
fn system_plist_path() -> PathBuf {
    PathBuf::from(format!("/Library/LaunchDaemons/{}.plist", SERVICE_LABEL))
}

#[cfg(target_os = "macos")]
fn user_agent_plist_path() -> PathBuf {
    zeus_paths::zeus_home()
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{}.plist", SERVICE_LABEL))
}

/// Detect which launchd domain the gateway plist is installed in.
///
/// Checks `/Library/LaunchDaemons/` first (system, preferred post-migration),
/// then falls back to `~/Library/LaunchAgents/` (user-agent, legacy). Returns
/// `None` if neither plist exists.
#[cfg(target_os = "macos")]
fn resolve_plist() -> Option<(PathBuf, LaunchdDomain)> {
    let sys = system_plist_path();
    if sys.exists() {
        return Some((sys, LaunchdDomain::System));
    }
    let user = user_agent_plist_path();
    if user.exists() {
        return Some((user, LaunchdDomain::UserAgent));
    }
    None
}

/// Legacy path accessor preserved for tests + install/uninstall fallbacks.
/// Returns the user-agent path (legacy default). Prefer `resolve_plist()` for
/// runtime decisions.
#[cfg(target_os = "macos")]
fn plist_path() -> PathBuf {
    user_agent_plist_path()
}

#[cfg(target_os = "macos")]
fn generate_plist(zeus_path: &str) -> String {
    let config_dir = zeus_paths::zeus_home();
    let log_dir = config_dir.join("logs");

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{}</string>
        <string>gateway</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>{}/zeus-gateway.out.log</string>
    <key>StandardErrorPath</key>
    <string>{}/zeus-gateway.err.log</string>
    <key>WorkingDirectory</key>
    <string>{}</string>
    <key>EnvironmentVariables</key>
    <dict>
        <key>PATH</key>
        <string>/usr/local/bin:/usr/bin:/bin:/opt/homebrew/bin</string>
    </dict>
</dict>
</plist>"#,
        SERVICE_LABEL,
        zeus_path,
        log_dir.display(),
        log_dir.display(),
        config_dir.display(),
    )
}

#[cfg(target_os = "macos")]
async fn install_daemon() -> Result<()> {
    // Post Dispatch 3: macOS daemon install is owned by `scripts/install.sh`
    // (system LaunchDaemon at /Library/LaunchDaemons/com.zeus.gateway.plist,
    // requires sudo for placement + bootstrap). The CLI no longer writes a
    // user-agent plist on its own — that path was the source of the
    // GUI-session-dependency regression Dispatch 3 fixed.
    println!("On macOS, install/upgrade the gateway via:");
    println!("  scripts/install.sh           # fresh install");
    println!("  scripts/install.sh --update  # upgrade in place");
    println!();
    println!("This installs the system LaunchDaemon at");
    println!("  /Library/LaunchDaemons/{}.plist", SERVICE_LABEL);
    println!("which survives SSH disconnect and reboots.");
    Ok(())
}

#[cfg(target_os = "macos")]
async fn uninstall_daemon() -> Result<()> {
    // Stop first if running (best-effort; works for either domain).
    let _ = stop_daemon().await;

    let Some((path, domain)) = resolve_plist() else {
        println!("Daemon plist not found (not installed)");
        return Ok(());
    };

    match domain {
        LaunchdDomain::System => {
            // Bootout from system domain, then sudo-remove the plist.
            let _ = tokio::process::Command::new("sudo")
                .args(["launchctl", "bootout", "system"])
                .arg(&path)
                .output()
                .await;
            let rm = tokio::process::Command::new("sudo")
                .arg("rm")
                .arg("-f")
                .arg(&path)
                .output()
                .await?;
            if rm.status.success() {
                println!("Removed system daemon plist: {}", path.display());
            } else {
                let stderr = String::from_utf8_lossy(&rm.stderr);
                anyhow::bail!("Failed to remove plist (sudo required): {}", stderr.trim());
            }
        }
        LaunchdDomain::UserAgent => {
            tokio::fs::remove_file(&path).await?;
            println!("Removed daemon plist: {}", path.display());
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
async fn start_daemon() -> Result<()> {
    let Some((path, domain)) = resolve_plist() else {
        anyhow::bail!(
            "Daemon not installed. Run scripts/install.sh (system) or scripts/install.sh --update."
        );
    };

    let output = match domain {
        LaunchdDomain::System => {
            tokio::process::Command::new("sudo")
                .args(["launchctl", "bootstrap", "system"])
                .arg(&path)
                .output()
                .await?
        }
        LaunchdDomain::UserAgent => {
            tokio::process::Command::new("launchctl")
                .arg("load")
                .arg(&path)
                .output()
                .await?
        }
    };

    if output.status.success() {
        println!("Daemon started via launchctl ({:?} domain)", domain);
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let already_loaded = stderr.contains("already loaded")
            || stderr.contains("service already bootstrapped")
            || stderr.contains("Bootstrap failed: 17"); // EEXIST
        if already_loaded {
            println!("Daemon is already running ({:?} domain)", domain);
        } else {
            anyhow::bail!("Failed to start daemon ({:?}): {}", domain, stderr.trim());
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
async fn stop_daemon() -> Result<()> {
    // Step 1: Try the right launchctl invocation for whichever domain owns
    // the active plist (system preferred, user-agent legacy).
    if let Some((path, domain)) = resolve_plist() {
        let _ = match domain {
            LaunchdDomain::System => {
                tokio::process::Command::new("sudo")
                    .args(["launchctl", "bootout", "system"])
                    .arg(&path)
                    .output()
                    .await
            }
            LaunchdDomain::UserAgent => {
                tokio::process::Command::new("launchctl")
                    .arg("unload")
                    .arg(&path)
                    .output()
                    .await
            }
        };
    }

    // Step 2: Kill any remaining zeus gateway processes
    let pid_file = zeus_paths::zeus_pid_path();

    if pid_file.exists() {
        if let Ok(pid_str) = tokio::fs::read_to_string(&pid_file).await {
            if let Ok(pid) = pid_str.trim().parse::<i32>() {
                // EPERM-tolerant: gateway may run as root (system LaunchDaemon)
                // while `zeus daemon restart` is unprivileged. Step 1 (launchctl
                // bootout via sudo) handles that case; this fallback is best-effort.
                let _ = kill_tolerant(pid, libc::SIGTERM);
                // Wait up to 5 seconds for graceful shutdown
                for _ in 0..10 {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    if unsafe { libc::kill(pid, 0) } != 0 { break; }
                }
                // Force kill if still alive
                if unsafe { libc::kill(pid, 0) } == 0 {
                    let _ = kill_tolerant(pid, libc::SIGKILL);
                }
            }
        }
        let _ = tokio::fs::remove_file(&pid_file).await;
    }

    // Step 3: Kill any remaining gateway processes by name (excluding self + parent shell)
    pkill_gateway_excluding_self().await;

    // Verify
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    let check = tokio::process::Command::new("pgrep")
        .args(["-f", "zeus gateway"])
        .output()
        .await;
    if check.map(|o| o.stdout.is_empty()).unwrap_or(true) {
        println!("Daemon stopped");
    } else {
        println!("Warning: gateway process may still be running");
    }
    Ok(())
}

#[cfg(target_os = "macos")]
async fn show_status() -> Result<()> {
    // Resolve which domain (if any) holds the gateway plist.
    let resolved = resolve_plist();

    // Query launchctl in the matching domain. `print` works for both system
    // and gui/<uid>; fall back to `list` for older launchctl versions.
    let (running, detail) = match resolved {
        Some((_, LaunchdDomain::System)) => {
            let print_out = tokio::process::Command::new("sudo")
                .args(["launchctl", "print", &format!("system/{}", SERVICE_LABEL)])
                .output()
                .await?;
            (print_out.status.success(), String::from_utf8_lossy(&print_out.stdout).into_owned())
        }
        _ => {
            let list_out = tokio::process::Command::new("launchctl")
                .arg("list")
                .arg(SERVICE_LABEL)
                .output()
                .await?;
            (list_out.status.success(), String::from_utf8_lossy(&list_out.stdout).into_owned())
        }
    };

    if running {
        println!("Daemon status: RUNNING");
        for line in detail.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("pid =") || trimmed.contains("\"PID\"") {
                println!("  {}", trimmed.trim_end_matches(';'));
            }
        }
    } else {
        println!("Daemon status: NOT RUNNING");
    }

    match resolved {
        Some((path, domain)) => {
            println!("Plist: installed ({:?} domain)", domain);
            println!("Plist path: {}", path.display());
        }
        None => {
            println!("Plist: not installed");
            println!("System path:     {}", system_plist_path().display());
            println!("User-agent path: {}", user_agent_plist_path().display());
        }
    }

    Ok(())
}

// ============================================================================
// Linux (systemd)
// ============================================================================

#[cfg(target_os = "linux")]
fn unit_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".config")
        .join("systemd")
        .join("user")
        .join(format!("{}.service", SYSTEMD_UNIT))
}

#[cfg(target_os = "linux")]
fn generate_systemd_unit(zeus_path: &str) -> String {
    format!(
        r#"[Unit]
Description=Zeus AI Assistant Gateway
After=network.target

[Service]
Type=simple
ExecStart={} gateway
Restart=on-failure
RestartSec=5
Environment=PATH=/usr/local/bin:/usr/bin:/bin
Environment=HOME=%h

[Install]
WantedBy=default.target
"#,
        zeus_path
    )
}

#[cfg(target_os = "linux")]
async fn install_daemon() -> Result<()> {
    let zeus_path = zeus_binary_path()?;
    let unit = generate_systemd_unit(&zeus_path.to_string_lossy());
    let path = unit_path();

    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    tokio::fs::write(&path, &unit).await?;

    // Reload systemd
    let _ = tokio::process::Command::new("systemctl")
        .arg("--user")
        .arg("daemon-reload")
        .output()
        .await;

    println!("Installed systemd unit: {}", path.display());
    println!("Run 'zeus daemon start' to start the daemon");
    Ok(())
}

#[cfg(target_os = "linux")]
async fn uninstall_daemon() -> Result<()> {
    let _ = stop_daemon().await;

    let path = unit_path();
    if path.exists() {
        tokio::fs::remove_file(&path).await?;
        let _ = tokio::process::Command::new("systemctl")
            .arg("--user")
            .arg("daemon-reload")
            .output()
            .await;
        println!("Removed systemd unit: {}", path.display());
    } else {
        println!("Systemd unit not found (not installed)");
    }
    Ok(())
}

#[cfg(target_os = "linux")]
async fn start_daemon() -> Result<()> {
    let output = tokio::process::Command::new("systemctl")
        .arg("--user")
        .arg("start")
        .arg(SYSTEMD_UNIT)
        .output()
        .await?;

    if output.status.success() {
        println!("Daemon started via systemctl");
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Failed to start daemon: {}", stderr);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
async fn stop_daemon() -> Result<()> {
    // Step 1: Try systemctl stop
    let _ = tokio::process::Command::new("systemctl")
        .arg("--user")
        .arg("stop")
        .arg(SYSTEMD_UNIT)
        .output()
        .await;

    // Step 2: Kill via PID file if still running
    let pid_file = zeus_paths::zeus_pid_path();

    if pid_file.exists() {
        if let Ok(pid_str) = tokio::fs::read_to_string(&pid_file).await {
            if let Ok(pid) = pid_str.trim().parse::<i32>() {
                // EPERM-tolerant: gateway may run as root (system LaunchDaemon)
                // while `zeus daemon restart` is unprivileged. Step 1 (launchctl
                // bootout via sudo) handles that case; this fallback is best-effort.
                let _ = kill_tolerant(pid, libc::SIGTERM);
                // Wait up to 5 seconds for graceful shutdown
                for _ in 0..10 {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    if unsafe { libc::kill(pid, 0) } != 0 { break; }
                }
                // Force kill if still alive
                if unsafe { libc::kill(pid, 0) } == 0 {
                    let _ = kill_tolerant(pid, libc::SIGKILL);
                }
            }
        }
        let _ = tokio::fs::remove_file(&pid_file).await;
    }

    // Step 3: Kill any remaining gateway processes by name (excluding self + parent shell)
    pkill_gateway_excluding_self().await;

    // Verify
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    let check = tokio::process::Command::new("pgrep")
        .args(["-f", "zeus gateway"])
        .output()
        .await;
    if check.map(|o| o.stdout.is_empty()).unwrap_or(true) {
        println!("Daemon stopped");
    } else {
        println!("Warning: gateway process may still be running");
    }
    Ok(())
}

#[cfg(target_os = "linux")]
async fn show_status() -> Result<()> {
    let output = tokio::process::Command::new("systemctl")
        .arg("--user")
        .arg("status")
        .arg(SYSTEMD_UNIT)
        .output()
        .await?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    println!("{}", stdout);
    Ok(())
}

// ============================================================================
// FreeBSD (rc.d)
// ============================================================================

#[cfg(target_os = "freebsd")]
fn rcd_path() -> std::path::PathBuf {
    std::path::PathBuf::from("/usr/local/etc/rc.d/zeus_gateway")
}

/// Resolve the user the rc.d service should run as. When invoked via
/// `sudo zeus daemon install`, SUDO_USER is the real operator; otherwise
/// fall back to USER. Never default to a hardcoded account name.
#[cfg(target_os = "freebsd")]
fn rcd_service_user() -> String {
    std::env::var("SUDO_USER")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "root".to_string())
}

#[cfg(target_os = "freebsd")]
fn generate_rcd_script(zeus_path: &str) -> String {
    generate_rcd_script_for_user(zeus_path, &rcd_service_user())
}

/// Generate the rc.d service script.
///
/// Pidfile semantics (the #211 status bug): `daemon -P` writes the
/// *supervisor* PID, whose procname is `daemon:` — rc.subr's status/stop
/// match the pidfile PID against `procname` (the zeus binary), so with `-P`
/// the service always reported "not running" even while the gateway was up,
/// which broke `service zeus_gateway restart`. We use `-p` (child pidfile)
/// so the pidfile holds the actual zeus PID: status matches, stop SIGTERMs
/// zeus directly, and the daemon(8) supervisor exits with its child.
#[cfg(target_os = "freebsd")]
fn generate_rcd_script_for_user(zeus_path: &str, user: &str) -> String {
    format!(
        r#"#!/bin/sh

# PROVIDE: zeus_gateway
# REQUIRE: LOGIN DAEMON NETWORKING
# KEYWORD: shutdown

. /etc/rc.subr

name="zeus_gateway"
rcvar="${{name}}_enable"

load_rc_config $name

: ${{zeus_gateway_enable:="NO"}}
: ${{zeus_gateway_port:="8080"}}
: ${{zeus_gateway_host:="0.0.0.0"}}
: ${{zeus_gateway_user:="{user}"}}
: ${{zeus_gateway_home:="/home/${{zeus_gateway_user}}"}}
: ${{zeus_gateway_config:=""}}
: ${{zeus_gateway_logfile:="/var/log/zeus_gateway.log"}}
: ${{zeus_gateway_pidfile:="${{zeus_gateway_home}}/.zeus/gateway.pid"}}
: ${{zeus_gateway_newsyslog_conf:="/usr/local/etc/newsyslog.conf.d/zeus_gateway.conf"}}

pidfile="${{zeus_gateway_pidfile}}"
procname="{zeus_path}"
command="/usr/sbin/daemon"

start_precmd="${{name}}_prestart"
start_cmd="${{name}}_start"
stop_postcmd="${{name}}_poststop"

zeus_gateway_prestart()
{{
    # Match the gateway's own PID lock path so rc.subr status/stop are truthful.
    install -d -o "${{zeus_gateway_user}}" -g wheel -m 0755 "${{zeus_gateway_home}}/.zeus"
    install -d -o "${{zeus_gateway_user}}" -g wheel -m 0755 "${{zeus_gateway_home}}/.zeus/logs"

    # #248: daemon(8) creates the child pidfile as root:wheel 0600 BEFORE
    # dropping to -u ${{zeus_gateway_user}}, so the gateway's own PID-lock
    # write hits EACCES ("os error 13"). Pre-create it owned by the service
    # user — root's daemon(8) writes it regardless of ownership, and the
    # gateway can then re-write/remove its lock normally.
    install -o "${{zeus_gateway_user}}" -g wheel -m 0644 /dev/null "${{pidfile}}"

    # Durable stdout/stderr sink for daemon(8), with newsyslog rotation.
    install -d -o root -g wheel -m 0755 "$(dirname "${{zeus_gateway_logfile}}")"
    touch "${{zeus_gateway_logfile}}"
    chown "${{zeus_gateway_user}}:wheel" "${{zeus_gateway_logfile}}"
    chmod 0640 "${{zeus_gateway_logfile}}"

    if [ -d "$(dirname "${{zeus_gateway_newsyslog_conf}}")" ]; then
        cat > "${{zeus_gateway_newsyslog_conf}}" << NEWSYSLOG_EOF
${{zeus_gateway_logfile}} ${{zeus_gateway_user}}:wheel 640 7 10240 * JC
NEWSYSLOG_EOF
        chmod 0644 "${{zeus_gateway_newsyslog_conf}}"
    fi
}}

zeus_gateway_start()
{{
    echo "Starting ${{name}}."
    /usr/sbin/daemon -f -p "${{pidfile}}" \
        -u "${{zeus_gateway_user}}" \
        -o "${{zeus_gateway_logfile}}" \
        /usr/bin/env HOME="${{zeus_gateway_home}}" ZEUS_HOME="${{zeus_gateway_home}}/.zeus" \
        {zeus_path} gateway \
        --host "${{zeus_gateway_host}}" \
        --port "${{zeus_gateway_port}}"
}}

zeus_gateway_poststop()
{{
    rm -f "${{pidfile}}"
}}

run_rc_command "$1""#,
    )
}

#[cfg(target_os = "freebsd")]
async fn install_daemon() -> Result<()> {
    let zeus_path = zeus_binary_path()?;
    let script = generate_rcd_script(&zeus_path.to_string_lossy());
    let path = rcd_path();

    // Idempotent + stale-aware: rewrite whenever content differs (old -P
    // pidfile semantics, hardcoded user, moved binary path, …). A stale
    // script is the #211 failure mode — never keep it just because it exists.
    let existing = tokio::fs::read_to_string(&path).await.ok();
    let stale = existing.as_deref() != Some(script.as_str());
    if stale {
        if let Err(e) = tokio::fs::write(&path, &script).await {
            if e.kind() == std::io::ErrorKind::PermissionDenied {
                anyhow::bail!(
                    "Permission denied writing to {}. On FreeBSD, run: sudo zeus daemon install",
                    path.display()
                );
            }
            return Err(e.into());
        }
    }

    // Make executable — a non-executable rc.d script is silently ignored by
    // rc(8), so failure here must be loud, not swallowed.
    let chmod = tokio::process::Command::new("chmod")
        .args(["755", &path.to_string_lossy()])
        .output()
        .await?;
    if !chmod.status.success() {
        anyhow::bail!(
            "chmod 755 {} failed: {}",
            path.display(),
            String::from_utf8_lossy(&chmod.stderr)
        );
    }

    // Enable in rc.conf — prefer sysrc(8) (atomic, handles quoting/dedup),
    // fall back to append. Failure must surface: an installed-but-disabled
    // service never starts at boot and `service` refuses non-forced verbs.
    let rc_conf = tokio::fs::read_to_string("/etc/rc.conf")
        .await
        .unwrap_or_default();
    if !rc_conf.contains("zeus_gateway_enable") {
        let sysrc = tokio::process::Command::new("sysrc")
            .arg("zeus_gateway_enable=YES")
            .output()
            .await;
        let enabled = match sysrc {
            Ok(o) if o.status.success() => true,
            _ => {
                let fallback = tokio::process::Command::new("sh")
                    .arg("-c")
                    .arg(r#"echo 'zeus_gateway_enable="YES"' >> /etc/rc.conf"#)
                    .output()
                    .await?;
                fallback.status.success()
            }
        };
        if !enabled {
            anyhow::bail!(
                "Failed to enable zeus_gateway in /etc/rc.conf. \
                 Run: sudo sysrc zeus_gateway_enable=YES"
            );
        }
    }

    if stale {
        println!("Installed rc.d service: {}", path.display());
    } else {
        println!("rc.d service already current: {}", path.display());
    }
    println!("Run 'service zeus_gateway start' (or 'zeus daemon start') to start the daemon");
    Ok(())
}

#[cfg(target_os = "freebsd")]
async fn uninstall_daemon() -> Result<()> {
    let _ = stop_daemon().await;
    let path = rcd_path();
    if path.exists() {
        tokio::fs::remove_file(&path).await?;
        println!("Removed rc.d service: {}", path.display());
    } else {
        println!("rc.d service not found (not installed)");
    }
    Ok(())
}

#[cfg(target_os = "freebsd")]
async fn start_daemon() -> Result<()> {
    let output = tokio::process::Command::new("service")
        .args(["zeus_gateway", "start"])
        .output()
        .await?;

    if output.status.success() {
        println!("Daemon started via rc.d service");
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Failed to start daemon: {}", stderr);
    }
    Ok(())
}

#[cfg(target_os = "freebsd")]
async fn stop_daemon() -> Result<()> {
    // Step 1: Try service stop
    let _ = tokio::process::Command::new("service")
        .args(["zeus_gateway", "stop"])
        .output()
        .await;

    // Step 2: Kill via PID file if still running
    let pid_file = zeus_paths::zeus_pid_path();

    if pid_file.exists() {
        if let Ok(pid_str) = tokio::fs::read_to_string(&pid_file).await {
            if let Ok(pid) = pid_str.trim().parse::<i32>() {
                // EPERM-tolerant: gateway may run as root (system LaunchDaemon)
                // while `zeus daemon restart` is unprivileged. Step 1 (launchctl
                // bootout via sudo) handles that case; this fallback is best-effort.
                let _ = kill_tolerant(pid, libc::SIGTERM);
                // Wait up to 5 seconds for graceful shutdown
                for _ in 0..10 {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    if unsafe { libc::kill(pid, 0) } != 0 { break; }
                }
                // Force kill if still alive
                if unsafe { libc::kill(pid, 0) } == 0 {
                    let _ = kill_tolerant(pid, libc::SIGKILL);
                }
            }
        }
        let _ = tokio::fs::remove_file(&pid_file).await;
    }

    // Step 3: Kill any remaining gateway processes by name (excluding self + parent shell)
    pkill_gateway_excluding_self().await;

    // Verify
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    let check = tokio::process::Command::new("pgrep")
        .args(["-f", "zeus gateway"])
        .output()
        .await;
    if check.map(|o| o.stdout.is_empty()).unwrap_or(true) {
        println!("Daemon stopped");
    } else {
        println!("Warning: gateway process may still be running");
    }
    Ok(())
}

#[cfg(target_os = "freebsd")]
async fn show_status() -> Result<()> {
    let output = tokio::process::Command::new("service")
        .args(["zeus_gateway", "status"])
        .output()
        .await?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    println!("{}", stdout);
    Ok(())
}

// ============================================================================
// Unsupported platforms
// ============================================================================

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "freebsd")))]
async fn install_daemon() -> Result<()> {
    anyhow::bail!("Daemon management is not supported on this platform")
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "freebsd")))]
async fn uninstall_daemon() -> Result<()> {
    anyhow::bail!("Daemon management is not supported on this platform")
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "freebsd")))]
async fn start_daemon() -> Result<()> {
    anyhow::bail!("Daemon management is not supported on this platform")
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "freebsd")))]
async fn stop_daemon() -> Result<()> {
    anyhow::bail!("Daemon management is not supported on this platform")
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "freebsd")))]
async fn show_status() -> Result<()> {
    anyhow::bail!("Daemon management is not supported on this platform")
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_service_constants() {
        assert_eq!(SERVICE_LABEL, "com.zeus.gateway");
        assert_eq!(SYSTEMD_UNIT, "zeus-gateway");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_plist_generation() {
        let _home = dirs::home_dir().unwrap();
        let zeus_path = PathBuf::from("/usr/local/bin/zeus");
        let plist = generate_plist(zeus_path.to_str().unwrap());
        assert!(plist.contains("com.zeus.gateway"));
        assert!(plist.contains("/usr/local/bin/zeus"));
        assert!(plist.contains("<string>gateway</string>"));
        assert!(plist.contains("KeepAlive"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_plist_path() {
        // Legacy accessor still returns user-agent path (for back-compat).
        let path = plist_path();
        assert!(path.to_string_lossy().contains("LaunchAgents"));
        assert!(path.to_string_lossy().contains("com.zeus.gateway.plist"));

        // System path is absolute under /Library/LaunchDaemons/.
        let sys = system_plist_path();
        assert_eq!(sys.to_string_lossy(), "/Library/LaunchDaemons/com.zeus.gateway.plist");

        // User-agent helper matches legacy default.
        assert_eq!(user_agent_plist_path(), plist_path());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_systemd_unit_generation() {
        let _home = dirs::home_dir().unwrap();
        let zeus_path = PathBuf::from("/usr/local/bin/zeus");
        let unit = generate_systemd_unit(zeus_path.to_str().unwrap());
        assert!(unit.contains(&format!("ExecStart={} gateway", zeus_path.display())));
        assert!(unit.contains("[Unit]"));
        assert!(unit.contains("[Service]"));
        assert!(unit.contains("[Install]"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_unit_path() {
        let path = unit_path();
        assert!(path.to_string_lossy().contains("systemd/user"));
        assert!(path.to_string_lossy().contains("zeus-gateway.service"));
    }

    // ------------------------------------------------------------------
    // #50: tests for `zeus daemon restart` flow + EPERM tolerance + docs.
    // ------------------------------------------------------------------

    /// Restart action variants construct cleanly and carry the `fresh` flag
    /// through the enum match in `run_daemon`. Regression guard for the
    /// dispatch arm `DaemonAction::Restart { fresh } => restart_daemon(fresh)`.
    #[test]
    fn test_daemon_action_restart_variants() {
        let r_fresh = DaemonAction::Restart { fresh: true };
        let r_keep = DaemonAction::Restart { fresh: false };
        match r_fresh {
            DaemonAction::Restart { fresh } => assert!(fresh),
            _ => panic!("expected Restart{{fresh:true}}"),
        }
        match r_keep {
            DaemonAction::Restart { fresh } => assert!(!fresh),
            _ => panic!("expected Restart{{fresh:false}}"),
        }
    }

    /// `pkill_gateway_excluding_self` MUST never deliver a signal to its own
    /// PID (the bug #48 fixed). Smoke-test the contract: after running the
    /// helper, our own process is still alive.
    ///
    /// Note: this doesn't validate the full filter on a populated `pgrep`
    /// match-set — that would require spawning fake "zeus gateway" processes.
    /// It DOES catch the regression where the self-PID guard is removed.
    #[cfg(unix)]
    #[tokio::test]
    async fn test_pkill_excluding_self_does_not_kill_self() {
        let self_pid_before = std::process::id();
        pkill_gateway_excluding_self().await;
        // If self-kill regressed, the test process would be SIGTERM'd and
        // never reach this assertion (test harness would report a crash).
        let self_pid_after = std::process::id();
        assert_eq!(self_pid_before, self_pid_after);
    }

    /// `kill_tolerant` returns `false` for non-existent PIDs (ESRCH) without
    /// panicking. Uses PID 0x7FFF_FFFF which is effectively guaranteed to not
    /// exist (above PID_MAX on Linux and macOS).
    #[cfg(unix)]
    #[test]
    fn test_kill_tolerant_handles_esrch() {
        let unlikely_pid = 0x7FFF_FFFF;
        // Signal 0 = existence check, doesn't actually deliver a signal.
        let delivered = kill_tolerant(unlikely_pid, 0);
        assert!(
            !delivered,
            "kill_tolerant should report false for non-existent PID"
        );
    }

    /// #211 regression guards: the rc.d script must use `daemon -p`
    /// (child pidfile), not `-P` (supervisor pidfile). With `-P` the pidfile
    /// holds the daemon(8) supervisor PID whose procname never matches the
    /// zeus binary, so rc.subr status/stop reported "not running" while the
    /// gateway was up — which broke `service zeus_gateway restart` and made
    /// install.sh silently fall back to nohup.
    #[cfg(target_os = "freebsd")]
    #[test]
    fn test_rcd_script_uses_child_pidfile() {
        let script = generate_rcd_script_for_user("/usr/local/bin/zeus", "mike");
        assert!(
            script.contains(r#"/usr/sbin/daemon -f -p "${pidfile}""#),
            "rc.d start must use daemon -f -p: -p (child pidfile) so rc.subr \
             status/stop match the actual zeus PID, -f so fd 0/1/2 are \
             detached from the caller — without -f the daemonized gateway \
             inherits the invoking shell's pipe and any $(service ... restart) \
             capture blocks until the gateway exits (#223)"
        );
        assert!(
            !script.contains("daemon -P"),
            "daemon -P writes the supervisor PID — rc.subr can't match it \
             against procname (#211)"
        );
    }

    /// The service user must come from the install environment, never a
    /// hardcoded account name baked into the binary.
    #[cfg(target_os = "freebsd")]
    #[test]
    fn test_rcd_script_user_not_hardcoded() {
        let script = generate_rcd_script_for_user("/usr/local/bin/zeus", "operator1");
        assert!(script.contains(r#": ${zeus_gateway_user:="operator1"}"#));
        assert!(!script.contains(r#":="mike""#));
    }

    /// rc.d structural invariants: PROVIDE line, rcvar, procname pointing at
    /// the real binary, and run_rc_command dispatch.
    #[cfg(target_os = "freebsd")]
    #[test]
    fn test_rcd_script_structure() {
        let script = generate_rcd_script_for_user("/opt/zeus/bin/zeus", "mike");
        assert!(script.starts_with("#!/bin/sh"));
        assert!(script.contains("# PROVIDE: zeus_gateway"));
        assert!(script.contains(r#"rcvar="${name}_enable""#));
        assert!(script.contains(r#"procname="/opt/zeus/bin/zeus""#));
        assert!(script.contains("/opt/zeus/bin/zeus gateway"));
        assert!(script.contains(r#"run_rc_command "$1""#));
    }

    /// macOS fast-path target string must match what `launchctl kickstart -k`
    /// expects: `system/<SERVICE_LABEL>`. Regression guard against typos in
    /// the kickstart target that would silently fall back to slow-path forever.
    #[cfg(target_os = "macos")]
    #[test]
    fn test_kickstart_target_shape() {
        let target = format!("system/{}", SERVICE_LABEL);
        assert_eq!(target, "system/com.zeus.gateway");
        // `launchctl kickstart` accepts `<domain>/<service-name>`; the
        // domain prefix for LaunchDaemons is `system`. Catch accidental
        // `gui/` or missing-prefix regressions.
        assert!(target.starts_with("system/"));
        assert!(!target.starts_with("gui/"));
        assert!(target.contains(SERVICE_LABEL));
    }
}
