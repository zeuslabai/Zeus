//! Daemon management - install/uninstall Zeus as a system service
//!
//! On macOS: Uses launchd with a plist in ~/Library/LaunchAgents/
//! On Linux: Uses systemd user units in ~/.config/systemd/user/

use anyhow::Result;
use std::path::PathBuf;

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

/// Restart: stop, optionally clear sessions, wait 2 seconds, then start
async fn restart_daemon(fresh: bool) -> Result<()> {
    println!("Restarting daemon...");
    stop_daemon().await?;
    if fresh {
        let sessions_dir = dirs::home_dir()
            .unwrap_or_default()
            .join(".zeus")
            .join("sessions");
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
        let fresh_marker = dirs::home_dir()
            .unwrap_or_default()
            .join(".zeus")
            .join(".fresh_start");
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
    dirs::home_dir()
        .unwrap_or_default()
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
    let config_dir = dirs::home_dir().unwrap_or_default().join(".zeus");
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
    let pid_file = dirs::home_dir()
        .unwrap_or_default()
        .join(".zeus")
        .join("gateway.pid");

    if pid_file.exists() {
        if let Ok(pid_str) = tokio::fs::read_to_string(&pid_file).await {
            if let Ok(pid) = pid_str.trim().parse::<i32>() {
                unsafe { libc::kill(pid, libc::SIGTERM); }
                // Wait up to 5 seconds for graceful shutdown
                for _ in 0..10 {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    if unsafe { libc::kill(pid, 0) } != 0 { break; }
                }
                // Force kill if still alive
                if unsafe { libc::kill(pid, 0) } == 0 {
                    unsafe { libc::kill(pid, libc::SIGKILL); }
                }
            }
        }
        let _ = tokio::fs::remove_file(&pid_file).await;
    }

    // Step 3: Kill any remaining gateway processes by name
    let _ = tokio::process::Command::new("pkill")
        .args(["-f", "zeus gateway"])
        .output()
        .await;

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
    let pid_file = dirs::home_dir()
        .unwrap_or_default()
        .join(".zeus")
        .join("gateway.pid");

    if pid_file.exists() {
        if let Ok(pid_str) = tokio::fs::read_to_string(&pid_file).await {
            if let Ok(pid) = pid_str.trim().parse::<i32>() {
                unsafe { libc::kill(pid, libc::SIGTERM); }
                // Wait up to 5 seconds for graceful shutdown
                for _ in 0..10 {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    if unsafe { libc::kill(pid, 0) } != 0 { break; }
                }
                // Force kill if still alive
                if unsafe { libc::kill(pid, 0) } == 0 {
                    unsafe { libc::kill(pid, libc::SIGKILL); }
                }
            }
        }
        let _ = tokio::fs::remove_file(&pid_file).await;
    }

    // Step 3: Kill any remaining gateway processes by name
    let _ = tokio::process::Command::new("pkill")
        .args(["-f", "zeus gateway"])
        .output()
        .await;

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

#[cfg(target_os = "freebsd")]
fn generate_rcd_script(zeus_path: &str) -> String {
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
: ${{zeus_gateway_user:="mike"}}
: ${{zeus_gateway_config:=""}}
: ${{zeus_gateway_logfile:="/var/log/zeus-gateway.log"}}
: ${{zeus_gateway_pidfile:="/var/run/${{name}}.pid"}}

pidfile="${{zeus_gateway_pidfile}}"
procname="{zeus_path}"
command="/usr/sbin/daemon"

start_cmd="${{name}}_start"
stop_postcmd="${{name}}_poststop"

zeus_gateway_start()
{{
    echo "Starting ${{name}}."
    /usr/sbin/daemon -P "${{pidfile}}" \
        -u "${{zeus_gateway_user}}" \
        -o "${{zeus_gateway_logfile}}" \
        {zeus_path} gateway \
        --host "${{zeus_gateway_host}}" \
        --port "${{zeus_gateway_port}}"
}}

zeus_gateway_poststop()
{{
    rm -f "${{pidfile}}"
}}

run_rc_command "$1"
"#
    )
}

#[cfg(target_os = "freebsd")]
async fn install_daemon() -> Result<()> {
    let zeus_path = zeus_binary_path()?;
    let script = generate_rcd_script(&zeus_path.to_string_lossy());
    let path = rcd_path();

    if let Err(e) = tokio::fs::write(&path, &script).await {
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            anyhow::bail!(
                "Permission denied writing to {}. On FreeBSD, run: sudo zeus daemon install",
                path.display()
            );
        }
        return Err(e.into());
    }

    // Make executable
    let _ = tokio::process::Command::new("chmod")
        .args(["755", &path.to_string_lossy()])
        .output()
        .await;

    // Enable in rc.conf if not already
    let rc_conf = tokio::fs::read_to_string("/etc/rc.conf").await.unwrap_or_default();
    if !rc_conf.contains("zeus_gateway_enable") {
        let _ = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(r#"echo 'zeus_gateway_enable="YES"' >> /etc/rc.conf"#)
            .output()
            .await;
    }

    println!("Installed rc.d service: {}", path.display());
    println!("Run 'zeus daemon start' to start the daemon");
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
    let pid_file = dirs::home_dir()
        .unwrap_or_default()
        .join(".zeus")
        .join("gateway.pid");

    if pid_file.exists() {
        if let Ok(pid_str) = tokio::fs::read_to_string(&pid_file).await {
            if let Ok(pid) = pid_str.trim().parse::<i32>() {
                unsafe { libc::kill(pid, libc::SIGTERM); }
                // Wait up to 5 seconds for graceful shutdown
                for _ in 0..10 {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    if unsafe { libc::kill(pid, 0) } != 0 { break; }
                }
                // Force kill if still alive
                if unsafe { libc::kill(pid, 0) } == 0 {
                    unsafe { libc::kill(pid, libc::SIGKILL); }
                }
            }
        }
        let _ = tokio::fs::remove_file(&pid_file).await;
    }

    // Step 3: Kill any remaining gateway processes by name
    let _ = tokio::process::Command::new("pkill")
        .args(["-f", "zeus gateway"])
        .output()
        .await;

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
}
