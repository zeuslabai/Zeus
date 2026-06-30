//! Gateway Bootstrap — one-time initialization tasks.
//!
//! Creates default HEARTBEAT.md, generates CAPABILITIES.md,
//! loads workspace goal files, and resumes interrupted cooking sessions.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};
use crate::gateway::spawn_typing_heartbeat;

/// Process-lifetime flag for `--fresh` start. Set ONCE at gateway boot by
/// `init_fresh_start_flag()`, then read by all downstream callsites
/// (session resume, channel history injection). This avoids the race where
/// the first callsite to check the `.fresh_start` marker file deletes it,
/// causing subsequent callsites to see a stale (non-fresh) state.
static FRESH_START: AtomicBool = AtomicBool::new(false);

/// Initialize the fresh-start flag from the marker file, exactly once, at boot.
/// Removes the marker file so the next gateway start is not treated as fresh.
/// Must be called before `is_fresh_start()` or the session-resume / history-injection paths.
pub fn init_fresh_start_flag() -> bool {
    let marker = dirs::home_dir()
        .unwrap_or_default()
        .join(".zeus")
        .join(".fresh_start");
    let is_fresh = marker.exists();
    if is_fresh {
        let _ = std::fs::remove_file(&marker);
        FRESH_START.store(true, Ordering::SeqCst);
        info!("Fresh start detected — marker consumed, in-memory flag set for this process");
    }
    is_fresh
}

/// Check whether this gateway process was started with `--fresh`.
/// Safe to call from any task at any time during process lifetime.
pub fn is_fresh_start() -> bool {
    FRESH_START.load(Ordering::SeqCst)
}

/// Ensure default HEARTBEAT.md exists and generate CAPABILITIES.md.
pub async fn bootstrap_workspace(agent: &Arc<RwLock<zeus_agent::Agent>>, config: &zeus_core::Config) {
    let agent_guard = agent.read().await;
    let ws = agent_guard.workspace();

    // Create default HEARTBEAT.md if missing
    if ws.get_heartbeat().await.unwrap_or_default().is_empty() {
        let default_heartbeat = "## hourly\n- Check gateway health and respond HEARTBEAT_OK if all systems operational\n\n## daily\n- Consolidate recent memories and prune stale entries\n- Review and summarize yesterday's session activity\n";
        if let Err(e) = ws.write("HEARTBEAT.md", default_heartbeat).await {
            warn!("Failed to create default HEARTBEAT.md: {}", e);
        } else {
            info!("Created default HEARTBEAT.md with hourly/daily tasks");
        }
    }

    // Generate CAPABILITIES.md — inject platform knowledge into agent context
    let caps = generate_capabilities(config);
    if let Err(e) = ws.write("CAPABILITIES.md", &caps).await {
        warn!("Failed to create CAPABILITIES.md: {}", e);
    } else {
        info!("Generated CAPABILITIES.md with platform knowledge");
    }
}

/// Load workspace goal files into Prometheus on startup (S66-P1C).
pub async fn load_goal_files(
    prometheus: &Arc<RwLock<zeus_prometheus::Prometheus>>,
    workspace_path: &std::path::Path,
) {
    let goals_dir = workspace_path.join("goals");
    if !goals_dir.exists() {
        return;
    }
    let prom_guard = prometheus.read().await;
    let Some(stack) = prom_guard.goal_stack() else { return };

    match std::fs::read_dir(&goals_dir) {
        Ok(entries) => {
            let mut loaded = 0usize;
            let now_ts = chrono::Utc::now().timestamp();
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map_or(false, |e| e == "md") {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        // #157 item B (bootstrap parity): full hot-loader parity
                        // in the boot sweep, mirroring gateway.rs ~3304-3342.
                        // Before item B this loader loaded EVERY .md once,
                        // unconditionally, with no remove_file — so on restart a
                        // future-dated loop file fired immediately (early-fire)
                        // and was then re-read by the 60s hot-loader into a
                        // duplicate (bounded) SQLite row (dup). The three guards
                        // below close both:
                        //
                        // (1) Sweep capped stragglers — a future-dated loop file
                        //     already at its retry cap is the "clear-survival"
                        //     wake (the N+1). loop_tool stopped re-arming, but
                        //     this pending file would still fire once more. The
                        //     file *is* the wake, so cancel it = remove_file.
                        let (attempt, max_attempts) =
                            zeus_agent::tools::parse_goal_retry_counters(&content);
                        if let (Some(a), Some(m)) = (attempt, max_attempts) {
                            if m > 0 && a >= m {
                                info!(
                                    "Boot loader: sweeping capped loop straggler {} (attempt={} >= max_attempts={})",
                                    path.display(),
                                    a,
                                    m
                                );
                                let _ = std::fs::remove_file(&path);
                                continue;
                            }
                        }
                        // (2) Honor `not_before` — a future-dated file is not yet
                        //     due; leave it on disk so the hot-loader fires it at
                        //     the right time instead of early-firing at boot.
                        let (not_before, _body) =
                            zeus_agent::tools::parse_goal_front_matter(&content);
                        if let Some(nb) = not_before {
                            if now_ts < nb {
                                debug!(
                                    "Boot loader: skipping {} (not_before={}, now={}, {}s remaining)",
                                    path.display(),
                                    nb,
                                    now_ts,
                                    nb - now_ts
                                );
                                continue;
                            }
                        }
                        let mut goal = zeus_prometheus::Goal::new(
                            content.trim(),
                            zeus_prometheus::Priority::Normal,
                            zeus_prometheus::GoalSource::System,
                        );
                        // Seed the DURABLE bounded-retry counters from the file's
                        // front-matter, mirroring the hot-loader's promotion seed
                        // (gateway.rs ~3361). Without this, a loop file still
                        // pre-promotion on disk at restart is loaded here as
                        // max_attempts=0 (unbounded) → the cap is silently lost
                        // for that goal, contradicting the survive-restart thesis.
                        if let Some(max) = max_attempts {
                            goal.max_attempts = max as u32;
                            goal.attempt = attempt.unwrap_or(0) as u32;
                        }
                        match stack.add(&goal) {
                            Ok(id) => {
                                info!("Loaded goal from {}: {}", path.display(), id);
                                loaded += 1;
                                // (3) remove_file after a successful add — dedup.
                                //     Mirrors the hot-loader's post-add removal so
                                //     the 60s hot-loader doesn't re-read this same
                                //     file into a duplicate SQLite row.
                                let _ = std::fs::remove_file(&path);
                            }
                            Err(e) => warn!("Failed to add goal from {}: {}", path.display(), e),
                        }
                    }
                }
            }
            if loaded > 0 {
                info!("Loaded {} goal(s) from workspace/goals/", loaded);
            }
        }
        Err(e) => warn!("Failed to read goals directory: {}", e),
    }
}

/// Check if the gateway was started with `--fresh` (clean slate).
///
/// Deprecated in favor of `init_fresh_start_flag()` + `is_fresh_start()`.
/// Kept as a thin shim for any legacy callers; delegates to the in-memory flag
/// so it no longer races on the marker file.
#[deprecated(note = "Use init_fresh_start_flag() once at boot, then is_fresh_start() everywhere else")]
pub fn consume_fresh_start_marker() -> bool {
    is_fresh_start()
}

/// Check for interrupted cooking sessions and auto-resume them (S66-P1A).
pub fn spawn_session_resume(
    prometheus: Arc<RwLock<zeus_prometheus::Prometheus>>,
    agent: Arc<RwLock<zeus_agent::Agent>>,
    interrupted: Vec<zeus_prometheus::InterruptedSession>,
) {
    if interrupted.is_empty() {
        return;
    }
    info!("Found {} interrupted cooking session(s):", interrupted.len());
    for session in &interrupted {
        info!(
            "  - '{}' (iteration {}, {} tool calls, started {})",
            session.original_message.chars().take(80).collect::<String>(),
            session.iteration,
            session.tool_call_count,
            session.started_at.format("%H:%M:%S"),
        );
    }
    tokio::spawn(async move {
        // Small delay to let other subsystems stabilize
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        for session in interrupted {
            info!("Auto-resuming interrupted session: {}", session.session_id);

            // Fix C (Dispatch 24): Recursive-resume-prompt gate.
            // The auto-resume captures `original_message` as the task to pick
            // back up. If a prior auto-resume already ran, that field can
            // contain the full assembled "You were in the middle of: …"
            // prompt — leading to recursive resume of an already-resumed
            // session, which spirals on every gateway restart.
            //
            // Conservative gate: skip resume if the original_message is
            // suspiciously large (>2KB) OR contains the literal
            // recursive marker. Log warn so operators can clean state.
            const RECURSIVE_MARKER: &str = "You were in the middle of:";
            const MAX_RESUME_PROMPT_BYTES: usize = 2048;
            if session.original_message.len() > MAX_RESUME_PROMPT_BYTES
                || session.original_message.contains(RECURSIVE_MARKER)
            {
                warn!(
                    "Skipping auto-resume for session {} — original_message looks recursive (len={}, contains_marker={}). Clean ~/.zeus/sessions/cooking-*.json* to reset.",
                    session.session_id,
                    session.original_message.len(),
                    session.original_message.contains(RECURSIVE_MARKER),
                );
                continue;
            }

            let prom_guard = prometheus.read().await;
            let agent_guard = agent.read().await;
            let tools = agent_guard.tool_schemas();
            let _typing_guard = spawn_typing_heartbeat(agent_guard.channel_manager(), None);
            drop(agent_guard);
            let prompt = format!(
                "You were in the middle of: \"{}\"\nYou got interrupted at step {}. Pick it back up.",
                session.original_message,
                session.iteration,
            );
            match prom_guard.cook_with_history(&prompt, &tools, &[]).await {
                Ok(result) => info!(
                    "Resumed session {} complete: {} iterations",
                    session.session_id, result.iterations
                ),
                Err(e) => warn!(
                    "Failed to resume session {}: {}",
                    session.session_id, e
                ),
            }
        }
    });
}

/// Generate CAPABILITIES.md content from config + hardware auto-detection.
fn generate_capabilities(config: &zeus_core::Config) -> String {
    let mut lines = vec![
        "# Zeus Platform Capabilities\n".to_string(),
    ];

    // Auto-detect hardware environment
    lines.push("## Environment".to_string());
    let hostname = std::process::Command::new("hostname")
        .output().ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_else(|| "unknown".into());
    lines.push(format!("- Hostname: {}", hostname.trim()));
    lines.push(format!("- OS: {}", std::env::consts::OS));
    lines.push(format!("- Arch: {}", std::env::consts::ARCH));
    // GPU detection
    if let Ok(output) = std::process::Command::new("nvidia-smi")
        .args(["--query-gpu=name,memory.total", "--format=csv,noheader,nounits"])
        .output()
    {
        if output.status.success() {
            if let Ok(gpu_info) = String::from_utf8(output.stdout) {
                for (i, line) in gpu_info.trim().lines().enumerate() {
                    lines.push(format!("- GPU {}: {}", i, line.trim()));
                }
            }
        }
    }
    // RAM detection
    #[cfg(target_os = "macos")]
    if let Ok(output) = std::process::Command::new("sysctl").args(["-n", "hw.memsize"]).output() {
        if let Ok(bytes) = String::from_utf8(output.stdout).unwrap_or_default().trim().parse::<u64>() {
            lines.push(format!("- RAM: {} GB", bytes / (1024 * 1024 * 1024)));
        }
    }
    #[cfg(not(target_os = "macos"))]
    if let Ok(output) = std::process::Command::new("free").args(["-g"]).output() {
        if let Ok(free_out) = String::from_utf8(output.stdout) {
            if let Some(mem_line) = free_out.lines().find(|l| l.starts_with("Mem:")) {
                if let Some(total) = mem_line.split_whitespace().nth(1) {
                    lines.push(format!("- RAM: {} GB", total));
                }
            }
        }
    }
    // Local service discovery — check known ports
    let services: &[(&str, u16)] = &[
        ("Z-Image Turbo (image gen)", 7860),
        ("Ollama (local LLM)", 11434),
        ("LTX-2 (video gen)", 7861),
        ("Whisper STT", 8787),
        ("Kokoro TTS", 8788),
    ];
    let mut found_services = Vec::new();
    for (name, port) in services {
        if std::net::TcpStream::connect_timeout(
            &std::net::SocketAddr::from(([127, 0, 0, 1], *port)),
            std::time::Duration::from_millis(100),
        ).is_ok() {
            found_services.push(format!("- localhost:{} — {}", port, name));
        }
    }
    if !found_services.is_empty() {
        lines.push(String::new());
        lines.push("## Local Services (auto-detected)".to_string());
        lines.extend(found_services);
    }
    lines.push(String::new());
    lines.push("**You are running ON this machine. Use localhost for all local services.**".to_string());
    lines.push("**Do NOT SSH into yourself — you already have shell access.**".to_string());
    lines.push(String::new());

    lines.push("## Model".to_string());
    lines.push(format!("- Provider/Model: {}", config.model));
    lines.push(format!("- Max iterations: {}", config.max_iterations));
    lines.push(String::new());
    lines.push("## Connected Channels".to_string());
    if let Some(ref ch) = config.channels {
        if ch.discord.is_some() { lines.push("- Discord: connected".to_string()); }
        if ch.telegram.is_some() { lines.push("- Telegram: connected".to_string()); }
        if ch.slack.is_some() { lines.push("- Slack: connected".to_string()); }
        if ch.email.is_some() { lines.push("- Email: connected".to_string()); }
        if ch.whatsapp.is_some() { lines.push("- WhatsApp: connected".to_string()); }
        if ch.signal.is_some() { lines.push("- Signal: connected".to_string()); }
        if ch.matrix.is_some() { lines.push("- Matrix: connected".to_string()); }
    }
    lines.push(String::new());
    lines.push("## Core Tools (always available)".to_string());
    lines.push("- read_file, write_file, edit_file, list_dir — file operations".to_string());
    lines.push("- shell — execute any command".to_string());
    lines.push("- web_fetch, web_search, deep_research — web intelligence".to_string());
    lines.push("- spawn — launch parallel subagents".to_string());
    lines.push("- message — send to Discord/Telegram/Slack/etc.".to_string());
    lines.push("- loop — schedule self-wakeups for autonomous continuation".to_string());
    lines.push(String::new());
    lines.push("## Platform Features".to_string());
    lines.push("- Browser automation (Chrome CDP): navigate, click, type, screenshot".to_string());
    lines.push("- Memory (Mnemosyne): vector + FTS search, embeddings".to_string());
    lines.push("- Heartbeat: proactive task execution every 5 min".to_string());
    lines.push("- Goals: drop .md files in workspace/goals/ for autonomous processing".to_string());
    lines.push("- Skills: 70+ installed skills for specialized tasks".to_string());
    lines.push(String::new());
    lines.push("## Fleet".to_string());
    // #213: derive coordinator from config instead of hardcoding the fleet value.
    if let Some(coord) = config.agent.as_ref().and_then(|a| a.coordinator.as_deref()) {
        lines.push(format!("- Coordinator: {}", coord));
        lines.push("- Communication: primary team channel".to_string());
    } else {
        lines.push("- Standalone deploy (no coordinator configured)".to_string());
    }
    lines.push("- You can spawn sub-agents for parallel tasks".to_string());
    lines.push("- Report progress as you work — never go silent".to_string());
    lines.join("\n")
}
