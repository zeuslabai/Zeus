//! TUI application state machine and event loop

use crate::event::ProgressEvent;
use crate::theme::Theme;
use crate::views;
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

const ZEUS_VERSION: &str = env!("CARGO_PKG_VERSION");
const TICK_RATE: Duration = Duration::from_millis(80);
const SPINNER_FRAMES: &[char] = &[
    '\u{28CB}', '\u{28D9}', '\u{28F9}', '\u{28F8}', '\u{28FC}', '\u{28F4}', '\u{28E6}', '\u{28E7}',
    '\u{28C7}', '\u{28CF}',
];

/// Current view of the application
#[derive(Debug, Clone, PartialEq)]
pub enum AppView {
    MainMenu,
    InstallMenu,
    BuildMenu,
    DeployMenu,
    McpMenu,
    ServiceMenu,
    Running,
    DeploySummary,
    DoctorResults,
    Finished,
}

/// Step execution state
#[derive(Debug, Clone)]
pub struct StepState {
    pub name: String,
    pub status: StepStatus,
    pub message: String,
    pub duration: Option<Duration>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StepStatus {
    Pending,
    Running,
    Done,
    Failed,
    Warning,
}

/// Deploy result for summary table
#[derive(Debug, Clone)]
pub struct DeployResult {
    pub host: String,
    pub ip: String,
    pub os: String,
    pub status: String,
    pub duration: Duration,
}

/// Doctor check result
#[derive(Debug, Clone)]
pub struct DoctorCheck {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

/// Actions that can be triggered from menus
#[derive(Debug, Clone)]
pub enum Action {
    Doctor,
    InstallDownload,
    InstallSource,
    InstallLocal,
    InstallAuto,
    BuildQuick,
    BuildFull,
    BuildDeploy,
    BuildMacOS,
    BuildIOS,
    BuildApple,
    BuildWebOnly,
    BuildXcode,
    DeployAll,
    DeployWeb,
    Package,
    McpCode,
    McpDesktop,
    McpRemove,
    McpShow,
    ServiceInstall,
    ServiceStart,
    ServiceStop,
    ServiceRestart,
    ServiceStatus,
    ServiceLogs,
    ServiceUninstall,
}

/// Main application state
pub struct App {
    pub view: AppView,
    pub theme: Theme,
    pub progress_rx: mpsc::Receiver<ProgressEvent>,
    pub progress_tx: mpsc::Sender<ProgressEvent>,

    // Running state
    pub steps: Vec<StepState>,
    pub log_buffer: Vec<String>,
    pub log_scroll: usize,
    pub current_step: usize,
    pub overall_progress: u8,
    pub progress_message: String,
    pub spinner_frame: usize,
    pub operation_start: Option<Instant>,
    pub operation_name: String,

    // Menu state
    pub menu_cursor: usize,

    // Pending action from menu selection
    pub pending_action: Option<Action>,

    // Results
    pub deploy_results: Vec<DeployResult>,
    pub doctor_checks: Vec<DoctorCheck>,
    pub finish_success: bool,
    pub finish_summary: String,
    pub finish_elapsed: Duration,

    // Control
    pub should_quit: bool,
}

impl App {
    pub fn new(theme_name: &str) -> Self {
        let (tx, rx) = crate::event::progress_channel();
        Self {
            view: AppView::MainMenu,
            theme: Theme::from_name(theme_name),
            progress_rx: rx,
            progress_tx: tx,
            pending_action: None,
            steps: Vec::new(),
            log_buffer: Vec::new(),
            log_scroll: 0,
            current_step: 0,
            overall_progress: 0,
            progress_message: String::new(),
            spinner_frame: 0,
            operation_start: None,
            operation_name: String::new(),
            menu_cursor: 0,
            deploy_results: Vec::new(),
            doctor_checks: Vec::new(),
            finish_success: false,
            finish_summary: String::new(),
            finish_elapsed: Duration::ZERO,
            should_quit: false,
        }
    }

    pub fn version() -> &'static str {
        ZEUS_VERSION
    }

    pub fn spinner_char(&self) -> char {
        SPINNER_FRAMES[self.spinner_frame % SPINNER_FRAMES.len()]
    }

    /// Process incoming progress events
    pub fn drain_progress(&mut self) {
        while let Ok(event) = self.progress_rx.try_recv() {
            match event {
                ProgressEvent::StepStarted { name, index, total } => {
                    // Ensure we have enough steps
                    while self.steps.len() <= index {
                        self.steps.push(StepState {
                            name: String::new(),
                            status: StepStatus::Pending,
                            message: String::new(),
                            duration: None,
                        });
                    }
                    self.steps[index] = StepState {
                        name: name.clone(),
                        status: StepStatus::Running,
                        message: String::new(),
                        duration: None,
                    };
                    self.current_step = index;
                    self.overall_progress = ((index as f32 / total as f32) * 100.0) as u8;
                    self.progress_message = name;
                }
                ProgressEvent::StepCompleted { name, message } => {
                    if let Some(step) = self.steps.iter_mut().find(|s| s.name == name) {
                        step.status = StepStatus::Done;
                        step.message = message;
                    }
                }
                ProgressEvent::StepFailed { name, error } => {
                    if let Some(step) = self.steps.iter_mut().find(|s| s.name == name) {
                        step.status = StepStatus::Failed;
                        step.message = error;
                    }
                }
                ProgressEvent::StepWarning { name, message } => {
                    if let Some(step) = self.steps.iter_mut().find(|s| s.name == name) {
                        step.status = StepStatus::Warning;
                        step.message = message;
                    }
                }
                ProgressEvent::LogLine(line) => {
                    self.log_buffer.push(line);
                    // Auto-scroll to bottom
                    if self.log_buffer.len() > 500 {
                        self.log_buffer.drain(..self.log_buffer.len() - 500);
                    }
                    self.log_scroll = self.log_buffer.len().saturating_sub(1);
                }
                ProgressEvent::Progress { percent, message } => {
                    self.overall_progress = percent;
                    self.progress_message = message;
                }
                ProgressEvent::Finished {
                    success,
                    elapsed,
                    summary,
                } => {
                    self.finish_success = success;
                    self.finish_summary = summary;
                    self.finish_elapsed = elapsed;
                    self.overall_progress = 100;
                    // Switch to appropriate results view
                    if !self.doctor_checks.is_empty() {
                        self.view = AppView::DoctorResults;
                    } else if !self.deploy_results.is_empty() {
                        self.view = AppView::DeploySummary;
                    } else {
                        self.view = AppView::Finished;
                    }
                }
                ProgressEvent::DeployResult {
                    host,
                    ip,
                    os,
                    status,
                    duration,
                } => {
                    self.deploy_results.push(DeployResult {
                        host,
                        ip,
                        os,
                        status,
                        duration,
                    });
                }
                ProgressEvent::DoctorCheck { name, ok, detail } => {
                    self.doctor_checks.push(DoctorCheck { name, ok, detail });
                }
                ProgressEvent::DoctorRepair {
                    name,
                    success,
                    detail,
                } => {
                    // Show repairs as doctor checks with a "[REPAIR]" prefix
                    self.doctor_checks.push(DoctorCheck {
                        name: format!("[REPAIR] {}", name),
                        ok: success,
                        detail,
                    });
                }
            }
        }
    }

    /// Handle a key event
    pub fn handle_key(&mut self, key: KeyEvent) {
        // Global: Ctrl+C or q quits
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }

        match &self.view {
            AppView::MainMenu
            | AppView::InstallMenu
            | AppView::BuildMenu
            | AppView::DeployMenu
            | AppView::McpMenu
            | AppView::ServiceMenu => {
                self.handle_menu_key(key);
            }
            AppView::Running => {
                // Only allow scroll and quit during operation
                match key.code {
                    KeyCode::Char('q') | KeyCode::Char('Q') => self.should_quit = true,
                    KeyCode::Up | KeyCode::Char('k') => {
                        self.log_scroll = self.log_scroll.saturating_sub(1);
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if self.log_scroll < self.log_buffer.len().saturating_sub(1) {
                            self.log_scroll += 1;
                        }
                    }
                    _ => {}
                }
            }
            AppView::Finished | AppView::DeploySummary | AppView::DoctorResults => {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => {
                        self.should_quit = true;
                    }
                    KeyCode::Enter => {
                        // Return to main menu
                        self.view = AppView::MainMenu;
                        self.menu_cursor = 0;
                        self.reset_operation();
                    }
                    _ => {}
                }
            }
        }
    }

    fn handle_menu_key(&mut self, key: KeyEvent) {
        let item_count = self.menu_items().len();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.menu_cursor = (self.menu_cursor + item_count - 1) % item_count;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.menu_cursor = (self.menu_cursor + 1) % item_count;
            }
            KeyCode::Enter => {
                self.select_menu_item();
            }
            KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => match &self.view {
                AppView::MainMenu => self.should_quit = true,
                _ => {
                    self.view = AppView::MainMenu;
                    self.menu_cursor = 0;
                }
            },
            _ => {}
        }
    }

    /// Get menu items for the current view
    pub fn menu_items(&self) -> Vec<&'static str> {
        match &self.view {
            AppView::MainMenu => vec![
                "Install Zeus locally",
                "Deploy to remote hosts",
                "Deploy web frontend (.226)",
                "Package for distribution",
                "Configure MCP",
                "Manage services",
                "Run diagnostics",
            ],
            AppView::InstallMenu => vec![
                "Quick Install (download latest release)",
                "Build from source",
                "Install from local binary",
                "Update existing installation",
            ],
            AppView::BuildMenu => vec![
                "Quick build (CLI + install + restart)",
                "Full build (test + CLI + web + install)",
                "Full deploy (test + build + install + MCP)",
                "Build macOS Desktop (FFI + Desktop)",
                "Build iOS app",
                "Build all Apple targets",
                "Build web only",
                "Regenerate Xcode projects",
            ],
            AppView::McpMenu => vec![
                "Claude Code (stdio MCP)",
                "Claude Desktop",
                "Remove MCP configuration",
                "Show current config",
            ],
            AppView::ServiceMenu => vec![
                "Install gateway service",
                "Start gateway",
                "Stop gateway",
                "Restart gateway",
                "Show status",
                "View logs",
                "Uninstall service",
            ],
            AppView::DeployMenu => vec![
                "Select from fleet.conf",
                "Enter hosts manually",
                "Deploy to all fleet nodes",
            ],
            _ => vec![],
        }
    }

    fn select_menu_item(&mut self) {
        match &self.view {
            AppView::MainMenu => match self.menu_cursor {
                0 => {
                    self.view = AppView::InstallMenu;
                    self.menu_cursor = 0;
                }
                1 => {
                    self.view = AppView::DeployMenu;
                    self.menu_cursor = 0;
                }
                2 => {
                    self.start_operation("Deploy Web Frontend");
                    self.pending_action = Some(Action::DeployWeb);
                }
                3 => {
                    self.start_operation("Package for Distribution");
                    self.pending_action = Some(Action::Package);
                }
                4 => {
                    self.view = AppView::McpMenu;
                    self.menu_cursor = 0;
                }
                5 => {
                    self.view = AppView::ServiceMenu;
                    self.menu_cursor = 0;
                }
                6 => {
                    self.start_operation("Run Diagnostics");
                    self.pending_action = Some(Action::Doctor);
                }
                _ => {}
            },
            AppView::InstallMenu => {
                let (name, action) = match self.menu_cursor {
                    0 => ("Install Zeus (download)", Action::InstallDownload),
                    1 => ("Install Zeus (source)", Action::InstallSource),
                    2 => ("Install Zeus (local)", Action::InstallLocal),
                    3 => ("Install Zeus (auto)", Action::InstallAuto),
                    _ => return,
                };
                self.start_operation(name);
                self.pending_action = Some(action);
            }
            AppView::BuildMenu => {
                let (name, action) = match self.menu_cursor {
                    0 => ("Quick Build", Action::BuildQuick),
                    1 => ("Full Build", Action::BuildFull),
                    2 => ("Full Deploy", Action::BuildDeploy),
                    3 => ("Build macOS Desktop", Action::BuildMacOS),
                    4 => ("Build iOS", Action::BuildIOS),
                    5 => ("Build Apple Targets", Action::BuildApple),
                    6 => ("Build Web Only", Action::BuildWebOnly),
                    7 => ("Regenerate Xcode", Action::BuildXcode),
                    _ => return,
                };
                self.start_operation(name);
                self.pending_action = Some(action);
            }
            AppView::McpMenu => {
                let (name, action) = match self.menu_cursor {
                    0 => ("Configure Claude Code MCP", Action::McpCode),
                    1 => ("Configure Claude Desktop MCP", Action::McpDesktop),
                    2 => ("Remove MCP Configuration", Action::McpRemove),
                    3 => ("Show MCP Configuration", Action::McpShow),
                    _ => return,
                };
                self.start_operation(name);
                self.pending_action = Some(action);
            }
            AppView::ServiceMenu => {
                let (name, action) = match self.menu_cursor {
                    0 => ("Install Gateway Service", Action::ServiceInstall),
                    1 => ("Start Gateway", Action::ServiceStart),
                    2 => ("Stop Gateway", Action::ServiceStop),
                    3 => ("Restart Gateway", Action::ServiceRestart),
                    4 => ("Service Status", Action::ServiceStatus),
                    5 => ("Service Logs", Action::ServiceLogs),
                    6 => ("Uninstall Service", Action::ServiceUninstall),
                    _ => return,
                };
                self.start_operation(name);
                self.pending_action = Some(action);
            }
            AppView::DeployMenu => {
                let (name, action) = match self.menu_cursor {
                    0..=2 => ("Deploy to Fleet", Action::DeployAll),
                    _ => return,
                };
                self.start_operation(name);
                self.pending_action = Some(action);
            }
            _ => {}
        }
    }

    fn start_operation(&mut self, name: &str) {
        self.view = AppView::Running;
        self.operation_name = name.to_string();
        self.operation_start = Some(Instant::now());
        self.steps.clear();
        self.log_buffer.clear();
        self.log_scroll = 0;
        self.overall_progress = 0;
        self.progress_message = format!("Starting {}...", name);
    }

    fn reset_operation(&mut self) {
        self.steps.clear();
        self.log_buffer.clear();
        self.log_scroll = 0;
        self.overall_progress = 0;
        self.progress_message.clear();
        self.operation_name.clear();
        self.operation_start = None;
        self.deploy_results.clear();
        self.doctor_checks.clear();
    }

    /// Render the current view
    pub fn render(&self, frame: &mut Frame) {
        match &self.view {
            AppView::MainMenu
            | AppView::InstallMenu
            | AppView::BuildMenu
            | AppView::DeployMenu
            | AppView::McpMenu
            | AppView::ServiceMenu => {
                views::main_menu::render(frame, self);
            }
            AppView::Running => {
                views::progress::render(frame, self);
            }
            AppView::Finished => {
                views::progress::render(frame, self);
            }
            AppView::DoctorResults => {
                views::doctor::render(frame, self);
            }
            AppView::DeploySummary => {
                views::deploy_table::render(frame, self);
            }
        }
    }
}

/// Run the TUI event loop
pub async fn run(app: &mut App) -> Result<()> {
    crate::tui::install_panic_hook();
    let mut terminal = crate::tui::init()?;

    let result = run_loop(app, &mut terminal).await;

    crate::tui::restore()?;
    result
}

async fn run_loop(app: &mut App, terminal: &mut crate::tui::Tui) -> Result<()> {
    loop {
        terminal.draw(|f| app.render(f))?;

        // Check for pending actions to dispatch
        if let Some(action) = app.pending_action.take() {
            dispatch_action(action, &app.progress_tx);
        }

        // Poll terminal events
        if event::poll(TICK_RATE)?
            && let Event::Key(key) = event::read()?
        {
            app.handle_key(key);
        }

        // Drain progress channel
        app.drain_progress();

        // Advance spinner
        app.spinner_frame = app.spinner_frame.wrapping_add(1);

        if app.should_quit {
            break;
        }
    }

    Ok(())
}

/// Spawn an async operation based on a menu action
fn dispatch_action(action: Action, tx: &mpsc::Sender<ProgressEvent>) {
    let tx = tx.clone();
    let project_root = crate::config::find_project_root()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let cpu_count = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);

    tokio::spawn(async move {
        let result = match action {
            Action::Doctor => crate::ops::doctor::run(tx.clone()).await,

            Action::InstallDownload => {
                crate::ops::install::run(
                    crate::ops::install::InstallMode::Download,
                    std::path::PathBuf::from("/usr/local"),
                    None,
                    false,
                    tx.clone(),
                )
                .await
            }
            Action::InstallSource => {
                crate::ops::install::run(
                    crate::ops::install::InstallMode::Source,
                    std::path::PathBuf::from("/usr/local"),
                    None,
                    false,
                    tx.clone(),
                )
                .await
            }
            Action::InstallLocal => {
                // Default to target/release/zeus if available
                let local_bin = project_root.join("target/release/zeus");
                crate::ops::install::run(
                    crate::ops::install::InstallMode::Local(local_bin),
                    std::path::PathBuf::from("/usr/local"),
                    None,
                    false,
                    tx.clone(),
                )
                .await
            }
            Action::InstallAuto => {
                crate::ops::install::run(
                    crate::ops::install::InstallMode::Auto,
                    std::path::PathBuf::from("/usr/local"),
                    None,
                    false,
                    tx.clone(),
                )
                .await
            }

            Action::BuildQuick => {
                crate::ops::build::run(
                    crate::ops::build::BuildOpts {
                        project_root,
                        cli: true,
                        install: true,
                        restart: true,
                        jobs: cpu_count,
                        ..Default::default()
                    },
                    tx.clone(),
                )
                .await
            }
            Action::BuildFull => {
                crate::ops::build::run(
                    crate::ops::build::BuildOpts {
                        project_root,
                        test: true,
                        cli: true,
                        web: true,
                        install: true,
                        restart: true,
                        jobs: cpu_count,
                        ..Default::default()
                    },
                    tx.clone(),
                )
                .await
            }
            Action::BuildDeploy => {
                crate::ops::build::run(
                    crate::ops::build::BuildOpts {
                        project_root,
                        test: true,
                        cli: true,
                        web: true,
                        install: true,
                        restart: true,
                        mcp: true,
                        jobs: cpu_count,
                        ..Default::default()
                    },
                    tx.clone(),
                )
                .await
            }
            Action::BuildMacOS => {
                crate::ops::build::run(
                    crate::ops::build::BuildOpts {
                        project_root,
                        ffi: true,
                        macos: true,
                        jobs: cpu_count,
                        ..Default::default()
                    },
                    tx.clone(),
                )
                .await
            }
            Action::BuildIOS => {
                crate::ops::build::run(
                    crate::ops::build::BuildOpts {
                        project_root,
                        ios: true,
                        jobs: cpu_count,
                        ..Default::default()
                    },
                    tx.clone(),
                )
                .await
            }
            Action::BuildApple => {
                crate::ops::build::run(
                    crate::ops::build::BuildOpts {
                        project_root,
                        ffi: true,
                        macos: true,
                        ios: true,
                        jobs: cpu_count,
                        ..Default::default()
                    },
                    tx.clone(),
                )
                .await
            }
            Action::BuildWebOnly => {
                crate::ops::build::run(
                    crate::ops::build::BuildOpts {
                        project_root,
                        web: true,
                        jobs: cpu_count,
                        ..Default::default()
                    },
                    tx.clone(),
                )
                .await
            }
            Action::BuildXcode => {
                crate::ops::build::run(
                    crate::ops::build::BuildOpts {
                        project_root,
                        xcode: true,
                        jobs: cpu_count,
                        ..Default::default()
                    },
                    tx.clone(),
                )
                .await
            }

            Action::DeployAll => {
                let nodes = crate::fleet::load_fleet_conf(Some(&project_root)).unwrap_or_default();
                let binary = project_root.join("target/release/zeus");
                let opts = crate::ops::deploy::DeployOpts {
                    setup: true,
                    config_only: false,
                    install_service: false,
                };
                crate::ops::deploy::run(nodes, binary, opts, tx.clone()).await
            }

            Action::DeployWeb => crate::ops::deploy_web::run(None, false, tx.clone()).await,

            Action::Package => {
                let project_root = crate::config::find_project_root()
                    .unwrap_or_else(|| std::env::current_dir().unwrap());
                let pkg_opts = crate::ops::package::PackageOpts {
                    cli_only: false,
                    app_only: false,
                    version: env!("CARGO_PKG_VERSION").to_string(),
                    sign_identity: None,
                    notarize: false,
                    apple_id: None,
                    team_id: None,
                    dist_dir: project_root.join("dist/mac"),
                    project_root,
                    skip_build: false,
                };
                crate::ops::package::run(pkg_opts, tx.clone()).await
            }

            Action::McpCode => {
                let result = crate::ops::mcp::configure_code().await;
                let _ = tx
                    .send(ProgressEvent::Finished {
                        success: result.is_ok(),
                        elapsed: Duration::from_millis(100),
                        summary: match &result {
                            Ok(_) => "Claude Code MCP configured".into(),
                            Err(e) => format!("Failed: {e}"),
                        },
                    })
                    .await;
                Ok(())
            }
            Action::McpDesktop => {
                let result = crate::ops::mcp::configure_desktop().await;
                let _ = tx
                    .send(ProgressEvent::Finished {
                        success: result.is_ok(),
                        elapsed: Duration::from_millis(100),
                        summary: match &result {
                            Ok(_) => "Claude Desktop MCP configured".into(),
                            Err(e) => format!("Failed: {e}"),
                        },
                    })
                    .await;
                Ok(())
            }
            Action::McpRemove => {
                let result = crate::ops::mcp::remove().await;
                let _ = tx
                    .send(ProgressEvent::Finished {
                        success: result.is_ok(),
                        elapsed: Duration::from_millis(100),
                        summary: match &result {
                            Ok(_) => "MCP configuration removed".into(),
                            Err(e) => format!("Failed: {e}"),
                        },
                    })
                    .await;
                Ok(())
            }
            Action::McpShow => {
                match crate::ops::mcp::show().await {
                    Ok(output) => {
                        for line in output.lines() {
                            let _ = tx.send(ProgressEvent::LogLine(line.to_string())).await;
                        }
                        let _ = tx
                            .send(ProgressEvent::Finished {
                                success: true,
                                elapsed: Duration::from_millis(50),
                                summary: "MCP configuration shown".into(),
                            })
                            .await;
                    }
                    Err(e) => {
                        let _ = tx
                            .send(ProgressEvent::Finished {
                                success: false,
                                elapsed: Duration::from_millis(50),
                                summary: format!("Failed: {}", e),
                            })
                            .await;
                    }
                }
                Ok(())
            }

            Action::ServiceInstall => crate::ops::service::run("install", tx.clone()).await,
            Action::ServiceStart => crate::ops::service::run("start", tx.clone()).await,
            Action::ServiceStop => crate::ops::service::run("stop", tx.clone()).await,
            Action::ServiceRestart => crate::ops::service::run("restart", tx.clone()).await,
            Action::ServiceStatus => crate::ops::service::run("status", tx.clone()).await,
            Action::ServiceLogs => {
                // Logs need special handling — tail -f
                let _ = tx
                    .send(ProgressEvent::LogLine(
                        "Use 'zeus-setup service logs' from CLI for live log tailing".into(),
                    ))
                    .await;
                let _ = tx
                    .send(ProgressEvent::Finished {
                        success: true,
                        elapsed: Duration::from_millis(50),
                        summary: "Use CLI for log tailing".into(),
                    })
                    .await;
                Ok(())
            }
            Action::ServiceUninstall => crate::ops::service::run("uninstall", tx.clone()).await,
        };

        if let Err(e) = result {
            let _ = tx
                .send(ProgressEvent::Finished {
                    success: false,
                    elapsed: Duration::from_secs(0),
                    summary: format!("Error: {}", e),
                })
                .await;
        }
    });
}

/// Run in headless mode (no TUI, plain text output)
pub async fn run_headless(mut rx: mpsc::Receiver<ProgressEvent>) -> Result<bool> {
    let mut success = true;

    while let Some(event) = rx.recv().await {
        match event {
            ProgressEvent::StepStarted {
                name, index, total, ..
            } => {
                println!("[{}/{}] {}...", index + 1, total, name);
            }
            ProgressEvent::StepCompleted { name, message } => {
                println!("  \x1b[32m✓\x1b[0m {} — {}", name, message);
            }
            ProgressEvent::StepFailed { name, error } => {
                eprintln!("  \x1b[31m✗\x1b[0m {} — {}", name, error);
                success = false;
            }
            ProgressEvent::StepWarning { name, message } => {
                println!("  \x1b[33m!\x1b[0m {} — {}", name, message);
            }
            ProgressEvent::LogLine(line) => {
                println!("    {}", line);
            }
            ProgressEvent::Progress { percent, message } => {
                println!("  [{:3}%] {}", percent, message);
            }
            ProgressEvent::Finished {
                success: s,
                elapsed,
                summary,
            } => {
                success = s;
                if s {
                    println!(
                        "\n\x1b[32m✓ Done\x1b[0m ({:.1}s) — {}",
                        elapsed.as_secs_f64(),
                        summary
                    );
                } else {
                    eprintln!(
                        "\n\x1b[31m✗ Failed\x1b[0m ({:.1}s) — {}",
                        elapsed.as_secs_f64(),
                        summary
                    );
                }
                break;
            }
            ProgressEvent::DoctorCheck { name, ok, detail } => {
                let icon = if ok {
                    "\x1b[32m✓\x1b[0m"
                } else {
                    "\x1b[31m✗\x1b[0m"
                };
                println!("  {} {} — {}", icon, name, detail);
            }
            ProgressEvent::DoctorRepair {
                name,
                success,
                detail,
            } => {
                let icon = if success {
                    "\x1b[34m⚡\x1b[0m"
                } else {
                    "\x1b[31m✗\x1b[0m"
                };
                println!("  {} [REPAIR] {} — {}", icon, name, detail);
            }
            ProgressEvent::DeployResult {
                host,
                ip,
                status,
                duration,
                ..
            } => {
                println!(
                    "  {} ({}) — {} ({:.1}s)",
                    host,
                    ip,
                    status,
                    duration.as_secs_f64()
                );
            }
        }
    }

    Ok(success)
}
