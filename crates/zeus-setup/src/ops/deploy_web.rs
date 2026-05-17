//! Deploy web frontend to .226 FreeBSD

use crate::event::ProgressEvent;
use anyhow::Result;
use tokio::sync::mpsc;

pub async fn run(
    branch: Option<String>,
    skip_pull: bool,
    tx: mpsc::Sender<ProgressEvent>,
) -> Result<()> {
    let start = std::time::Instant::now();
    let total = 5;

    // Step 1: Connect to .226
    tx.send(ProgressEvent::StepStarted {
        name: "Connect to .226".into(),
        index: 0,
        total,
    })
    .await?;

    let ssh_opts = ["-o", "StrictHostKeyChecking=no", "-o", "ConnectTimeout=10"];
    let host_str = std::env::var("ZEUS_DEPLOY_WEB_HOST").map_err(|_| {
        anyhow::anyhow!("ZEUS_DEPLOY_WEB_HOST env var not set (example: user@host.example)")
    })?;
    let host = host_str.as_str();

    let status = tokio::process::Command::new("ssh")
        .args(ssh_opts)
        .arg(host)
        .args(["echo", "ok"])
        .output()
        .await?;

    if !status.status.success() {
        tx.send(ProgressEvent::StepFailed {
            name: "Connect to .226".into(),
            error: "SSH connection failed".into(),
        })
        .await?;
        anyhow::bail!("Cannot connect to .226");
    }

    tx.send(ProgressEvent::StepCompleted {
        name: "Connect to .226".into(),
        message: "Connected".into(),
    })
    .await?;

    // Step 2: Git checkout/pull
    tx.send(ProgressEvent::StepStarted {
        name: "Update source".into(),
        index: 1,
        total,
    })
    .await?;

    if !skip_pull {
        let branch_cmd = if let Some(ref b) = branch {
            format!("cd ~/Zeus && git checkout {} && git pull", b)
        } else {
            "cd ~/Zeus && git pull".into()
        };

        let _ = tokio::process::Command::new("ssh")
            .args(ssh_opts)
            .arg(host)
            .arg(&branch_cmd)
            .status()
            .await?;
    }

    tx.send(ProgressEvent::StepCompleted {
        name: "Update source".into(),
        message: if skip_pull { "Skipped" } else { "Updated" }.into(),
    })
    .await?;

    // Step 3: Build web frontend
    tx.send(ProgressEvent::StepStarted {
        name: "Build web frontend".into(),
        index: 2,
        total,
    })
    .await?;

    let build_status = tokio::process::Command::new("ssh")
        .args(ssh_opts)
        .arg(host)
        .arg("cd ~/Zeus/apps/ZeusWeb && trunk build --release")
        .status()
        .await?;

    if !build_status.success() {
        tx.send(ProgressEvent::StepFailed {
            name: "Build web frontend".into(),
            error: "trunk build failed".into(),
        })
        .await?;
        anyhow::bail!("trunk build failed on .226");
    }

    tx.send(ProgressEvent::StepCompleted {
        name: "Build web frontend".into(),
        message: "Built".into(),
    })
    .await?;

    // Step 4: Deploy to nginx
    tx.send(ProgressEvent::StepStarted {
        name: "Deploy to nginx".into(),
        index: 3,
        total,
    })
    .await?;

    let deploy_cmd = "sudo rm -rf /usr/local/www/zeus/*.wasm /usr/local/www/zeus/*.js && \
                      sudo cp -r ~/Zeus/apps/ZeusWeb/dist/* /usr/local/www/zeus/";

    let _ = tokio::process::Command::new("ssh")
        .args(ssh_opts)
        .arg(host)
        .arg(deploy_cmd)
        .status()
        .await?;

    tx.send(ProgressEvent::StepCompleted {
        name: "Deploy to nginx".into(),
        message: "/usr/local/www/zeus/".into(),
    })
    .await?;

    // Step 5: Verify
    tx.send(ProgressEvent::StepStarted {
        name: "Verify deployment".into(),
        index: 4,
        total,
    })
    .await?;

    let verify = tokio::process::Command::new("ssh")
        .args(ssh_opts)
        .arg(host)
        .arg("ls -la /usr/local/www/zeus/index.html")
        .output()
        .await?;

    if verify.status.success() {
        tx.send(ProgressEvent::StepCompleted {
            name: "Verify deployment".into(),
            message: "index.html present".into(),
        })
        .await?;
    } else {
        tx.send(ProgressEvent::StepWarning {
            name: "Verify deployment".into(),
            message: "Could not verify index.html".into(),
        })
        .await?;
    }

    tx.send(ProgressEvent::Finished {
        success: true,
        elapsed: start.elapsed(),
        summary: "Web frontend deployed to .226".into(),
    })
    .await?;

    Ok(())
}
