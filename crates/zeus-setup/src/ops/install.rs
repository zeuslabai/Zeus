//! Install Zeus locally — download, build from source, or local binary

use crate::event::ProgressEvent;
use anyhow::Result;
use std::path::PathBuf;
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub enum InstallMode {
    Download,
    Source,
    Local(PathBuf),
    Auto,
}

pub async fn run(
    mode: InstallMode,
    prefix: PathBuf,
    _version: Option<String>,
    reconfigure: bool,
    tx: mpsc::Sender<ProgressEvent>,
) -> Result<()> {
    let start = std::time::Instant::now();
    let total_steps = 7;

    // Step 1: Detect platform
    tx.send(ProgressEvent::StepStarted {
        name: "Detect platform".into(),
        index: 0,
        total: total_steps,
    })
    .await?;

    let platform = crate::platform::Platform::detect()?;
    tx.send(ProgressEvent::StepCompleted {
        name: "Detect platform".into(),
        message: format!("{}", platform),
    })
    .await?;

    // Step 2: Resolve binary
    tx.send(ProgressEvent::StepStarted {
        name: "Resolve binary".into(),
        index: 1,
        total: total_steps,
    })
    .await?;

    let binary_path = match &mode {
        InstallMode::Source => {
            tx.send(ProgressEvent::LogLine("Building from source...".into()))
                .await?;
            build_from_source(&tx).await?
        }
        InstallMode::Local(path) => {
            if !path.exists() {
                tx.send(ProgressEvent::StepFailed {
                    name: "Resolve binary".into(),
                    error: format!("File not found: {}", path.display()),
                })
                .await?;
                anyhow::bail!("Local binary not found: {}", path.display());
            }
            path.clone()
        }
        InstallMode::Download => {
            tx.send(ProgressEvent::LogLine(
                "Downloading latest release from GitHub...".into(),
            ))
            .await?;
            download_release(&platform, &tx).await?
        }
        InstallMode::Auto => {
            // Check for local build first
            let local = PathBuf::from("target/release/zeus");
            if local.exists() {
                tx.send(ProgressEvent::LogLine(
                    "Found local build at target/release/zeus".into(),
                ))
                .await?;
                local
            } else {
                tx.send(ProgressEvent::LogLine("Building from source...".into()))
                    .await?;
                build_from_source(&tx).await?
            }
        }
    };

    tx.send(ProgressEvent::StepCompleted {
        name: "Resolve binary".into(),
        message: format!("{}", binary_path.display()),
    })
    .await?;

    // Step 3: Install binary
    let install_path = prefix.join("bin/zeus");
    tx.send(ProgressEvent::StepStarted {
        name: "Install binary".into(),
        index: 2,
        total: total_steps,
    })
    .await?;

    // Ensure directory exists
    if let Some(parent) = install_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Copy binary to /usr/local/bin
    std::fs::copy(&binary_path, &install_path)
        .map_err(|e| anyhow::anyhow!("Failed to copy to {}: {}", install_path.display(), e))?;

    // Make executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&install_path, std::fs::Permissions::from_mode(0o755))?;
    }

    tx.send(ProgressEvent::StepCompleted {
        name: "Install binary".into(),
        message: format!("{}", install_path.display()),
    })
    .await?;

    // Step 4: Codesign (macOS only)
    tx.send(ProgressEvent::StepStarted {
        name: "Codesign".into(),
        index: 3,
        total: total_steps,
    })
    .await?;

    if platform.is_macos() {
        let _ = tokio::process::Command::new("codesign")
            .args(["--force", "--sign", "-"])
            .arg(&install_path)
            .status()
            .await;
        tx.send(ProgressEvent::StepCompleted {
            name: "Codesign".into(),
            message: "Ad-hoc signed".into(),
        })
        .await?;
    } else {
        tx.send(ProgressEvent::StepCompleted {
            name: "Codesign".into(),
            message: "Skipped (not macOS)".into(),
        })
        .await?;
    }

    // Step 5: Backup existing config + setup workspace
    tx.send(ProgressEvent::StepStarted {
        name: "Setup workspace".into(),
        index: 4,
        total: total_steps,
    })
    .await?;

    // Backup ~/.zeus/config.toml → ~/.zeus/config.toml.pre-install (always, if it exists)
    let zeus_dir = crate::config::zeus_home();
    let config_path = zeus_dir.join("config.toml");
    let backup_path = zeus_dir.join("config.toml.pre-install");
    if config_path.exists() {
        match std::fs::copy(&config_path, &backup_path) {
            Ok(_) => {
                tx.send(ProgressEvent::LogLine(format!(
                    "Backed up config.toml → {}",
                    backup_path.display()
                )))
                .await?;
            }
            Err(e) => {
                // Non-fatal: log and continue
                tx.send(ProgressEvent::LogLine(format!(
                    "Warning: could not back up config.toml: {}",
                    e
                )))
                .await?;
            }
        }

        // If --reconfigure: remove existing config so initialize_workspace writes a fresh one
        if reconfigure {
            if let Err(e) = std::fs::remove_file(&config_path) {
                tx.send(ProgressEvent::LogLine(format!(
                    "Warning: could not remove old config for reconfigure: {}",
                    e
                )))
                .await?;
            } else {
                tx.send(ProgressEvent::LogLine(
                    "Removed old config.toml (--reconfigure)".into(),
                ))
                .await?;
            }
        }
    }

    crate::config::initialize_workspace()?;

    tx.send(ProgressEvent::StepCompleted {
        name: "Setup workspace".into(),
        message: "~/.zeus/ initialized".into(),
    })
    .await?;

    // Step 6: Verify
    tx.send(ProgressEvent::StepStarted {
        name: "Verify installation".into(),
        index: 5,
        total: total_steps,
    })
    .await?;

    let output = tokio::process::Command::new(&install_path)
        .arg("--version")
        .output()
        .await?;

    let zeus_version = if output.status.success() {
        let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
        tx.send(ProgressEvent::StepCompleted {
            name: "Verify installation".into(),
            message: version.clone(),
        })
        .await?;
        version
    } else {
        tx.send(ProgressEvent::StepFailed {
            name: "Verify installation".into(),
            error: "zeus --version failed".into(),
        })
        .await?;
        "unknown".to_string()
    };

    // Step 7: Send first message via gateway (if healthy)
    tx.send(ProgressEvent::StepStarted {
        name: "First message".into(),
        index: 6,
        total: total_steps,
    })
    .await?;

    send_first_message(&zeus_version, &tx).await;

    tx.send(ProgressEvent::Finished {
        success: true,
        elapsed: start.elapsed(),
        summary: format!("Zeus installed to {}", install_path.display()),
    })
    .await?;

    Ok(())
}

/// Attempt to send a "first message" via the Zeus gateway.
/// Non-fatal: logs the result but never fails the install.
async fn send_first_message(zeus_version: &str, tx: &mpsc::Sender<ProgressEvent>) {
    const GATEWAY_HEALTH: &str = "http://127.0.0.1:8080/health";
    const GATEWAY_SEND: &str = "http://127.0.0.1:8080/v1/channels/send";
    const HEALTH_TIMEOUT_SECS: u64 = 5;

    // Determine hostname for the message
    let hostname = tokio::process::Command::new("hostname")
        .output()
        .await
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "zeus".to_string());

    let message = format!("{} online. {} Ready.", hostname, zeus_version);

    // Check if gateway is reachable
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(HEALTH_TIMEOUT_SECS))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            let _ = tx
                .send(ProgressEvent::LogLine(format!(
                    "First message skipped (http client error: {})",
                    e
                )))
                .await;
            let _ = tx
                .send(ProgressEvent::StepCompleted {
                    name: "First message".into(),
                    message: "Skipped (gateway not reachable)".into(),
                })
                .await;
            return;
        }
    };

    let gateway_healthy = client.get(GATEWAY_HEALTH).send().await.map_or(false, |r| r.status().is_success());

    if !gateway_healthy {
        let _ = tx
            .send(ProgressEvent::LogLine(
                "Gateway not running — first message skipped (start with `zeus gateway`)".into(),
            ))
            .await;
        let _ = tx
            .send(ProgressEvent::StepCompleted {
                name: "First message".into(),
                message: "Skipped (gateway offline)".into(),
            })
            .await;
        return;
    }

    // POST the first message
    let body = serde_json::json!({
        "channel": "discord",
        "message": message,
    });

    match client.post(GATEWAY_SEND).json(&body).send().await {
        Ok(resp) if resp.status().is_success() => {
            let _ = tx
                .send(ProgressEvent::StepCompleted {
                    name: "First message".into(),
                    message: format!("Sent: {}", message),
                })
                .await;
        }
        Ok(resp) => {
            let _ = tx
                .send(ProgressEvent::LogLine(format!(
                    "First message: gateway returned {}",
                    resp.status()
                )))
                .await;
            let _ = tx
                .send(ProgressEvent::StepCompleted {
                    name: "First message".into(),
                    message: "Sent (non-200 response)".into(),
                })
                .await;
        }
        Err(e) => {
            let _ = tx
                .send(ProgressEvent::LogLine(format!(
                    "First message failed: {}",
                    e
                )))
                .await;
            let _ = tx
                .send(ProgressEvent::StepCompleted {
                    name: "First message".into(),
                    message: "Skipped (send error)".into(),
                })
                .await;
        }
    }
}

async fn download_release(
    platform: &crate::platform::Platform,
    tx: &mpsc::Sender<ProgressEvent>,
) -> Result<PathBuf> {
    const REPO: &str = "zeuslabai/Zeus";
    let api_url = format!("https://api.github.com/repos/{REPO}/releases/latest");

    tx.send(ProgressEvent::LogLine(format!(
        "Querying GitHub API for latest release of {REPO}..."
    )))
    .await?;

    let client = reqwest::Client::builder()
        .user_agent("zeus-setup/0.1.0")
        .build()?;

    let release: serde_json::Value = client
        .get(&api_url)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to query GitHub releases: {e}"))?
        .error_for_status()
        .map_err(|e| {
            if e.status() == Some(reqwest::StatusCode::NOT_FOUND) {
                anyhow::anyhow!(
                    "No releases found for {REPO}. Publish a release first, or use --source to build locally."
                )
            } else {
                anyhow::anyhow!("GitHub API error: {e}")
            }
        })?
        .json()
        .await?;

    let tag = release["tag_name"].as_str().unwrap_or("unknown");
    tx.send(ProgressEvent::LogLine(format!("Latest release: {tag}")))
        .await?;

    // Find matching asset by platform triple
    // Expected naming: zeus-{triple} or zeus-{triple}.tar.gz
    let triple = &platform.triple;
    let asset_patterns: Vec<String> = vec![
        format!("zeus-{triple}"),
        format!("zeus-{triple}.tar.gz"),
        format!("zeus-{triple}.zip"),
    ];

    let assets = release["assets"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("No assets in release {tag}"))?;

    let asset = assets
        .iter()
        .find(|a| {
            let name = a["name"].as_str().unwrap_or("");
            asset_patterns.iter().any(|p| name == p.as_str())
        })
        .ok_or_else(|| {
            let available: Vec<&str> = assets.iter().filter_map(|a| a["name"].as_str()).collect();
            anyhow::anyhow!(
                "No asset found for platform {triple} in release {tag}.\nAvailable assets: {}",
                available.join(", ")
            )
        })?;

    let asset_name = asset["name"].as_str().unwrap_or("zeus");
    let download_url = asset["browser_download_url"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("No download URL for asset {asset_name}"))?;

    tx.send(ProgressEvent::LogLine(format!(
        "Downloading {asset_name}..."
    )))
    .await?;

    let response = client
        .get(download_url)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Download failed: {e}"))?
        .error_for_status()?;

    let bytes = response.bytes().await?;
    let total_mb = bytes.len() as f64 / 1_048_576.0;
    tx.send(ProgressEvent::LogLine(format!(
        "Downloaded {total_mb:.1} MB"
    )))
    .await?;

    // Write to temp directory
    let tmp_dir = std::env::temp_dir().join("zeus-setup");
    std::fs::create_dir_all(&tmp_dir)?;
    let binary_path = tmp_dir.join("zeus");

    if asset_name.ends_with(".tar.gz") {
        // Extract from tarball
        tx.send(ProgressEvent::LogLine("Extracting tarball...".into()))
            .await?;
        let tar_path = tmp_dir.join(asset_name);
        std::fs::write(&tar_path, &bytes)?;
        let status = tokio::process::Command::new("tar")
            .args([
                "xzf",
                &tar_path.to_string_lossy(),
                "-C",
                &tmp_dir.to_string_lossy(),
            ])
            .status()
            .await?;
        if !status.success() {
            anyhow::bail!("Failed to extract tarball");
        }
        // Look for the zeus binary inside the extracted files
        if !binary_path.exists() {
            // Try finding it recursively
            let mut found = None;
            for entry in walkdir(&tmp_dir) {
                if entry.file_name().map(|n| n == "zeus").unwrap_or(false) && entry.is_file() {
                    found = Some(entry.clone());
                    break;
                }
            }
            if let Some(found) = found {
                std::fs::rename(&found, &binary_path)?;
            } else {
                anyhow::bail!("zeus binary not found in tarball");
            }
        }
    } else if asset_name.ends_with(".zip") {
        // Extract from zip
        tx.send(ProgressEvent::LogLine("Extracting zip...".into()))
            .await?;
        let zip_path = tmp_dir.join(asset_name);
        std::fs::write(&zip_path, &bytes)?;
        let status = tokio::process::Command::new("unzip")
            .args([
                "-o",
                &zip_path.to_string_lossy(),
                "-d",
                &tmp_dir.to_string_lossy(),
            ])
            .status()
            .await?;
        if !status.success() {
            anyhow::bail!("Failed to extract zip");
        }
        if !binary_path.exists() {
            let mut found = None;
            for entry in walkdir(&tmp_dir) {
                if entry.file_name().map(|n| n == "zeus").unwrap_or(false) && entry.is_file() {
                    found = Some(entry.clone());
                    break;
                }
            }
            if let Some(found) = found {
                std::fs::rename(&found, &binary_path)?;
            } else {
                anyhow::bail!("zeus binary not found in zip");
            }
        }
    } else {
        // Raw binary
        std::fs::write(&binary_path, &bytes)?;
    }

    // Make executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&binary_path, std::fs::Permissions::from_mode(0o755))?;
    }

    tx.send(ProgressEvent::LogLine(format!(
        "Binary ready at {}",
        binary_path.display()
    )))
    .await?;

    Ok(binary_path)
}

/// Simple recursive directory walk (avoids adding walkdir dependency)
fn walkdir(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut results = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                results.extend(walkdir(&path));
            } else {
                results.push(path);
            }
        }
    }
    results
}

async fn build_from_source(tx: &mpsc::Sender<ProgressEvent>) -> Result<PathBuf> {
    tx.send(ProgressEvent::LogLine(
        "cargo build --release --bin zeus".into(),
    ))
    .await?;

    let mut child = tokio::process::Command::new("cargo")
        .args(["build", "--release", "--bin", "zeus"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;

    // Stream stderr (cargo outputs there)
    if let Some(stderr) = child.stderr.take() {
        let tx = tx.clone();
        tokio::spawn(async move {
            use tokio::io::{AsyncBufReadExt, BufReader};
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = tx.send(ProgressEvent::LogLine(line)).await;
            }
        });
    }

    let status = child.wait().await?;
    if !status.success() {
        anyhow::bail!("cargo build failed");
    }

    Ok(PathBuf::from("target/release/zeus"))
}
