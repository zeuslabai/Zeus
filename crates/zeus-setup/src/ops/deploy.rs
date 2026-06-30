//! Fleet deployment — parallel SSH/SCP to remote hosts with full workspace setup
//!
//! Handles:
//! 1. Binary deployment (SCP + install to /usr/local/bin/zeus)
//! 2. Workspace setup (~/.zeus/ with config.toml, .env, workspace files)
//! 3. Service installation (launchd on macOS, rc.d on FreeBSD)
//! 4. Verification (zeus --version, workspace check)

use crate::event::ProgressEvent;
use crate::fleet::FleetNode;
use anyhow::Result;
use std::path::PathBuf;
use std::time::Instant;
use tokio::sync::mpsc;
use tokio::task::JoinSet;

/// Deploy configuration flags
#[derive(Debug, Clone)]
pub struct DeployOpts {
    /// Set up workspace (~/.zeus/, config.toml, .env) on remote hosts
    pub setup: bool,
    /// Only push config files, skip binary deployment
    pub config_only: bool,
    /// Install and start gateway service (launchd/rc.d)
    pub install_service: bool,
}

const SSH_OPTS: [&str; 4] = ["-o", "StrictHostKeyChecking=no", "-o", "ConnectTimeout=10"];

pub async fn run(
    targets: Vec<FleetNode>,
    binary_path: PathBuf,
    opts: DeployOpts,
    tx: mpsc::Sender<ProgressEvent>,
) -> Result<()> {
    let start = Instant::now();
    let total = targets.len();

    tx.send(ProgressEvent::StepStarted {
        name: "Fleet deployment".into(),
        index: 0,
        total: 1,
    })
    .await?;

    tx.send(ProgressEvent::LogLine(format!(
        "Deploying to {} hosts in parallel (setup={}, config_only={}, service={})...",
        total, opts.setup, opts.config_only, opts.install_service,
    )))
    .await?;

    // Deploy to all targets in parallel
    let mut join_set = JoinSet::new();

    for target in targets {
        let binary = binary_path.clone();
        let tx = tx.clone();
        let opts = opts.clone();
        join_set.spawn(async move { deploy_to_host(&target, &binary, &opts, &tx).await });
    }

    // Collect results
    let mut success_count = 0;
    while let Some(result) = join_set.join_next().await {
        match result {
            Ok(Ok(())) => success_count += 1,
            Ok(Err(e)) => {
                tx.send(ProgressEvent::LogLine(format!("Deploy error: {}", e)))
                    .await?;
            }
            Err(e) => {
                tx.send(ProgressEvent::LogLine(format!("Task error: {}", e)))
                    .await?;
            }
        }
    }

    let all_ok = success_count == total;
    tx.send(ProgressEvent::StepCompleted {
        name: "Fleet deployment".into(),
        message: format!("{}/{} succeeded", success_count, total),
    })
    .await?;

    tx.send(ProgressEvent::Finished {
        success: all_ok,
        elapsed: start.elapsed(),
        summary: format!("{}/{} hosts deployed", success_count, total),
    })
    .await?;

    Ok(())
}

async fn deploy_to_host(
    target: &FleetNode,
    binary: &std::path::Path,
    opts: &DeployOpts,
    tx: &mpsc::Sender<ProgressEvent>,
) -> Result<()> {
    let start = Instant::now();
    let ssh_target = target.ssh_target();

    // ── 1. Test SSH connectivity ──────────────────────────────
    tx.send(ProgressEvent::LogLine(format!(
        "[{}] Testing SSH to {}...",
        target.name, ssh_target,
    )))
    .await?;

    let status = ssh_cmd(&ssh_target, "echo ok").await?;
    if !status.success() {
        emit_result(tx, target, "SSH Failed", start).await?;
        anyhow::bail!("SSH failed for {}", target.name);
    }

    // ── 2. Detect remote OS (verify against fleet.conf) ──────
    let os_output = ssh_output(&ssh_target, "uname -s").await?;
    let remote_os = os_output.trim().to_string();

    tx.send(ProgressEvent::LogLine(format!(
        "[{}] OS: {} (fleet.conf: {})",
        target.name, remote_os, target.os,
    )))
    .await?;

    // ── 3. Stop service ──────────────────────────────────────
    if !opts.config_only {
        tx.send(ProgressEvent::LogLine(format!(
            "[{}] Stopping service...",
            target.name,
        )))
        .await?;

        let stop_cmd = stop_service_cmd(target);
        let _ = ssh_cmd(&ssh_target, &stop_cmd).await;
    }

    // ── 4. Deploy binary ─────────────────────────────────────
    if !opts.config_only {
        tx.send(ProgressEvent::LogLine(format!(
            "[{}] Copying binary...",
            target.name,
        )))
        .await?;

        let scp_status = tokio::process::Command::new("scp")
            .args(SSH_OPTS)
            .arg(binary)
            .arg(format!("{}:/tmp/zeus-deploy", ssh_target))
            .status()
            .await?;

        if !scp_status.success() {
            emit_result(tx, target, "SCP Failed", start).await?;
            anyhow::bail!("SCP failed for {}", target.name);
        }

        let install_cmd = "sudo cp -f /tmp/zeus-deploy /usr/local/bin/zeus && \
                           sudo chmod +x /usr/local/bin/zeus && \
                           rm -f /tmp/zeus-deploy";

        let install_status = ssh_cmd(&ssh_target, install_cmd).await?;
        if !install_status.success() {
            emit_result(tx, target, "Install Failed", start).await?;
            anyhow::bail!("Install failed for {}", target.name);
        }

        // Ad-hoc codesign on macOS
        if target.is_macos() {
            let _ = ssh_cmd(
                &ssh_target,
                "codesign --force --sign - /usr/local/bin/zeus 2>/dev/null || true",
            )
            .await;
        }

        tx.send(ProgressEvent::LogLine(format!(
            "[{}] Binary installed to /usr/local/bin/zeus",
            target.name,
        )))
        .await?;
    }

    // ── 5. Setup workspace ───────────────────────────────────
    if opts.setup {
        tx.send(ProgressEvent::LogLine(format!(
            "[{}] Setting up workspace...",
            target.name,
        )))
        .await?;

        setup_workspace(target, tx).await?;
    }

    // ── 6. Install service ───────────────────────────────────
    if opts.install_service {
        tx.send(ProgressEvent::LogLine(format!(
            "[{}] Installing service...",
            target.name,
        )))
        .await?;

        install_service(target, tx).await?;
    }

    // ── 7. Restart service ───────────────────────────────────
    if !opts.config_only {
        tx.send(ProgressEvent::LogLine(format!(
            "[{}] Restarting service...",
            target.name,
        )))
        .await?;

        let start_cmd = start_service_cmd(target);
        let _ = ssh_cmd(&ssh_target, &start_cmd).await;
    }

    // ── 8. Verify ────────────────────────────────────────────
    let version = ssh_output(
        &ssh_target,
        "/usr/local/bin/zeus --version 2>/dev/null || echo 'not found'",
    )
    .await?;
    tx.send(ProgressEvent::LogLine(format!(
        "[{}] Version: {}",
        target.name,
        version.trim(),
    )))
    .await?;

    emit_result(tx, target, "OK", start).await?;
    Ok(())
}

/// Set up ~/.zeus/ workspace on a remote host via SSH
async fn setup_workspace(target: &FleetNode, tx: &mpsc::Sender<ProgressEvent>) -> Result<()> {
    let ssh_target = target.ssh_target();

    // Create directory structure
    let mkdir_cmd = "mkdir -p ~/.zeus/workspace/memory \
                            ~/.zeus/workspace/daily \
                            ~/.zeus/sessions \
                            ~/.zeus/logs";
    let status = ssh_cmd(&ssh_target, mkdir_cmd).await?;
    if !status.success() {
        tx.send(ProgressEvent::LogLine(format!(
            "[{}] WARNING: Failed to create directories",
            target.name,
        )))
        .await?;
    }

    // Write config.toml if it doesn't exist (strip macOS-only sections for non-macOS)
    let config_content = if target.is_macos() {
        crate::config::DEFAULT_CONFIG.to_string()
    } else {
        // Remove [talos] and enable_talos for FreeBSD/Linux — AppleScript doesn't exist
        crate::config::DEFAULT_CONFIG
            .lines()
            .map(|l| {
                if l.contains("enable_talos") {
                    "enable_talos = false"
                } else {
                    l
                }
            })
            .filter(|l| {
                !l.contains("[talos]")
                    && !l.contains("enable_applescript")
                    && !l.contains("Talos (macOS Automation)")
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let config_cmd = format!(
        r#"test -f ~/.zeus/config.toml || cat > ~/.zeus/config.toml << 'ZEUS_CONFIG_EOF'
{}
ZEUS_CONFIG_EOF"#,
        config_content,
    );
    let _ = ssh_cmd(&ssh_target, &config_cmd).await;

    tx.send(ProgressEvent::LogLine(format!(
        "[{}] config.toml ready",
        target.name,
    )))
    .await?;

    // Write .env if it doesn't exist
    let env_content = crate::config::DEFAULT_ENV;
    let env_cmd = format!(
        r#"test -f ~/.zeus/.env || (cat > ~/.zeus/.env << 'ZEUS_ENV_EOF'
{}
ZEUS_ENV_EOF
chmod 600 ~/.zeus/.env)"#,
        env_content,
    );
    let _ = ssh_cmd(&ssh_target, &env_cmd).await;

    tx.send(ProgressEvent::LogLine(format!(
        "[{}] .env ready (chmod 600)",
        target.name,
    )))
    .await?;

    // Write workspace files if they don't exist
    // Write CLAUDE.md (code quality + plan discipline for Claude Code agents)
    let claude_md_cmd = format!(
        r#"test -f ~/.zeus/CLAUDE.md || cat > ~/.zeus/CLAUDE.md << 'ZEUS_CLAUDE_EOF'
{}
ZEUS_CLAUDE_EOF"#,
        crate::config::DEFAULT_CLAUDE_MD,
    );
    let _ = ssh_cmd(&ssh_target, &claude_md_cmd).await;

    tx.send(ProgressEvent::LogLine(format!(
        "[{}] CLAUDE.md installed",
        target.name,
    )))
    .await?;

    let workspace_files = [
        (
            "workspace/AGENTS.md",
            "# Agent System Prompt\n\nYou are Zeus, an AI assistant.\n",
        ),
        (
            "workspace/SOUL.md",
            "# Personality\n\nHelpful, precise, and efficient.\n",
        ),
        (
            "workspace/USER.md",
            "# User Context\n\nEdit this file to tell Zeus about yourself.\n",
        ),
        (
            "workspace/HEARTBEAT.md",
            "# Proactive Tasks\n\n- [ ] Check system health\n",
        ),
        (
            "workspace/memory/MEMORY.md",
            "# Long-term Memory\n\nFacts and context remembered across sessions.\n",
        ),
    ];

    for (path, content) in &workspace_files {
        let cmd = format!(
            r#"test -f ~/.zeus/{path} || cat > ~/.zeus/{path} << 'ZEUS_WS_EOF'
{content}
ZEUS_WS_EOF"#
        );
        let _ = ssh_cmd(&ssh_target, &cmd).await;
    }

    // Copy coordinator's fleet.conf to remote if it doesn't exist
    let local_fleet = dirs::home_dir().map(|h| h.join(".zeus/fleet.conf"));
    if let Some(ref fleet_path) = local_fleet
        && fleet_path.exists()
        && let Ok(fleet_content) = std::fs::read_to_string(fleet_path)
    {
        let fleet_cmd = format!(
            r#"test -f ~/.zeus/fleet.conf || cat > ~/.zeus/fleet.conf << 'ZEUS_FLEET_EOF'
{}
ZEUS_FLEET_EOF"#,
            fleet_content,
        );
        let _ = ssh_cmd(&ssh_target, &fleet_cmd).await;
    }

    // /usr/local/bin is already in PATH on macOS and FreeBSD by default — no PATH modification needed

    tx.send(ProgressEvent::LogLine(format!(
        "[{}] Workspace setup complete",
        target.name,
    )))
    .await?;

    Ok(())
}

/// Install gateway service on a remote host
async fn install_service(target: &FleetNode, tx: &mpsc::Sender<ProgressEvent>) -> Result<()> {
    let ssh_target = target.ssh_target();

    if target.is_macos() {
        // macOS: system LaunchDaemon (com.zeus.gateway) — survives logout/reboot,
        // no GUI session required. Matches what install.sh manages.
        let plist = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.zeus.gateway</string>
    <key>ProgramArguments</key>
    <array>
        <string>__ZEUS_BIN__</string>
        <string>gateway</string>
    </array>
    <key>EnvironmentVariables</key>
    <dict>
        <key>HOME</key>
        <string>__HOME__</string>
        <key>PATH</key>
        <string>/usr/local/bin:/usr/bin:/bin</string>
        <key>ZEUS_HOME</key>
        <string>__HOME__/.zeus</string>
    </dict>
    <key>WorkingDirectory</key>
    <string>__HOME__/.zeus</string>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>
    <key>StandardOutPath</key>
    <string>__HOME__/.zeus/logs/gateway.out.log</string>
    <key>StandardErrorPath</key>
    <string>__HOME__/.zeus/logs/gateway.err.log</string>
</dict>
</plist>"#;

        // Stop-before-replace: bootout the system daemon, kill any stale
        // processes, and let KeepAlive settle before installing the new binary.
        let stop_before = r#"sudo launchctl bootout system /Library/LaunchDaemons/com.zeus.gateway.plist 2>/dev/null || true
rm -f "$HOME/.zeus/gateway.pid"
pkill -f 'zeus gateway' 2>/dev/null || true
sleep 2
# Also remove any leftover user-agent from the old ai.zeus.gateway label
launchctl unload ~/Library/LaunchAgents/ai.zeus.gateway.plist 2>/dev/null || true
rm -f ~/Library/LaunchAgents/ai.zeus.gateway.plist"#;

        let _ = ssh_cmd(&ssh_target, stop_before).await;

        let install_cmd = format!(
            r#"ZEUS_BIN="/usr/local/bin/zeus" && \
SYS_DST="/Library/LaunchDaemons/com.zeus.gateway.plist" && \
cat > /tmp/com.zeus.gateway.plist << 'PLIST_EOF'
{}
PLIST_EOF
sed -i '' "s|__ZEUS_BIN__|$ZEUS_BIN|g; s|__HOME__|$HOME|g" /tmp/com.zeus.gateway.plist && \
sudo cp -f /tmp/com.zeus.gateway.plist "$SYS_DST" && \
sudo chmod 644 "$SYS_DST" && \
sudo chown root:wheel "$SYS_DST" && \
rm -f /tmp/com.zeus.gateway.plist"#,
            plist,
        );

        let status = ssh_cmd(&ssh_target, &install_cmd).await?;
        if status.success() {
            // Bootstrap and start the system daemon
            let _ = ssh_cmd(
                &ssh_target,
                "sudo launchctl bootstrap system /Library/LaunchDaemons/com.zeus.gateway.plist 2>/dev/null || \
                 sudo launchctl load /Library/LaunchDaemons/com.zeus.gateway.plist 2>/dev/null",
            )
            .await;
            tx.send(ProgressEvent::LogLine(format!(
                "[{}] system LaunchDaemon (com.zeus.gateway) installed and started",
                target.name,
            )))
            .await?;
        }
    } else if target.is_freebsd() {
        // FreeBSD: use the proper rc.d script from scripts/freebsd/zeus_gateway
        // which has configurable rc.conf variables and defaults to 'zeus' user.
        let rc_script = include_str!("../../../../scripts/freebsd/zeus_gateway");

        let install_cmd = format!(
            r#"sudo tee /usr/local/etc/rc.d/zeus_gateway > /dev/null << 'RCD_EOF'
{}
RCD_EOF
sudo chmod +x /usr/local/etc/rc.d/zeus_gateway && \
sudo sysrc zeus_gateway_enable="YES" && \
sudo sysrc zeus_gateway_user="{user}" && \
sudo sysrc zeus_gateway_home="/home/{user}" && \
sudo service zeus_gateway start"#,
            rc_script,
            user = target.user.as_str(),
        );

        let status = ssh_cmd(&ssh_target, &install_cmd).await?;
        if status.success() {
            tx.send(ProgressEvent::LogLine(format!(
                "[{}] rc.d service installed, enabled, and started",
                target.name,
            )))
            .await?;
        }
    } else {
        // Linux: systemd user unit (no User= needed — runs as invoking user)
        let unit = r#"[Unit]
Description=Zeus Gateway
After=network.target

[Service]
Type=simple
ExecStart=/usr/local/bin/zeus gateway
Environment=ZEUS_HOME=%h/.zeus
Restart=always
RestartSec=5

[Install]
WantedBy=default.target"#;

        let install_cmd = format!(
            r#"mkdir -p ~/.config/systemd/user && \
cat > ~/.config/systemd/user/zeus-gateway.service << 'UNIT_EOF'
{}
UNIT_EOF
systemctl --user daemon-reload && \
systemctl --user enable zeus-gateway"#,
            unit,
        );

        let status = ssh_cmd(&ssh_target, &install_cmd).await?;
        if status.success() {
            tx.send(ProgressEvent::LogLine(format!(
                "[{}] systemd user service installed, enabled, and started",
                target.name,
            )))
            .await?;
        }
    }

    Ok(())
}

/// Get the stop-service command for a target
fn stop_service_cmd(target: &FleetNode) -> String {
    if target.is_macos() {
        // Stop the system LaunchDaemon (com.zeus.gateway) and clean up stale PID
        "sudo launchctl bootout system /Library/LaunchDaemons/com.zeus.gateway.plist 2>/dev/null || true; \
         rm -f ~/.zeus/gateway.pid; \
         pkill -f 'zeus gateway' 2>/dev/null || true".into()
    } else if target.is_freebsd() {
        "sudo service zeus_gateway stop 2>/dev/null || true".into()
    } else {
        "systemctl --user stop zeus-gateway 2>/dev/null || true".into()
    }
}

/// Get the start-service command for a target
fn start_service_cmd(target: &FleetNode) -> String {
    if target.is_macos() {
        // Bootstrap the system LaunchDaemon (com.zeus.gateway)
        "sudo launchctl bootstrap system /Library/LaunchDaemons/com.zeus.gateway.plist 2>/dev/null || \
         sudo launchctl load /Library/LaunchDaemons/com.zeus.gateway.plist 2>/dev/null || true".into()
    } else if target.is_freebsd() {
        "sudo service zeus_gateway start 2>/dev/null || true".into()
    } else {
        "systemctl --user start zeus-gateway 2>/dev/null || true".into()
    }
}

// ── SSH helpers ──────────────────────────────────────────────

/// Run an SSH command, return its exit status
async fn ssh_cmd(target: &str, cmd: &str) -> Result<std::process::ExitStatus> {
    Ok(tokio::process::Command::new("ssh")
        .args(SSH_OPTS)
        .arg(target)
        .arg(cmd)
        .status()
        .await?)
}

/// Run an SSH command and capture stdout
async fn ssh_output(target: &str, cmd: &str) -> Result<String> {
    let output = tokio::process::Command::new("ssh")
        .args(SSH_OPTS)
        .arg(target)
        .arg(cmd)
        .output()
        .await?;
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Emit a DeployResult event
async fn emit_result(
    tx: &mpsc::Sender<ProgressEvent>,
    target: &FleetNode,
    status: &str,
    start: Instant,
) -> Result<()> {
    tx.send(ProgressEvent::DeployResult {
        host: target.name.clone(),
        ip: target.ip.clone(),
        os: target.os.clone(),
        status: status.into(),
        duration: start.elapsed(),
    })
    .await?;
    Ok(())
}
