//! Zeus Setup — Professional TUI installer, builder, and deployer
//!
//! Usage:
//!   zeus-setup              # Interactive TUI menu
//!   zeus-setup install      # Install latest release
//!   zeus-setup build        # Quick build
//!   zeus-setup doctor       # Run diagnostics
//!   zeus-setup --help       # Full help

use anyhow::Result;
use clap::Parser;
use zeus_setup::cli::{Cli, Command, ServiceAction};
use zeus_setup::{app, config, event, fleet, ops};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        // No subcommand — interactive TUI menu
        None => {
            if cli.non_interactive {
                eprintln!("No command specified. Use --help for usage.");
                std::process::exit(1);
            }
            let mut app = app::App::new(&cli.theme);
            app::run(&mut app).await?;
        }

        // Doctor — runs directly
        Some(Command::Doctor) => {
            let (tx, rx) = event::progress_channel();
            let handle = tokio::spawn(async move {
                if let Err(e) = ops::doctor::run(tx).await {
                    eprintln!("Doctor failed: {}", e);
                }
            });

            if cli.non_interactive {
                let success = app::run_headless(rx).await?;
                handle.await?;
                if !success {
                    std::process::exit(1);
                }
            } else {
                let mut app = app::App::new(&cli.theme);
                app.view = app::AppView::DoctorResults;
                app.progress_rx = rx;
                app::run(&mut app).await?;
                handle.await?;
            }
        }

        // Install
        Some(Command::Install {
            version,
            local,
            source,
            prefix,
            reconfigure,
        }) => {
            let mode = if source {
                ops::install::InstallMode::Source
            } else if let Some(path) = local {
                ops::install::InstallMode::Local(path)
            } else {
                ops::install::InstallMode::Download
            };

            let prefix = prefix.unwrap_or_else(|| {
                dirs::home_dir()
                    .expect("Could not determine home directory")
                    .join(".local")
            });

            let (tx, rx) = event::progress_channel();
            let handle = tokio::spawn(async move {
                if let Err(e) = ops::install::run(mode, prefix, version, reconfigure, tx).await {
                    eprintln!("Install failed: {}", e);
                }
            });

            if cli.non_interactive {
                let success = app::run_headless(rx).await?;
                handle.await?;
                if !success {
                    std::process::exit(1);
                }
            } else {
                let mut app = app::App::new(&cli.theme);
                app.view = app::AppView::Running;
                app.operation_name = "Install Zeus".into();
                app.operation_start = Some(std::time::Instant::now());
                app.progress_rx = rx;
                app::run(&mut app).await?;
                handle.await?;
            }
        }

        // Build
        Some(Command::Build {
            pull,
            test,
            web,
            ffi,
            macos,
            ios,
            apple,
            xcode,
            no_install,
            no_restart,
            cli_only,
            web_only,
            mcp,
            all,
            deploy,
            jobs,
        }) => {
            let project_root =
                config::find_project_root().unwrap_or_else(|| std::env::current_dir().unwrap());

            let cpu_count = std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4);

            let opts = ops::build::BuildOpts {
                project_root,
                pull,
                test: test || all || deploy,
                cli: !web_only,
                web: web || web_only || all || deploy,
                ffi: ffi || apple,
                macos: macos || apple,
                ios: ios || apple,
                xcode,
                install: !no_install && !cli_only,
                restart: !no_restart && !no_install,
                mcp: mcp || deploy,
                jobs: jobs.unwrap_or(cpu_count),
            };

            let (tx, rx) = event::progress_channel();
            let handle = tokio::spawn(async move {
                if let Err(e) = ops::build::run(opts, tx).await {
                    eprintln!("Build failed: {}", e);
                }
            });

            if cli.non_interactive {
                let success = app::run_headless(rx).await?;
                handle.await?;
                if !success {
                    std::process::exit(1);
                }
            } else {
                let mut app = app::App::new(&cli.theme);
                app.view = app::AppView::Running;
                app.operation_name = "Build from Source".into();
                app.operation_start = Some(std::time::Instant::now());
                app.progress_rx = rx;
                app::run(&mut app).await?;
                handle.await?;
            }
        }

        // Deploy
        Some(Command::Deploy {
            targets,
            all,
            hosts,
            list,
            setup,
            config_only,
            no_setup,
            service,
        }) => {
            let project_root = config::find_project_root();
            let nodes = fleet::load_fleet_conf(project_root.as_deref())?;

            if list {
                println!("Fleet nodes:");
                for node in &nodes {
                    println!("  {}", node);
                }
                return Ok(());
            }

            let deploy_targets = if all {
                nodes
            } else if let Some(hosts_file) = hosts {
                let content = std::fs::read_to_string(hosts_file)?;
                content
                    .lines()
                    .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
                    .map(|l| {
                        let parts: Vec<&str> = l.split_whitespace().collect();
                        fleet::FleetNode {
                            name: parts.first().unwrap_or(&"").to_string(),
                            ip: parts
                                .get(1)
                                .unwrap_or(parts.first().unwrap_or(&""))
                                .to_string(),
                            os: parts.get(2).unwrap_or(&"darwin").to_string(),
                            user: parts.get(3).unwrap_or(&"mike").to_string(),
                            comment: String::new(),
                        }
                    })
                    .collect()
            } else {
                // Resolve shortnames
                targets
                    .iter()
                    .map(|t| {
                        if let Some(node) = nodes.iter().find(|n| n.name == *t) {
                            node.clone()
                        } else {
                            // Assume user@host format
                            fleet::FleetNode {
                                name: t.clone(),
                                ip: t.clone(),
                                os: "darwin".into(),
                                user: "mike".into(),
                                comment: String::new(),
                            }
                        }
                    })
                    .collect()
            };

            let binary = std::path::PathBuf::from("target/release/zeus");
            if !config_only && !binary.exists() {
                anyhow::bail!("Binary not found at target/release/zeus — build first");
            }

            let deploy_opts = ops::deploy::DeployOpts {
                setup: setup || !no_setup, // default: setup enabled
                config_only,
                install_service: service,
            };

            let (tx, rx) = event::progress_channel();
            let handle = tokio::spawn(async move {
                if let Err(e) = ops::deploy::run(deploy_targets, binary, deploy_opts, tx).await {
                    eprintln!("Deploy failed: {}", e);
                }
            });

            if cli.non_interactive {
                let success = app::run_headless(rx).await?;
                handle.await?;
                if !success {
                    std::process::exit(1);
                }
            } else {
                let mut app = app::App::new(&cli.theme);
                app.view = app::AppView::Running;
                app.operation_name = "Fleet Deployment".into();
                app.operation_start = Some(std::time::Instant::now());
                app.progress_rx = rx;
                app::run(&mut app).await?;
                handle.await?;
            }
        }

        // Deploy Web
        Some(Command::DeployWeb { branch, skip_pull }) => {
            let (tx, rx) = event::progress_channel();
            let handle = tokio::spawn(async move {
                if let Err(e) = ops::deploy_web::run(branch, skip_pull, tx).await {
                    eprintln!("Deploy web failed: {}", e);
                }
            });

            if cli.non_interactive {
                let success = app::run_headless(rx).await?;
                handle.await?;
                if !success {
                    std::process::exit(1);
                }
            } else {
                let mut app = app::App::new(&cli.theme);
                app.view = app::AppView::Running;
                app.operation_name = "Deploy Web Frontend".into();
                app.operation_start = Some(std::time::Instant::now());
                app.progress_rx = rx;
                app::run(&mut app).await?;
                handle.await?;
            }
        }

        // Package
        Some(Command::Package {
            cli_only,
            app_only,
            version,
            sign,
            dist_dir,
            notarize,
            apple_id,
            team_id,
            skip_build,
        }) => {
            let project_root =
                config::find_project_root().unwrap_or_else(|| std::env::current_dir().unwrap());
            let version = version.unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());
            let dist_dir = dist_dir.unwrap_or_else(|| project_root.join("dist/mac"));

            let pkg_opts = ops::package::PackageOpts {
                cli_only,
                app_only,
                version,
                sign_identity: sign,
                notarize,
                apple_id,
                team_id,
                dist_dir,
                project_root,
                skip_build,
            };

            let (tx, rx) = event::progress_channel();
            let handle = tokio::spawn(async move {
                if let Err(e) = ops::package::run(pkg_opts, tx).await {
                    eprintln!("Package failed: {}", e);
                }
            });

            if cli.non_interactive {
                let success = app::run_headless(rx).await?;
                handle.await?;
                if !success {
                    std::process::exit(1);
                }
            } else {
                let mut app = app::App::new(&cli.theme);
                app.view = app::AppView::Running;
                app.operation_name = "Package for Distribution".into();
                app.operation_start = Some(std::time::Instant::now());
                app.progress_rx = rx;
                app::run(&mut app).await?;
                handle.await?;
            }
        }

        // MCP
        Some(Command::Mcp {
            code,
            desktop,
            remove,
            show,
        }) => {
            if show {
                let output = ops::mcp::show().await?;
                println!("{}", output);
            } else if remove {
                ops::mcp::remove().await?;
                println!("MCP configuration removed");
            } else if desktop {
                ops::mcp::configure_desktop().await?;
                println!("Claude Desktop MCP configured");
            } else if code {
                ops::mcp::configure_code().await?;
                println!("Claude Code MCP configured");
            } else if !cli.non_interactive {
                // Interactive MCP menu
                let mut app = app::App::new(&cli.theme);
                app.view = app::AppView::McpMenu;
                app::run(&mut app).await?;
            } else {
                eprintln!("Specify --code, --desktop, --remove, or --show");
                std::process::exit(1);
            }
        }

        // Service
        Some(Command::Service { action }) => {
            if let Some(action) = action {
                let action_str = match action {
                    ServiceAction::Install => "install",
                    ServiceAction::Start => "start",
                    ServiceAction::Stop => "stop",
                    ServiceAction::Restart => "restart",
                    ServiceAction::Status => "status",
                    ServiceAction::Logs => "logs",
                    ServiceAction::Uninstall => "uninstall",
                };

                let (tx, rx) = event::progress_channel();
                let action_owned = action_str.to_string();
                let handle = tokio::spawn(async move {
                    if let Err(e) = ops::service::run(&action_owned, tx).await {
                        eprintln!("Service {} failed: {}", action_owned, e);
                    }
                });

                if cli.non_interactive {
                    let success = app::run_headless(rx).await?;
                    handle.await?;
                    if !success {
                        std::process::exit(1);
                    }
                } else {
                    let mut app = app::App::new(&cli.theme);
                    app.view = app::AppView::Running;
                    app.operation_name = format!("Service {}", action_str);
                    app.operation_start = Some(std::time::Instant::now());
                    app.progress_rx = rx;
                    app::run(&mut app).await?;
                    handle.await?;
                }
            } else if !cli.non_interactive {
                let mut app = app::App::new(&cli.theme);
                app.view = app::AppView::ServiceMenu;
                app::run(&mut app).await?;
            } else {
                eprintln!(
                    "Specify an action: install, start, stop, restart, status, logs, uninstall"
                );
                std::process::exit(1);
            }
        }
    }

    Ok(())
}
