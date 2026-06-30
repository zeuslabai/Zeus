//! Gateway service management — launchd (macOS), systemd (Linux), rc.d (FreeBSD)

use crate::event::ProgressEvent;
use crate::platform::{Os, Platform};
use anyhow::Result;
use std::path::PathBuf;
use tokio::sync::mpsc;

pub async fn run(action: &str, tx: mpsc::Sender<ProgressEvent>) -> Result<()> {
    let start = std::time::Instant::now();
    let platform = Platform::detect()?;

    tx.send(ProgressEvent::StepStarted {
        name: format!("Service {}", action),
        index: 0,
        total: 1,
    })
    .await?;

    let result = match action {
        "install" => install_service(&platform, &tx).await,
        "start" => control_service(&platform, "start").await,
        "stop" => control_service(&platform, "stop").await,
        "restart" => {
            control_service(&platform, "stop").await?;
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            control_service(&platform, "start").await
        }
        "status" => show_status(&platform, &tx).await,
        "uninstall" => uninstall_service(&platform).await,
        _ => anyhow::bail!("Unknown service action: {}", action),
    };

    match &result {
        Ok(()) => {
            tx.send(ProgressEvent::StepCompleted {
                name: format!("Service {}", action),
                message: "Done".into(),
            })
            .await?;
        }
        Err(e) => {
            tx.send(ProgressEvent::StepFailed {
                name: format!("Service {}", action),
                error: format!("{}", e),
            })
            .await?;
        }
    }

    tx.send(ProgressEvent::Finished {
        success: result.is_ok(),
        elapsed: start.elapsed(),
        summary: format!(
            "Service {} {}",
            action,
            if result.is_ok() {
                "completed"
            } else {
                "failed"
            }
        ),
    })
    .await?;

    result
}

async fn install_service(platform: &Platform, tx: &mpsc::Sender<ProgressEvent>) -> Result<()> {
    let zeus_bin = crate::config::zeus_bin();
    if !zeus_bin.exists() {
        anyhow::bail!(
            "Zeus binary not found at {} — install first",
            zeus_bin.display()
        );
    }

    match platform.os {
        Os::MacOS => {
            let plist_path = dirs::home_dir()
                .unwrap()
                .join("Library/LaunchAgents/ai.zeus.gateway.plist");
            let plist = format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>ai.zeus.gateway</string>
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
    <string>{}/.zeus/logs/gateway.out.log</string>
    <key>StandardErrorPath</key>
    <string>{}/.zeus/logs/gateway.err.log</string>
</dict>
</plist>"#,
                zeus_bin.display(),
                dirs::home_dir().unwrap().display(),
                dirs::home_dir().unwrap().display(),
            );
            // Ensure ~/.zeus/logs/ exists before launchd loads the plist — without it,
            // StandardOutPath/StandardErrorPath redirects silently drop to /dev/null.
            if let Some(home) = dirs::home_dir() {
                std::fs::create_dir_all(home.join(".zeus").join("logs")).ok();
            }
            std::fs::write(&plist_path, plist)?;
            tx.send(ProgressEvent::LogLine(format!(
                "Created {}",
                plist_path.display()
            )))
            .await?;
        }
        Os::Linux => {
            // Use the proper hardened systemd unit from scripts/systemd/zeus-gateway.service
            // as the source of truth, substituting the actual binary path.
            let script_unit = include_str!("../../../../scripts/systemd/zeus-gateway.service");
            let unit = script_unit.replace(
                "ExecStart=/usr/local/bin/zeus gateway",
                &format!("ExecStart={} gateway", zeus_bin.display()),
            );

            let unit_path = PathBuf::from("/etc/systemd/system/zeus-gateway.service");

            // Write the unit content to stdin of tee
            if let Ok(mut child) = tokio::process::Command::new("sudo")
                .args(["tee", &unit_path.to_string_lossy()])
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .spawn()
            {
                use tokio::io::AsyncWriteExt;
                if let Some(ref mut stdin) = child.stdin {
                    let _ = stdin.write_all(unit.as_bytes()).await;
                }
                let _ = child.wait().await;
            } else {
                // Fallback: user service
                let user_dir = dirs::home_dir().unwrap().join(".config/systemd/user");
                std::fs::create_dir_all(&user_dir)?;
                std::fs::write(user_dir.join("zeus-gateway.service"), &unit)?;
                tx.send(ProgressEvent::LogLine(
                    "Installed as user service (no sudo)".into(),
                ))
                .await?;
            }

            let _ = tokio::process::Command::new("sudo")
                .args(["systemctl", "daemon-reload"])
                .status()
                .await;
            let _ = tokio::process::Command::new("sudo")
                .args(["systemctl", "enable", "zeus-gateway"])
                .status()
                .await;

            tx.send(ProgressEvent::LogLine(format!(
                "Installed systemd unit at {}",
                unit_path.display()
            )))
            .await?;
        }
        Os::FreeBSD => {
            // Install the proper rc.d script from scripts/freebsd/zeus_gateway
            let rc_d_script = include_str!("../../../../scripts/freebsd/zeus_gateway");
            let rc_d_path = PathBuf::from("/usr/local/etc/rc.d/zeus_gateway");

            // Ensure rc.d directory exists
            let _ = tokio::process::Command::new("sudo")
                .args(["mkdir", "-p", "/usr/local/etc/rc.d"])
                .status()
                .await;

            // Write the rc.d script
            if let Ok(mut child) = tokio::process::Command::new("sudo")
                .args(["tee", &rc_d_path.to_string_lossy()])
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .spawn()
            {
                use tokio::io::AsyncWriteExt;
                if let Some(ref mut stdin) = child.stdin {
                    let _ = stdin.write_all(rc_d_script.as_bytes()).await;
                }
                let _ = child.wait().await;
            }

            // Make it executable
            let _ = tokio::process::Command::new("sudo")
                .args(["chmod", "+x", &rc_d_path.to_string_lossy()])
                .status()
                .await;

            // Enable the service and bind it to the operator's HOME/ZEUS_HOME.
            // The gateway writes its lock PID to ${ZEUS_HOME}/gateway.pid; rc.d
            // must track that same path for truthful service status.
            let sudo_user = std::env::var("SUDO_USER").ok().filter(|u| !u.is_empty());
            let service_user = sudo_user
                .clone()
                .or_else(|| std::env::var("USER").ok())
                .unwrap_or_else(|| "zeus".to_string());
            let service_home = if sudo_user.is_some() && service_user != "root" {
                format!("/home/{service_user}")
            } else {
                std::env::var("HOME")
                    .ok()
                    .filter(|h| !h.is_empty())
                    .unwrap_or_else(|| format!("/home/{service_user}"))
            };
            let _ = tokio::process::Command::new("sudo")
                .args(["sysrc", "zeus_gateway_enable=YES"])
                .status()
                .await;
            let _ = tokio::process::Command::new("sudo")
                .args(["sysrc", &format!("zeus_gateway_user={service_user}")])
                .status()
                .await;
            let _ = tokio::process::Command::new("sudo")
                .args(["sysrc", &format!("zeus_gateway_home={service_home}")])
                .status()
                .await;

            // Start it now so onboarding leaves the gateway actually running.
            let status = tokio::process::Command::new("sudo")
                .args(["service", "zeus_gateway", "start"])
                .status()
                .await?;
            if !status.success() {
                anyhow::bail!("Failed to start FreeBSD rc.d service zeus_gateway");
            }

            tx.send(ProgressEvent::LogLine(format!(
                "Installed, enabled, and started rc.d script at {}",
                rc_d_path.display()
            )))
            .await?;
        }
    }

    Ok(())
}

async fn control_service(platform: &Platform, action: &str) -> Result<()> {
    let status = match platform.os {
        Os::MacOS => {
            let launchctl_action = match action {
                "start" => "load",
                "stop" => "unload",
                _ => action,
            };
            let plist = dirs::home_dir()
                .unwrap()
                .join("Library/LaunchAgents/ai.zeus.gateway.plist");
            tokio::process::Command::new("launchctl")
                .args([launchctl_action, &plist.to_string_lossy()])
                .status()
                .await?
        }
        Os::Linux => {
            tokio::process::Command::new("sudo")
                .args(["systemctl", action, "zeus-gateway"])
                .status()
                .await?
        }
        Os::FreeBSD => {
            tokio::process::Command::new("sudo")
                .args(["service", "zeus_gateway", action])
                .status()
                .await?
        }
    };

    if !status.success() {
        anyhow::bail!("Service {} failed", action);
    }

    Ok(())
}

async fn show_status(platform: &Platform, tx: &mpsc::Sender<ProgressEvent>) -> Result<()> {
    let output = match platform.os {
        Os::MacOS => {
            tokio::process::Command::new("launchctl")
                .args(["list", "ai.zeus.gateway"])
                .output()
                .await?
        }
        Os::Linux => {
            tokio::process::Command::new("systemctl")
                .args(["status", "zeus-gateway"])
                .output()
                .await?
        }
        Os::FreeBSD => {
            tokio::process::Command::new("service")
                .args(["zeus_gateway", "status"])
                .output()
                .await?
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        tx.send(ProgressEvent::LogLine(line.to_string())).await?;
    }

    Ok(())
}

async fn uninstall_service(platform: &Platform) -> Result<()> {
    // Stop first
    let _ = control_service(platform, "stop").await;

    match platform.os {
        Os::MacOS => {
            let plist = dirs::home_dir()
                .unwrap()
                .join("Library/LaunchAgents/ai.zeus.gateway.plist");
            if plist.exists() {
                std::fs::remove_file(&plist)?;
            }
        }
        Os::Linux => {
            let _ = tokio::process::Command::new("sudo")
                .args(["systemctl", "disable", "zeus-gateway"])
                .status()
                .await;
            let unit = PathBuf::from("/etc/systemd/system/zeus-gateway.service");
            if unit.exists() {
                let _ = tokio::process::Command::new("sudo")
                    .args(["rm", "-f", &unit.to_string_lossy()])
                    .status()
                    .await;
            }
            let _ = tokio::process::Command::new("sudo")
                .args(["systemctl", "daemon-reload"])
                .status()
                .await;
        }
        Os::FreeBSD => {
            let _ = tokio::process::Command::new("sudo")
                .args(["sysrc", "-x", "zeus_gateway_enable"])
                .status()
                .await;
        }
    }

    Ok(())
}
