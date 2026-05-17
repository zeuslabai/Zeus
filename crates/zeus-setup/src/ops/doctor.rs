//! Diagnostics — check binary, config, workspace, services, ports

use crate::config::zeus_home;
use crate::event::ProgressEvent;
use crate::platform::Platform;
use anyhow::Result;
use std::hash::{Hash, Hasher};
use std::time::Instant;
use tokio::sync::mpsc;

pub async fn run(tx: mpsc::Sender<ProgressEvent>) -> Result<()> {
    run_with_repair(tx, false).await
}

pub async fn run_repair(tx: mpsc::Sender<ProgressEvent>) -> Result<()> {
    run_with_repair(tx, true).await
}

async fn run_with_repair(tx: mpsc::Sender<ProgressEvent>, repair: bool) -> Result<()> {
    let start = Instant::now();
    let zeus_dir = zeus_home();
    let platform = Platform::detect()?;

    tx.send(ProgressEvent::StepStarted {
        name: if repair {
            "Diagnostics + Repair".into()
        } else {
            "Diagnostics".into()
        },
        index: 0,
        total: 1,
    })
    .await?;

    // 1. Binary exists
    let binary = crate::config::zeus_bin();
    check(
        &tx,
        "Zeus binary",
        binary.exists(),
        if binary.exists() {
            format!("Found at {}", binary.display())
        } else {
            format!("Not found at {}", binary.display())
        },
    )
    .await;

    // 2. Config file
    let config_path = zeus_dir.join("config.toml");
    check(
        &tx,
        "Config file",
        config_path.exists(),
        if config_path.exists() {
            format!("{}", config_path.display())
        } else {
            "Missing ~/.zeus/config.toml".into()
        },
    )
    .await;

    // 3. Workspace directories
    let workspace = zeus_dir.join("workspace");
    check(
        &tx,
        "Workspace",
        workspace.exists(),
        if workspace.exists() {
            format!("{}", workspace.display())
        } else {
            "Missing ~/.zeus/workspace/".into()
        },
    )
    .await;

    // 4. Sessions directory
    let sessions = zeus_dir.join("sessions");
    check(
        &tx,
        "Sessions",
        sessions.exists(),
        if sessions.exists() {
            format!("{}", sessions.display())
        } else {
            "Missing ~/.zeus/sessions/".into()
        },
    )
    .await;

    // 5. Environment file
    let env_file = zeus_dir.join(".env");
    check(
        &tx,
        "API keys (.env)",
        env_file.exists(),
        if env_file.exists() {
            let content = std::fs::read_to_string(&env_file).unwrap_or_default();
            let keys_set = content
                .lines()
                .filter(|l| !l.starts_with('#') && l.contains('='))
                .count();
            format!("{} keys configured", keys_set)
        } else {
            "Missing ~/.zeus/.env".into()
        },
    )
    .await;

    // 6. Platform
    check(&tx, "Platform", true, format!("{}", platform)).await;

    // 7. Zeus version (if binary exists)
    if binary.exists() {
        let output = tokio::process::Command::new(&binary)
            .arg("--version")
            .output()
            .await;
        match output {
            Ok(out) if out.status.success() => {
                let version = String::from_utf8_lossy(&out.stdout).trim().to_string();
                check(&tx, "Zeus version", true, version).await;
            }
            _ => {
                check(
                    &tx,
                    "Zeus version",
                    false,
                    "Failed to run zeus --version".into(),
                )
                .await;
            }
        }
    }

    // 8. Gateway health check
    let gateway_port: u16 = std::env::var("ZEUS_GATEWAY_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080);
    let health = check_port("127.0.0.1", gateway_port).await;
    check(
        &tx,
        &format!("Gateway ({})", gateway_port),
        health,
        if health {
            "Responding".into()
        } else {
            "Not responding".into()
        },
    )
    .await;

    // 9. API port
    let api = check_port("127.0.0.1", 3001).await;
    check(
        &tx,
        "API (3001)",
        api,
        if api {
            "Responding".into()
        } else {
            "Not responding".into()
        },
    )
    .await;

    // 10. WebSocket port
    let ws = check_port("127.0.0.1", 3002).await;
    check(
        &tx,
        "WebSocket (3002)",
        ws,
        if ws {
            "Responding".into()
        } else {
            "Not responding".into()
        },
    )
    .await;

    // 11. Claude Code MCP
    let claude_json = dirs::home_dir()
        .map(|h| h.join(".claude.json"))
        .unwrap_or_default();
    let mcp_ok = if claude_json.exists() {
        let content = std::fs::read_to_string(&claude_json).unwrap_or_default();
        content.contains("zeus")
    } else {
        false
    };
    check(
        &tx,
        "Claude Code MCP",
        mcp_ok,
        if mcp_ok {
            "Configured in ~/.claude.json".into()
        } else {
            "Not configured".into()
        },
    )
    .await;

    // 12. Service status
    let service_ok = check_service(&platform).await;
    check(
        &tx,
        "Gateway service",
        service_ok,
        if service_ok {
            "Running".into()
        } else {
            "Not running or not installed".into()
        },
    )
    .await;

    // 13. Stale sessions (>30 days without activity)
    {
        let (stale, total) = count_stale_sessions(&sessions, 30);
        let ok = stale == 0;
        check(
            &tx,
            "Stale sessions",
            ok,
            if ok {
                format!("{total} session(s), none stale (>30d)")
            } else {
                format!("{stale}/{total} session(s) inactive >30 days — consider pruning")
            },
        )
        .await;
    }

    // 14. Orphaned / empty JSONL files
    {
        let orphaned = count_orphaned_jsonl(&sessions);
        let ok = orphaned == 0;
        check(
            &tx,
            "Orphaned JSONL files",
            ok,
            if ok {
                "No empty session files".into()
            } else {
                format!("{orphaned} empty JSONL file(s) in sessions dir — safe to delete")
            },
        )
        .await;
    }

    // 15. Deprecated config keys
    {
        let warnings = check_deprecated_config_keys(&config_path);
        let ok = warnings.is_empty();
        check(
            &tx,
            "Config deprecation",
            ok,
            if ok {
                "No deprecated keys found".into()
            } else {
                warnings.join("; ")
            },
        )
        .await;
    }

    // 16. Workspace directory permissions
    {
        let (readable, writable) = check_dir_permissions(&workspace);
        let ok = readable && writable;
        check(
            &tx,
            "Workspace permissions",
            ok,
            if ok {
                "Readable and writable".into()
            } else if !readable {
                "Workspace directory is not readable".into()
            } else {
                "Workspace directory is not writable".into()
            },
        )
        .await;
    }

    // 17. Config drift: running gateway vs disk config.toml
    {
        let (in_sync, detail) = check_config_drift(&config_path).await;
        check(&tx, "Config drift", in_sync, detail).await;
    }

    // 18. Fallback model config
    {
        let has_fallback = check_fallback_config(&config_path, &zeus_dir);
        check(
            &tx,
            "LLM fallback chain",
            has_fallback,
            if has_fallback {
                "Fallback models configured".into()
            } else {
                "No fallback_models configured — consider adding for automatic failover".into()
            },
        )
        .await;
    }

    // 19. Telegram MTProto vs Bot API relay check
    {
        let (ok, detail) = check_telegram_mtproto_config(&config_path);
        check(&tx, "Telegram config", ok, detail).await;
    }

    // === Repair pass ===
    if repair {
        let outcomes = perform_repairs_sync(&workspace, &sessions, &config_path, &zeus_dir);
        let fixed = outcomes.len();
        for o in outcomes {
            repair_action(&tx, &o.name, o.success, o.detail).await;
        }
        let summary = if fixed == 0 {
            "Nothing to fix — all checks passed".to_string()
        } else {
            format!("{} issue(s) fixed", fixed)
        };
        repair_action(&tx, "Repair summary", true, summary).await;
    }

    tx.send(ProgressEvent::StepCompleted {
        name: if repair {
            "Diagnostics + Repair".into()
        } else {
            "Diagnostics".into()
        },
        message: "Complete".into(),
    })
    .await?;

    tx.send(ProgressEvent::Finished {
        success: true,
        elapsed: start.elapsed(),
        summary: if repair {
            "Diagnostics + repair complete".into()
        } else {
            "Diagnostics complete".into()
        },
    })
    .await?;

    Ok(())
}

async fn check(tx: &mpsc::Sender<ProgressEvent>, name: &str, ok: bool, detail: String) {
    let _ = tx
        .send(ProgressEvent::DoctorCheck {
            name: name.into(),
            ok,
            detail,
        })
        .await;
}

async fn repair_action(tx: &mpsc::Sender<ProgressEvent>, name: &str, success: bool, detail: String) {
    let _ = tx
        .send(ProgressEvent::DoctorRepair {
            name: name.into(),
            success,
            detail,
        })
        .await;
}

// ---------------------------------------------------------------------------
// Shared sync repair logic (used by both TUI doctor and CLI run_doctor)
// ---------------------------------------------------------------------------

/// A single repair action outcome — returned by [`perform_repairs_sync`].
pub struct RepairOutcome {
    pub success: bool,
    pub name: String,
    pub detail: String,
}

/// Execute all R1-R5 repair actions synchronously and return outcomes.
///
/// Callers translate outcomes to their output mechanism (mpsc for TUI,
/// `println!` for CLI). This is the single source of truth for repair logic.
pub fn perform_repairs_sync(
    workspace: &std::path::Path,
    sessions: &std::path::Path,
    config_path: &std::path::Path,
    zeus_dir: &std::path::Path,
) -> Vec<RepairOutcome> {
    let mut outcomes: Vec<RepairOutcome> = Vec::new();

    // R1: Workspace directory + subdirs
    if !workspace.exists() {
        let ok = std::fs::create_dir_all(workspace).is_ok();
        outcomes.push(RepairOutcome {
            success: ok,
            name: "Create workspace".into(),
            detail: if ok { workspace.display().to_string() } else { "Permission denied".into() },
        });
    }
    for subdir in &["memory", "daily"] {
        let dir = workspace.join(subdir);
        if !dir.exists() && std::fs::create_dir_all(&dir).is_ok() {
            outcomes.push(RepairOutcome {
                success: true,
                name: format!("Create {}", subdir),
                detail: dir.display().to_string(),
            });
        }
    }
    // R1b: Template files
    let templates: &[(&str, &str)] = &[
        ("AGENTS.md", "# Agent Definitions\n"),
        ("SOUL.md", "# Zeus Personality\n\nYou are Zeus, an autonomous AI assistant.\n"),
        ("USER.md", "# User Context\n"),
        ("HEARTBEAT.md", "# Heartbeat Tasks\n\n- Check for pending tasks\n"),
        ("memory/MEMORY.md", "# Long-term Memory\n"),
    ];
    for (file, content) in templates {
        let path = workspace.join(file);
        if !path.exists() && std::fs::write(&path, content).is_ok() {
            outcomes.push(RepairOutcome {
                success: true,
                name: format!("Create {}", file),
                detail: "Template created".into(),
            });
        }
    }

    // R2: Sessions directory
    if !sessions.exists() && std::fs::create_dir_all(sessions).is_ok() {
        outcomes.push(RepairOutcome {
            success: true,
            name: "Create sessions".into(),
            detail: sessions.display().to_string(),
        });
    }

    // R3: Orphaned (0-byte) JSONL files
    for path in find_orphaned_jsonl(sessions) {
        let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        if std::fs::remove_file(&path).is_ok() {
            outcomes.push(RepairOutcome {
                success: true,
                name: "Remove orphaned JSONL".into(),
                detail: name,
            });
        }
    }

    // R4: Config + .env file permissions (Unix — chmod 600)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let env_path = zeus_dir.join(".env");
        for path in &[config_path, env_path.as_path()] {
            if path.exists()
                && let Ok(meta) = std::fs::metadata(path)
            {
                let mode = meta.permissions().mode();
                if mode & 0o077 != 0 {
                    let mut perms = meta.permissions();
                    perms.set_mode(0o600);
                    if std::fs::set_permissions(path, perms).is_ok() {
                        outcomes.push(RepairOutcome {
                            success: true,
                            name: "Fix permissions".into(),
                            detail: format!("{}: {:o} -> 600", path.display(), mode & 0o777),
                        });
                    }
                }
            }
        }
    }

    // R5: Deprecated config key renames (line-aware TOML key matching)
    if config_path.exists()
        && let Ok(content) = std::fs::read_to_string(config_path)
    {
        let renames = [
            ("session_dir", "sessions"),
            ("workspace_dir", "workspace"),
            ("max_turns", "max_iterations"),
        ];
        let new_lines: Vec<String> = content
            .lines()
            .map(|line| {
                let trimmed = line.trim_start();
                for (old, new) in &renames {
                    if let Some(rest) = trimmed.strip_prefix(old)
                        && rest.starts_with(['=', ' '])
                    {
                        return line.replacen(old, new, 1);
                    }
                }
                line.to_string()
            })
            .collect();
        let updated = new_lines.join("\n");
        let mut any_changed = false;
        for (old, new) in &renames {
            if content.lines().zip(updated.lines()).any(|(a, b)| a != b && a.contains(old)) {
                any_changed = true;
                outcomes.push(RepairOutcome {
                    success: true,
                    name: "Rename config key".into(),
                    detail: format!("'{}' -> '{}'", old, new),
                });
            }
        }
        if any_changed {
            let _ = std::fs::write(config_path, &updated);
        }
    }

    outcomes
}

/// Collect paths of 0-byte JSONL files (orphaned sessions).
fn find_orphaned_jsonl(sessions_dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let Ok(entries) = std::fs::read_dir(sessions_dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            if path.extension().and_then(|x| x.to_str()) == Some("jsonl")
                && e.metadata().map(|m| m.len() == 0).unwrap_or(false)
            {
                Some(path)
            } else {
                None
            }
        })
        .collect()
}

async fn check_port(host: &str, port: u16) -> bool {
    let url = format!("http://{}:{}/health", host, port);
    match reqwest::Client::new()
        .get(&url)
        .timeout(std::time::Duration::from_secs(2))
        .send()
        .await
    {
        Ok(resp) => resp.status().is_success(),
        Err(_) => false,
    }
}

/// Count sessions that haven't been written to in `max_age_days`.
/// Returns `(stale_count, total_count)`.
fn count_stale_sessions(sessions_dir: &std::path::Path, max_age_days: u64) -> (usize, usize) {
    let Ok(entries) = std::fs::read_dir(sessions_dir) else {
        return (0, 0);
    };
    let cutoff = std::time::SystemTime::now()
        .checked_sub(std::time::Duration::from_secs(max_age_days * 86400))
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);

    let mut total = 0;
    let mut stale = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        total += 1;
        let modified = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        if modified < cutoff {
            stale += 1;
        }
    }
    (stale, total)
}

/// Count empty (0-byte) JSONL files — these are orphaned sessions with no messages.
fn count_orphaned_jsonl(sessions_dir: &std::path::Path) -> usize {
    let Ok(entries) = std::fs::read_dir(sessions_dir) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|e| {
            e.path().extension().and_then(|x| x.to_str()) == Some("jsonl")
                && e.metadata().map(|m| m.len() == 0).unwrap_or(false)
        })
        .count()
}

/// Scan config.toml for deprecated keys and return human-readable warnings.
fn check_deprecated_config_keys(config_path: &std::path::Path) -> Vec<String> {
    let Ok(content) = std::fs::read_to_string(config_path) else {
        return vec![];
    };
    let mut warnings = Vec::new();

    // `ollama_url` at top level was moved to `[ollama] url`
    if content.contains("ollama_url") && !content.contains("[ollama]") {
        warnings.push("'ollama_url' is deprecated — use '[ollama]\\nurl = ...' instead".into());
    }
    // `session_dir` renamed to `sessions`
    if content.contains("session_dir") {
        warnings.push("'session_dir' is deprecated — rename to 'sessions'".into());
    }
    // `workspace_dir` renamed to `workspace`
    if content.contains("workspace_dir") {
        warnings.push("'workspace_dir' is deprecated — rename to 'workspace'".into());
    }
    // `max_turns` renamed to `max_iterations`
    if content.contains("max_turns") {
        warnings.push("'max_turns' is deprecated — rename to 'max_iterations'".into());
    }
    // `log_level` moved to `[logging] level`
    if content.contains("log_level =") {
        warnings.push("'log_level' is deprecated — use '[logging]\\nlevel = ...' instead".into());
    }

    warnings
}

/// Check whether a directory is readable and writable by the current process.
fn check_dir_permissions(dir: &std::path::Path) -> (bool, bool) {
    if !dir.exists() {
        return (false, false);
    }
    // Readable: can list directory entries
    let readable = std::fs::read_dir(dir).is_ok();
    // Writable: can create a temp file inside
    let probe = dir.join(".zeus_doctor_probe");
    let writable = std::fs::write(&probe, b"").is_ok();
    if writable {
        let _ = std::fs::remove_file(&probe);
    }
    (readable, writable)
}

/// Detect drift between the on-disk `config.toml` and the running gateway's
/// live config by comparing content hashes.
///
/// Queries `GET http://127.0.0.1:8080/v1/config` and hashes the response body,
/// then hashes the local config file.  Returns `(in_sync, detail_message)`.
async fn check_config_drift(config_path: &std::path::Path) -> (bool, String) {
    let Ok(disk_content) = std::fs::read_to_string(config_path) else {
        return (true, "Config file not found — skipping drift check".into());
    };

    let disk_hash = quick_hash(disk_content.trim().as_bytes());

    match reqwest::Client::new()
        .get(format!(
            "{}/v1/config",
            std::env::var("ZEUS_GATEWAY_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:8080".to_string())
        ))
        .timeout(std::time::Duration::from_secs(2))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => match resp.text().await {
            Ok(body) => {
                let live_hash = quick_hash(body.trim().as_bytes());
                if disk_hash == live_hash {
                    (true, "Config on disk matches running gateway".into())
                } else {
                    (
                        false,
                        "Config on disk differs from running gateway — reload required \
                             (run: zeus gateway restart)"
                            .into(),
                    )
                }
            }
            Err(e) => (true, format!("Could not read gateway config response: {e}")),
        },
        _ => (
            true,
            "Gateway not reachable — skipping config drift check".into(),
        ),
    }
}

/// Fast non-cryptographic hash for change detection.
fn quick_hash(data: &[u8]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    data.hash(&mut hasher);
    hasher.finish()
}

async fn check_service(platform: &Platform) -> bool {
    let output = match platform.os {
        crate::platform::Os::MacOS => {
            tokio::process::Command::new("launchctl")
                .args(["list"])
                .output()
                .await
        }
        crate::platform::Os::FreeBSD => {
            tokio::process::Command::new("service")
                .args(["zeus_gateway", "status"])
                .output()
                .await
        }
        crate::platform::Os::Linux => {
            tokio::process::Command::new("systemctl")
                .args(["is-active", "zeus-gateway"])
                .output()
                .await
        }
    };

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            match platform.os {
                crate::platform::Os::MacOS => stdout.contains("ai.zeus.gateway"),
                _ => out.status.success(),
            }
        }
        Err(_) => false,
    }
}

/// Check whether `fallback_models` is configured in config.toml.
///
/// Returns `true` if `fallback_models` is present and non-empty, indicating
/// the user has automatic LLM failover configured.
fn check_fallback_config(config_path: &std::path::Path, zeus_dir: &std::path::Path) -> bool {
    let content = std::fs::read_to_string(config_path).unwrap_or_default();
    // Check for a non-empty fallback_models array in the TOML
    if let Ok(table) = content.parse::<toml::Table>()
        && let Some(toml::Value::Array(arr)) = table.get("fallback_models")
    {
        return !arr.is_empty();
    }
    // Also check if the .env file has multiple provider keys (advisory info)
    let env_path = zeus_dir.join(".env");
    let env_content = std::fs::read_to_string(env_path).unwrap_or_default();
    let provider_keys = [
        "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY",
        "GROQ_API_KEY",
        "GOOGLE_API_KEY",
        "MISTRAL_API_KEY",
        "OPENROUTER_API_KEY",
        "TOGETHER_API_KEY",
        "FIREWORKS_API_KEY",
    ];
    let keys_present = provider_keys
        .iter()
        .filter(|k| {
            env_content
                .lines()
                .any(|line| line.starts_with(*k) && line.contains('='))
                || std::env::var(*k).is_ok()
        })
        .count();
    // If only 0 or 1 key, no fallback is possible anyway → report as ok
    keys_present <= 1
}

/// Check Telegram configuration for spurious MTProto warnings.
///
/// When only `[telegram_relay]` (Bot API) is configured and `[channels.telegram]`
/// (MTProto) is absent, MTProto-related warnings (api_id=0, api_hash empty) are
/// misleading — the user doesn't need MTProto credentials at all.
///
/// Returns `(ok, detail)`:
/// - `(true, "Bot API relay only — no MTProto needed")` when relay-only
/// - `(true, "MTProto configured with credentials")` when MTProto is properly set up
/// - `(false, "...")` when MTProto is configured but missing credentials
/// - `(true, "No Telegram configured")` when neither section exists
fn check_telegram_mtproto_config(config_path: &std::path::Path) -> (bool, String) {
    let Ok(content) = std::fs::read_to_string(config_path) else {
        return (true, "Config file not found — skipping Telegram check".into());
    };

    let has_telegram_relay = content.contains("[telegram_relay]");
    let has_channels_telegram = content.contains("[channels.telegram]");

    match (has_telegram_relay, has_channels_telegram) {
        (false, false) => (true, "No Telegram configured".into()),
        (true, false) => {
            // Bot API relay only — no MTProto needed, suppress warnings
            (true, "Bot API relay only — no MTProto needed".into())
        }
        (false, true) | (true, true) => {
            // MTProto channel is configured — check credentials
            if let Ok(table) = content.parse::<toml::Table>() {
                // Navigate to channels.telegram
                if let Some(toml::Value::Table(channels)) = table.get("channels") {
                    if let Some(toml::Value::Table(tg)) = channels.get("telegram") {
                        let api_id = tg
                            .get("api_id")
                            .and_then(|v| v.as_integer())
                            .unwrap_or(0);
                        let api_hash = tg
                            .get("api_hash")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let phone = tg
                            .get("phone")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let bot_token = tg
                            .get("bot_token")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");

                        // Bot token mode: api_id/api_hash not required
                        if !bot_token.is_empty() {
                            return (true, "MTProto bot mode — bot_token configured".into());
                        }

                        // User mode: need api_id, api_hash, and phone
                        let mut issues = Vec::new();
                        if api_id == 0 {
                            issues.push("api_id is 0");
                        }
                        if api_hash.is_empty() {
                            issues.push("api_hash is empty");
                        }
                        if phone.is_empty() {
                            issues.push("phone is empty");
                        }

                        if issues.is_empty() {
                            (true, "MTProto configured with credentials".into())
                        } else {
                            (false, format!("MTProto incomplete: {}", issues.join(", ")))
                        }
                    } else {
                        (true, "MTProto section present but empty".into())
                    }
                } else {
                    // [channels.telegram] exists in text but not parsed as table
                    // (could be commented out or malformed)
                    (true, "Telegram channels section found".into())
                }
            } else {
                (true, "Cannot parse config for Telegram check".into())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_find_orphaned_jsonl_returns_empty_files() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("empty1.jsonl"), b"").unwrap();
        std::fs::write(dir.path().join("empty2.jsonl"), b"").unwrap();
        std::fs::write(dir.path().join("full.jsonl"), b"{\"data\":true}\n").unwrap();
        std::fs::write(dir.path().join("readme.txt"), b"").unwrap(); // not jsonl

        let orphaned = find_orphaned_jsonl(dir.path());
        assert_eq!(orphaned.len(), 2);
        assert!(orphaned.iter().all(|p| p.extension().unwrap() == "jsonl"));
    }

    #[test]
    fn test_find_orphaned_jsonl_empty_dir() {
        let dir = tempdir().unwrap();
        let orphaned = find_orphaned_jsonl(dir.path());
        assert!(orphaned.is_empty());
    }

    #[test]
    fn test_find_orphaned_jsonl_nonexistent_dir() {
        let orphaned = find_orphaned_jsonl(std::path::Path::new("/nonexistent/xyz"));
        assert!(orphaned.is_empty());
    }

    #[test]
    fn test_count_stale_sessions_empty_dir() {
        let dir = tempdir().unwrap();
        let (stale, total) = count_stale_sessions(dir.path(), 30);
        assert_eq!(stale, 0);
        assert_eq!(total, 0);
    }

    #[test]
    fn test_count_orphaned_jsonl_empty_file() {
        let dir = tempdir().unwrap();
        // Create one empty and one non-empty JSONL file
        std::fs::write(dir.path().join("empty.jsonl"), b"").unwrap();
        std::fs::write(dir.path().join("full.jsonl"), b"{\"role\":\"user\"}\n").unwrap();
        assert_eq!(count_orphaned_jsonl(dir.path()), 1);
    }

    #[test]
    fn test_count_orphaned_jsonl_no_orphans() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.jsonl"), b"data\n").unwrap();
        assert_eq!(count_orphaned_jsonl(dir.path()), 0);
    }

    #[test]
    fn test_deprecated_keys_none() {
        let dir = tempdir().unwrap();
        let cfg = dir.path().join("config.toml");
        std::fs::write(&cfg, "[ollama]\nurl = \"http://localhost:11434\"\n").unwrap();
        assert!(check_deprecated_config_keys(&cfg).is_empty());
    }

    #[test]
    fn test_deprecated_keys_detected() {
        let dir = tempdir().unwrap();
        let cfg = dir.path().join("config.toml");
        std::fs::write(
            &cfg,
            "ollama_url = \"http://localhost:11434\"\nsession_dir = \"/tmp\"\nmax_turns = 20\n",
        )
        .unwrap();
        let warnings = check_deprecated_config_keys(&cfg);
        assert!(warnings.iter().any(|w| w.contains("ollama_url")));
        assert!(warnings.iter().any(|w| w.contains("session_dir")));
        assert!(warnings.iter().any(|w| w.contains("max_turns")));
    }

    #[test]
    fn test_dir_permissions_existing() {
        let dir = tempdir().unwrap();
        let (readable, writable) = check_dir_permissions(dir.path());
        assert!(readable, "temp dir should be readable");
        assert!(writable, "temp dir should be writable");
    }

    #[test]
    fn test_dir_permissions_nonexistent() {
        let (readable, writable) = check_dir_permissions(std::path::Path::new("/nonexistent/xyz"));
        assert!(!readable);
        assert!(!writable);
    }

    #[test]
    fn test_fallback_config_with_fallback_models() {
        let dir = tempdir().unwrap();
        let cfg = dir.path().join("config.toml");
        std::fs::write(
            &cfg,
            "model = \"anthropic/claude-sonnet-4-20250514\"\nfallback_models = [\"openai/gpt-4o\"]\n",
        )
        .unwrap();
        assert!(check_fallback_config(&cfg, dir.path()));
    }

    #[test]
    fn test_fallback_config_without_fallback_and_single_key() {
        let dir = tempdir().unwrap();
        let cfg = dir.path().join("config.toml");
        std::fs::write(&cfg, "model = \"anthropic/claude-sonnet-4-20250514\"\n").unwrap();
        // No .env with multiple keys → ok (no fallback possible anyway)
        assert!(check_fallback_config(&cfg, dir.path()));
    }

    #[test]
    fn test_fallback_config_without_fallback_and_multiple_keys() {
        let dir = tempdir().unwrap();
        let cfg = dir.path().join("config.toml");
        std::fs::write(&cfg, "model = \"anthropic/claude-sonnet-4-20250514\"\n").unwrap();
        let env_path = dir.path().join(".env");
        std::fs::write(
            &env_path,
            "ANTHROPIC_API_KEY=sk-ant-xxx\nOPENAI_API_KEY=sk-xxx\n",
        )
        .unwrap();
        // Multiple keys but no fallback_models → should warn (returns false)
        assert!(!check_fallback_config(&cfg, dir.path()));
    }

    #[test]
    fn test_telegram_mtproto_no_telegram() {
        let dir = tempdir().unwrap();
        let cfg = dir.path().join("config.toml");
        std::fs::write(&cfg, "model = \"test\"\n").unwrap();
        let (ok, detail) = check_telegram_mtproto_config(&cfg);
        assert!(ok);
        assert!(detail.contains("No Telegram"));
    }

    #[test]
    fn test_telegram_mtproto_relay_only() {
        let dir = tempdir().unwrap();
        let cfg = dir.path().join("config.toml");
        std::fs::write(
            &cfg,
            "[telegram_relay]\nbot_token = \"123:abc\"\nchat_id = \"-100123\"\n",
        )
        .unwrap();
        let (ok, detail) = check_telegram_mtproto_config(&cfg);
        assert!(ok);
        assert!(detail.contains("Bot API relay only"));
        assert!(detail.contains("no MTProto needed"));
    }

    #[test]
    fn test_telegram_mtproto_configured_with_credentials() {
        let dir = tempdir().unwrap();
        let cfg = dir.path().join("config.toml");
        std::fs::write(
            &cfg,
            "[channels.telegram]\napi_id = 12345\napi_hash = \"abc123\"\nphone = \"+1234567890\"\n",
        )
        .unwrap();
        let (ok, detail) = check_telegram_mtproto_config(&cfg);
        assert!(ok);
        assert!(detail.contains("MTProto configured with credentials"));
    }

    #[test]
    fn test_telegram_mtproto_incomplete_credentials() {
        let dir = tempdir().unwrap();
        let cfg = dir.path().join("config.toml");
        std::fs::write(
            &cfg,
            "[channels.telegram]\napi_id = 0\napi_hash = \"\"\nphone = \"\"\n",
        )
        .unwrap();
        let (ok, detail) = check_telegram_mtproto_config(&cfg);
        assert!(!ok);
        assert!(detail.contains("api_id is 0"));
        assert!(detail.contains("api_hash is empty"));
        assert!(detail.contains("phone is empty"));
    }

    #[test]
    fn test_telegram_mtproto_bot_token_mode() {
        let dir = tempdir().unwrap();
        let cfg = dir.path().join("config.toml");
        std::fs::write(
            &cfg,
            "[channels.telegram]\napi_id = 0\napi_hash = \"\"\nbot_token = \"123:abc\"\n",
        )
        .unwrap();
        let (ok, detail) = check_telegram_mtproto_config(&cfg);
        assert!(ok);
        assert!(detail.contains("bot_token configured"));
    }

    #[test]
    fn test_telegram_mtproto_both_relay_and_channels() {
        let dir = tempdir().unwrap();
        let cfg = dir.path().join("config.toml");
        std::fs::write(
            &cfg,
            "[telegram_relay]\nbot_token = \"123:abc\"\nchat_id = \"-100123\"\n\n[channels.telegram]\napi_id = 12345\napi_hash = \"abc123\"\nphone = \"+1234567890\"\n",
        )
        .unwrap();
        let (ok, detail) = check_telegram_mtproto_config(&cfg);
        assert!(ok);
        assert!(detail.contains("MTProto configured with credentials"));
    }

    #[test]
    fn test_quick_hash_deterministic() {
        let h1 = quick_hash(b"hello world");
        let h2 = quick_hash(b"hello world");
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_quick_hash_differs_on_change() {
        let h1 = quick_hash(b"config v1");
        let h2 = quick_hash(b"config v2");
        assert_ne!(h1, h2);
    }
}
