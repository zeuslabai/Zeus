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
            let unit = format!(
                r#"[Unit]
Description=Zeus Gateway
After=network.target

[Service]
ExecStart={} gateway
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
"#,
                zeus_bin.display()
            );

            let unit_path = PathBuf::from("/etc/systemd/system/zeus-gateway.service");
            let status = tokio::process::Command::new("sudo")
                .args(["tee", &unit_path.to_string_lossy()])
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .spawn()?
                .wait()
                .await;

            if status.is_err() {
                // Try user service
                let user_dir = dirs::home_dir().unwrap().join(".config/systemd/user");
                std::fs::create_dir_all(&user_dir)?;
                std::fs::write(user_dir.join("zeus-gateway.service"), &unit)?;
            }

            let _ = tokio::process::Command::new("sudo")
                .args(["systemctl", "daemon-reload"])
                .status()
                .await;
            let _ = tokio::process::Command::new("sudo")
                .args(["systemctl", "enable", "zeus-gateway"])
                .status()
                .await;
        }
        Os::FreeBSD => {
            tx.send(ProgressEvent::LogLine(
                "FreeBSD rc.d: use scripts/freebsd/zeus_gateway".into(),
            ))
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
