//! Shared utilities for build and package operations

use crate::event::ProgressEvent;
use anyhow::Result;
use std::path::Path;
use tokio::sync::mpsc;

/// Run an external command, streaming stdout/stderr as ProgressEvent::LogLine.
pub async fn run_command(
    tx: &mpsc::Sender<ProgressEvent>,
    cmd: &str,
    args: &[&str],
    cwd: &Path,
) -> Result<()> {
    tx.send(ProgressEvent::LogLine(format!(
        "$ {} {}",
        cmd,
        args.join(" ")
    )))
    .await?;

    let mut child = tokio::process::Command::new(cmd)
        .args(args)
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;

    // Stream stderr
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

    // Stream stdout
    if let Some(stdout) = child.stdout.take() {
        let tx = tx.clone();
        tokio::spawn(async move {
            use tokio::io::{AsyncBufReadExt, BufReader};
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = tx.send(ProgressEvent::LogLine(line)).await;
            }
        });
    }

    let status = child.wait().await?;
    if !status.success() {
        anyhow::bail!("{} exited with {}", cmd, status);
    }

    Ok(())
}

/// Run a command and capture stdout as a String (no streaming).
pub async fn run_command_capture(cmd: &str, args: &[&str], cwd: &Path) -> Result<String> {
    let output = tokio::process::Command::new(cmd)
        .args(args)
        .current_dir(cwd)
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("{} exited with {}: {}", cmd, output.status, stderr.trim());
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
