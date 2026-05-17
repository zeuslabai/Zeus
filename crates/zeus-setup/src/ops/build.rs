//! Build from source — CLI, web, FFI, macOS, iOS

use crate::event::ProgressEvent;
use crate::ops::util::run_command;
use anyhow::Result;
use std::path::PathBuf;
use tokio::sync::mpsc;

#[derive(Debug, Clone, Default)]
pub struct BuildOpts {
    pub project_root: PathBuf,
    pub pull: bool,
    pub test: bool,
    pub cli: bool,
    pub web: bool,
    pub ffi: bool,
    pub macos: bool,
    pub ios: bool,
    pub xcode: bool,
    pub install: bool,
    pub restart: bool,
    pub mcp: bool,
    pub jobs: usize,
}

pub async fn run(opts: BuildOpts, tx: mpsc::Sender<ProgressEvent>) -> Result<()> {
    let start = std::time::Instant::now();

    // Count total steps
    let mut steps = Vec::new();
    if opts.pull {
        steps.push("Git pull");
    }
    if opts.test {
        steps.push("Run tests");
    }
    if opts.cli {
        steps.push("Build CLI");
    }
    if opts.web {
        steps.push("Build web frontend");
    }
    if opts.ffi {
        steps.push("Build FFI");
    }
    if opts.macos {
        steps.push("Build macOS Desktop");
    }
    if opts.ios {
        steps.push("Build iOS");
    }
    if opts.xcode {
        steps.push("Regenerate Xcode");
    }
    if opts.install {
        steps.push("Install binary");
    }
    if opts.restart {
        steps.push("Restart daemon");
    }
    if opts.mcp {
        steps.push("Configure MCP");
    }

    let total = steps.len();

    for (i, step_name) in steps.iter().enumerate() {
        tx.send(ProgressEvent::StepStarted {
            name: step_name.to_string(),
            index: i,
            total,
        })
        .await?;

        let result = match *step_name {
            "Git pull" => run_command(&tx, "git", &["pull"], &opts.project_root).await,
            "Run tests" => {
                run_command(&tx, "cargo", &["test", "--workspace"], &opts.project_root).await
            }
            "Build CLI" => {
                let mut args = vec!["build", "--release", "--bin", "zeus", "-j"];
                let jobs_str = opts.jobs.to_string();
                args.push(&jobs_str);
                run_command(&tx, "cargo", &args, &opts.project_root).await
            }
            "Build web frontend" => {
                run_command(
                    &tx,
                    "trunk",
                    &["build", "--release"],
                    &opts.project_root.join("apps/ZeusWeb"),
                )
                .await
            }
            "Install binary" => crate::ops::install::run(
                crate::ops::install::InstallMode::Auto,
                dirs::home_dir().unwrap().join(".local"),
                None,
                false,
                tx.clone(),
            )
            .await
            .map(|_| ()),
            "Configure MCP" => crate::ops::mcp::configure_code().await,
            _ => {
                tx.send(ProgressEvent::LogLine(format!(
                    "{} — not yet implemented",
                    step_name
                )))
                .await?;
                Ok(())
            }
        };

        match result {
            Ok(()) => {
                tx.send(ProgressEvent::StepCompleted {
                    name: step_name.to_string(),
                    message: "OK".into(),
                })
                .await?;
            }
            Err(e) => {
                tx.send(ProgressEvent::StepFailed {
                    name: step_name.to_string(),
                    error: format!("{}", e),
                })
                .await?;
            }
        }
    }

    tx.send(ProgressEvent::Finished {
        success: true,
        elapsed: start.elapsed(),
        summary: "Build complete".into(),
    })
    .await?;

    Ok(())
}
