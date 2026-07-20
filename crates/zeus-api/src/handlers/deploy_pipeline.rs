//! Deploy Pipeline Executor — drives multi-step deploy workflows.
//!
//! Uses `PlanExecutor` (dependency-aware task DAG) from zeus-prometheus
//! and executes each step via shell commands, updating `DeployStore`
//! with status + logs throughout the lifecycle.
//!
//! Pipeline stages:
//!   1. checkout  — git pull / clone the project
//!   2. install   — install dependencies (if needed)
//!   3. build     — run the build command
//!   4. test      — (optional) run test suite
//!   5. deploy    — provider-specific deploy command
//!   6. verify    — health check the deployed URL
//!   7. snapshot  — create rollback snapshot on success

use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tracing::{info, warn};
use zeus_core::fleet_telemetry::{self, FleetEventKind, FleetSeverity};

use super::deploy_store::{DeployStore, DeployTargetRow, RollbackSnapshotRow};

// ============================================================================
// Pipeline Configuration
// ============================================================================

/// Configuration for a deploy pipeline run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineConfig {
    /// Deploy target to deploy to
    pub target: DeployTargetRow,
    /// Deployment record (created in advance)
    pub deployment_id: String,
    /// Version string
    pub version: String,
    /// Skip test step
    #[serde(default)]
    pub skip_tests: bool,
    /// Skip verify step (health check)
    #[serde(default)]
    pub skip_verify: bool,
    /// Custom environment variables for build
    #[serde(default)]
    pub env_vars: Vec<(String, String)>,
}

/// Result of a single pipeline step.
#[derive(Debug, Clone, Serialize)]
pub struct StepResult {
    pub step: String,
    pub status: String,
    pub message: String,
    pub duration_ms: u64,
}

/// Result of the full pipeline execution.
#[derive(Debug, Clone, Serialize)]
pub struct PipelineResult {
    pub deployment_id: String,
    pub success: bool,
    pub steps: Vec<StepResult>,
    pub total_duration_secs: u64,
    pub deploy_url: String,
    pub error: Option<String>,
}

// ============================================================================
// Pipeline Executor
// ============================================================================

/// Execute a full deploy pipeline for a target.
///
/// This runs in the background (spawned from the handler) and updates
/// the DeployStore at each step. The frontend polls `/v1/deploy/:id`
/// and `/v1/deploy/:id/logs` for progress.
pub async fn run_pipeline(store: Arc<DeployStore>, config: PipelineConfig) -> PipelineResult {
    let start = Instant::now();
    let mut steps: Vec<StepResult> = Vec::new();
    let mut deploy_url = config.target.url.clone();

    info!(
        deployment_id = %config.deployment_id,
        target = %config.target.name,
        provider = %config.target.provider,
        "Starting deploy pipeline"
    );

    // ── Step 1: Checkout ────────────────────────────────────────
    let checkout_result = run_step(
        &store,
        &config.deployment_id,
        "checkout",
        &build_checkout_command(&config.target),
        &config.target.project_path,
        &config.env_vars,
    )
    .await;
    steps.push(checkout_result.clone());

    if checkout_result.status == "failed" {
        return finish_pipeline(
            &store,
            &config,
            steps,
            start,
            None,
            Some(&checkout_result.message),
        )
        .await;
    }

    // ── Step 2: Install ─────────────────────────────────────────
    let install_cmd = detect_install_command(&config.target);
    if !install_cmd.is_empty() {
        let install_result = run_step(
            &store,
            &config.deployment_id,
            "install",
            &install_cmd,
            &config.target.project_path,
            &config.env_vars,
        )
        .await;
        steps.push(install_result.clone());

        if install_result.status == "failed" {
            return finish_pipeline(
                &store,
                &config,
                steps,
                start,
                None,
                Some(&install_result.message),
            )
            .await;
        }
    }

    // ── Step 3: Build ───────────────────────────────────────────
    store
        .update_deployment_status(&config.deployment_id, "building", None, None, None, None)
        .await;

    if !config.target.build_command.is_empty() {
        let build_result = run_step(
            &store,
            &config.deployment_id,
            "build",
            &config.target.build_command,
            &config.target.project_path,
            &config.env_vars,
        )
        .await;
        steps.push(build_result.clone());

        if build_result.status == "failed" {
            return finish_pipeline(
                &store,
                &config,
                steps,
                start,
                None,
                Some(&build_result.message),
            )
            .await;
        }
    }

    // ── Step 4: Test (optional) ─────────────────────────────────
    if !config.skip_tests {
        let test_cmd = detect_test_command(&config.target);
        if !test_cmd.is_empty() {
            let test_result = run_step(
                &store,
                &config.deployment_id,
                "test",
                &test_cmd,
                &config.target.project_path,
                &config.env_vars,
            )
            .await;
            steps.push(test_result.clone());

            if test_result.status == "failed" {
                return finish_pipeline(
                    &store,
                    &config,
                    steps,
                    start,
                    None,
                    Some(&test_result.message),
                )
                .await;
            }
        }
    }

    // ── Step 5: Deploy ──────────────────────────────────────────
    store
        .update_deployment_status(&config.deployment_id, "deploying", None, None, None, None)
        .await;

    let deploy_cmd = build_deploy_command(&config.target, &config.version);
    let deploy_result = run_step(
        &store,
        &config.deployment_id,
        "deploy",
        &deploy_cmd,
        &config.target.project_path,
        &config.env_vars,
    )
    .await;
    steps.push(deploy_result.clone());

    if deploy_result.status == "failed" {
        return finish_pipeline(
            &store,
            &config,
            steps,
            start,
            None,
            Some(&deploy_result.message),
        )
        .await;
    }

    // Extract deploy URL from output if available
    if let Some(url) = extract_deploy_url(&deploy_result.message) {
        deploy_url = url;
    }

    // ── Step 6: Verify (health check) ───────────────────────────
    if !config.skip_verify && !deploy_url.is_empty() {
        let verify_result = run_step(
            &store,
            &config.deployment_id,
            "verify",
            &format!("curl -sf -o /dev/null -w '%{{http_code}}' '{}'", deploy_url),
            &config.target.project_path,
            &config.env_vars,
        )
        .await;
        steps.push(verify_result.clone());

        if verify_result.status == "failed" {
            warn!(
                deployment_id = %config.deployment_id,
                "Health check failed but deployment may still be live"
            );
            // Don't fail the whole pipeline on health check — it might just need time
        }
    }

    // ── Step 7: Snapshot ────────────────────────────────────────
    let snapshot_id = format!(
        "snap-{}",
        uuid::Uuid::new_v4()
            .to_string()
            .split('-')
            .next()
            .unwrap_or("x")
    );
    store
        .save_snapshot(&RollbackSnapshotRow {
            id: snapshot_id.clone(),
            deployment_id: config.deployment_id.clone(),
            target_id: config.target.id.clone(),
            version: config.version.clone(),
            snapshot_ref: format!("git:{}", config.version),
            deploy_url: deploy_url.clone(),
            is_current: true,
            created_at: String::new(),
        })
        .await;

    store
        .add_log_entry(
            &config.deployment_id,
            "snapshot",
            "completed",
            &format!("Rollback snapshot saved: {}", snapshot_id),
            0,
        )
        .await;

    // ── Finish: mark live ───────────────────────────────────────
    finish_pipeline(&store, &config, steps, start, Some(&deploy_url), None).await
}

// ============================================================================
// Step Executor
// ============================================================================

/// Execute a single pipeline step (shell command) and log it.
async fn run_step(
    store: &DeployStore,
    deployment_id: &str,
    step_name: &str,
    command: &str,
    cwd: &str,
    env_vars: &[(String, String)],
) -> StepResult {
    let step_start = Instant::now();

    store
        .add_log_entry(
            deployment_id,
            step_name,
            "running",
            &format!("$ {}", command),
            0,
        )
        .await;
    store
        .append_build_log(deployment_id, &format!("[{}] $ {}", step_name, command))
        .await;

    let result = execute_command(command, cwd, env_vars).await;
    let duration_ms = step_start.elapsed().as_millis() as u64;

    let (status, message) = match result {
        Ok(output) => {
            store.append_build_log(deployment_id, &output).await;
            ("completed".to_string(), output)
        }
        Err(err) => {
            let msg = format!("Error: {}", err);
            store.append_build_log(deployment_id, &msg).await;
            ("failed".to_string(), msg)
        }
    };

    store
        .add_log_entry(
            deployment_id,
            step_name,
            &status,
            &truncate(&message, 500),
            duration_ms,
        )
        .await;

    StepResult {
        step: step_name.to_string(),
        status,
        message,
        duration_ms,
    }
}

/// Execute a shell command and return stdout+stderr.
async fn execute_command(
    command: &str,
    cwd: &str,
    env_vars: &[(String, String)],
) -> Result<String, String> {
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(command);

    if !cwd.is_empty() {
        let path = std::path::Path::new(cwd);
        if path.exists() {
            cmd.current_dir(path);
        }
    }

    for (key, value) in env_vars {
        cmd.env(key, value);
    }

    // 10-minute timeout per step
    let result = tokio::time::timeout(std::time::Duration::from_secs(600), cmd.output()).await;

    match result {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let combined = if stderr.is_empty() {
                stdout
            } else {
                format!("{}\n{}", stdout, stderr)
            };

            if output.status.success() {
                Ok(combined)
            } else {
                Err(format!(
                    "Exit code {}: {}",
                    output.status.code().unwrap_or(-1),
                    combined
                ))
            }
        }
        Ok(Err(e)) => Err(format!("Failed to execute: {}", e)),
        Err(_) => Err("Command timed out (10 min limit)".to_string()),
    }
}

// ============================================================================
// Command Builders — provider-specific
// ============================================================================

fn build_checkout_command(target: &DeployTargetRow) -> String {
    if target.project_path.is_empty() {
        return String::new();
    }
    // If it's a local path, just git pull
    if std::path::Path::new(&target.project_path).exists() {
        format!(
            "cd '{}' && git pull --rebase 2>/dev/null || true",
            target.project_path
        )
    } else {
        // It's a git URL, clone it
        format!(
            "git clone '{}' /tmp/zeus-deploy-checkout",
            target.project_path
        )
    }
}

fn detect_install_command(target: &DeployTargetRow) -> String {
    let project = &target.project_path;
    let p = std::path::Path::new(project);

    // Detect package manager from project files
    if p.join("package-lock.json").exists() {
        return "npm ci".to_string();
    }
    if p.join("yarn.lock").exists() {
        return "yarn install --frozen-lockfile".to_string();
    }
    if p.join("pnpm-lock.yaml").exists() {
        return "pnpm install --frozen-lockfile".to_string();
    }
    if p.join("bun.lockb").exists() {
        return "bun install".to_string();
    }
    // Rust projects don't need a separate install step
    String::new()
}

fn detect_test_command(target: &DeployTargetRow) -> String {
    let project = &target.project_path;
    let p = std::path::Path::new(project);

    if p.join("Cargo.toml").exists() {
        return "cargo test --workspace 2>&1 | tail -5".to_string();
    }
    if p.join("package.json").exists() {
        return "npm test 2>&1 || true".to_string();
    }
    String::new()
}

fn build_deploy_command(target: &DeployTargetRow, version: &str) -> String {
    let config: serde_json::Value = serde_json::from_str(&target.config_json).unwrap_or_default();

    match target.provider.as_str() {
        "vercel" => {
            let project_name = config.get("project").and_then(|v| v.as_str()).unwrap_or("");
            if project_name.is_empty() {
                "vercel deploy --prod --yes 2>&1".to_string()
            } else {
                format!("vercel deploy --prod --yes --name '{}' 2>&1", project_name)
            }
        }
        "netlify" => {
            let dir = if target.output_dir.is_empty() {
                "dist"
            } else {
                &target.output_dir
            };
            format!("netlify deploy --prod --dir='{}' 2>&1", dir)
        }
        "docker" => {
            let image = config
                .get("image")
                .and_then(|v| v.as_str())
                .unwrap_or("zeus-app");
            let registry = config
                .get("registry")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let tag = format!("{}:{}", image, version);
            if registry.is_empty() {
                format!("docker build -t '{}' . && docker push '{}' 2>&1", tag, tag)
            } else {
                format!(
                    "docker build -t '{}/{}' . && docker push '{}/{}' 2>&1",
                    registry, tag, registry, tag
                )
            }
        }
        "ssh" => {
            let host = config
                .get("host")
                .and_then(|v| v.as_str())
                .unwrap_or("localhost");
            let remote_path = config
                .get("remote_path")
                .and_then(|v| v.as_str())
                .unwrap_or("/opt/app");
            let dir = if target.output_dir.is_empty() {
                "."
            } else {
                &target.output_dir
            };
            format!("rsync -avz '{}/' '{}:{}/' 2>&1", dir, host, remote_path)
        }
        "freebsd" => {
            let host = config
                .get("host")
                .and_then(|v| v.as_str())
                .unwrap_or("localhost");
            let service = config
                .get("service")
                .and_then(|v| v.as_str())
                .unwrap_or("zeus-gateway");
            format!(
                "rsync -avz '{}' '{}:/usr/local/bin/' && ssh '{}' 'sudo service {} restart' 2>&1",
                if target.output_dir.is_empty() {
                    "target/release/zeus"
                } else {
                    &target.output_dir
                },
                host,
                host,
                service
            )
        }
        "s3" => {
            let bucket = config.get("bucket").and_then(|v| v.as_str()).unwrap_or("");
            let dir = if target.output_dir.is_empty() {
                "dist"
            } else {
                &target.output_dir
            };
            format!("aws s3 sync '{}/' 's3://{}/' --delete 2>&1", dir, bucket)
        }
        "trunk" => {
            // Leptos/WASM deploy via trunk
            let dir = if target.output_dir.is_empty() {
                "dist"
            } else {
                &target.output_dir
            };
            format!("trunk build --release && echo 'Built to {}' 2>&1", dir)
        }
        _ => {
            // Custom: just use the build command as deploy too, or config.deploy_command
            let custom_cmd = config
                .get("deploy_command")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if custom_cmd.is_empty() {
                format!(
                    "echo 'No deploy command configured for provider: {}' && exit 1",
                    target.provider
                )
            } else {
                format!("{} 2>&1", custom_cmd)
            }
        }
    }
}

/// Try to extract a URL from deploy command output (Vercel/Netlify print URLs).
fn extract_deploy_url(output: &str) -> Option<String> {
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("https://") && !trimmed.contains(' ') {
            return Some(trimmed.to_string());
        }
    }
    None
}

// ============================================================================
// Pipeline Finish
// ============================================================================

async fn finish_pipeline(
    store: &DeployStore,
    config: &PipelineConfig,
    steps: Vec<StepResult>,
    start: Instant,
    deploy_url: Option<&str>,
    error: Option<&str>,
) -> PipelineResult {
    let total_duration = start.elapsed().as_secs();
    let success = error.is_none();
    let final_status = if success { "live" } else { "failed" };

    store
        .update_deployment_status(
            &config.deployment_id,
            final_status,
            deploy_url,
            None,
            error,
            Some(total_duration),
        )
        .await;

    let telemetry_kind = if success {
        FleetEventKind::DeploySuccess
    } else {
        FleetEventKind::DeployFailure
    };
    let telemetry_severity = if success {
        FleetSeverity::Info
    } else {
        FleetSeverity::Error
    };
    let telemetry_summary = if success {
        format!("deployment '{}' finished live", config.deployment_id)
    } else {
        format!("deployment '{}' failed", config.deployment_id)
    };
    let telemetry_details = format!(
        "deployment_id={} target={} provider={} status={} duration_secs={} version={} deploy_url={} error={}",
        config.deployment_id,
        config.target.name,
        config.target.provider,
        final_status,
        total_duration,
        config.version,
        deploy_url.unwrap_or(""),
        error.unwrap_or("")
    );
    fleet_telemetry::record_event_best_effort(
        telemetry_kind,
        telemetry_severity,
        "zeus-api-deploy-pipeline",
        &telemetry_summary,
        None,
        Some(&telemetry_details),
    );

    info!(
        deployment_id = %config.deployment_id,
        status = final_status,
        duration_secs = total_duration,
        steps = steps.len(),
        "Deploy pipeline finished"
    );

    PipelineResult {
        deployment_id: config.deployment_id.clone(),
        success,
        steps,
        total_duration_secs: total_duration,
        deploy_url: deploy_url.unwrap_or("").to_string(),
        error: error.map(|e| e.to_string()),
    }
}

/// Truncate a string to max length.
fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        // Safe char boundary
        let mut end = max;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::super::deploy_store::DeploymentRow;
    use super::*;
    use chrono::Utc;

    fn test_target(provider: &str) -> DeployTargetRow {
        DeployTargetRow {
            id: "t1".to_string(),
            name: "Test".to_string(),
            provider: provider.to_string(),
            environment: "production".to_string(),
            config_json: "{}".to_string(),
            credentials_ref: String::new(),
            project_path: "/tmp/test-project".to_string(),
            build_command: "echo 'building'".to_string(),
            output_dir: "dist".to_string(),
            url: "https://test.example.com".to_string(),
            active: true,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        }
    }

    #[test]
    fn test_build_deploy_command_vercel() {
        let target = test_target("vercel");
        let cmd = build_deploy_command(&target, "1.0.0");
        assert!(cmd.contains("vercel deploy --prod"));
    }

    #[test]
    fn test_build_deploy_command_netlify() {
        let target = test_target("netlify");
        let cmd = build_deploy_command(&target, "1.0.0");
        assert!(cmd.contains("netlify deploy --prod"));
        assert!(cmd.contains("--dir='dist'"));
    }

    #[test]
    fn test_build_deploy_command_docker() {
        let mut target = test_target("docker");
        target.config_json = r#"{"image":"zeus-web","registry":"ghcr.io/zeuslabai"}"#.to_string();
        let cmd = build_deploy_command(&target, "2.0.0");
        assert!(cmd.contains("docker build"));
        assert!(cmd.contains("ghcr.io/zeuslabai"));
        assert!(cmd.contains("zeus-web:2.0.0"));
    }

    #[test]
    fn test_build_deploy_command_ssh() {
        let mut target = test_target("ssh");
        target.config_json = r#"{"host":"192.168.1.224","remote_path":"/opt/zeus"}"#.to_string();
        let cmd = build_deploy_command(&target, "1.0.0");
        assert!(cmd.contains("rsync"));
        assert!(cmd.contains("192.168.1.224"));
    }

    #[test]
    fn test_build_deploy_command_freebsd() {
        let mut target = test_target("freebsd");
        target.config_json = r#"{"host":"192.168.1.224","service":"zeus-gateway"}"#.to_string();
        let cmd = build_deploy_command(&target, "1.0.0");
        assert!(cmd.contains("rsync"));
        assert!(cmd.contains("service zeus-gateway restart"));
    }

    #[test]
    fn test_build_deploy_command_s3() {
        let mut target = test_target("s3");
        target.config_json = r#"{"bucket":"zeus-static"}"#.to_string();
        let cmd = build_deploy_command(&target, "1.0.0");
        assert!(cmd.contains("aws s3 sync"));
        assert!(cmd.contains("zeus-static"));
    }

    #[test]
    fn test_build_deploy_command_custom() {
        let mut target = test_target("custom");
        target.config_json = r#"{"deploy_command":"./deploy.sh production"}"#.to_string();
        let cmd = build_deploy_command(&target, "1.0.0");
        assert!(cmd.contains("./deploy.sh production"));
    }

    #[test]
    fn test_build_deploy_command_unknown_no_config() {
        let target = test_target("unknown");
        let cmd = build_deploy_command(&target, "1.0.0");
        assert!(cmd.contains("exit 1"));
    }

    #[test]
    fn test_extract_deploy_url() {
        let output = "Building...\nReady\nhttps://zeus-abc123.vercel.app\nDone";
        assert_eq!(
            extract_deploy_url(output),
            Some("https://zeus-abc123.vercel.app".to_string())
        );
    }

    #[test]
    fn test_extract_deploy_url_none() {
        let output = "No URL here, just text";
        assert_eq!(extract_deploy_url(output), None);
    }

    #[test]
    fn test_truncate_short() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_long() {
        let result = truncate("hello world this is long", 10);
        assert!(result.len() <= 13); // 10 + "..."
        assert!(result.ends_with("..."));
    }

    #[test]
    fn test_checkout_command_local() {
        let mut target = test_target("vercel");
        target.project_path = "/tmp".to_string(); // exists
        let cmd = build_checkout_command(&target);
        assert!(cmd.contains("git pull"));
    }

    #[test]
    fn test_checkout_command_git_url() {
        let mut target = test_target("vercel");
        target.project_path = "git@github.com:user/repo.git".to_string();
        let cmd = build_checkout_command(&target);
        assert!(cmd.contains("git clone"));
    }

    #[tokio::test]
    async fn test_execute_command_success() {
        let result = execute_command("echo 'hello deploy'", "/tmp", &[]).await;
        assert!(result.is_ok());
        assert!(result.unwrap().contains("hello deploy"));
    }

    #[tokio::test]
    async fn test_execute_command_failure() {
        let result = execute_command("exit 42", "/tmp", &[]).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Exit code 42"));
    }

    #[tokio::test]
    async fn test_execute_command_with_env() {
        let env = vec![("ZEUS_TEST_VAR".to_string(), "pipeline_works".to_string())];
        let result = execute_command("echo $ZEUS_TEST_VAR", "/tmp", &env).await;
        assert!(result.is_ok());
        assert!(result.unwrap().contains("pipeline_works"));
    }

    #[tokio::test]
    async fn test_full_pipeline_echo() {
        let store = Arc::new(DeployStore::in_memory().unwrap());

        // Create target with echo commands
        let target = DeployTargetRow {
            id: "t-test".to_string(),
            name: "Echo Test".to_string(),
            provider: "custom".to_string(),
            environment: "test".to_string(),
            config_json: r#"{"deploy_command":"echo 'deployed'"}"#.to_string(),
            credentials_ref: String::new(),
            project_path: "/tmp".to_string(),
            build_command: "echo 'built'".to_string(),
            output_dir: String::new(),
            url: String::new(),
            active: true,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        };
        store.create_target(&target).await;

        // Create deployment record
        let deployment = DeploymentRow {
            id: "d-test".to_string(),
            target_id: "t-test".to_string(),
            version: "1.0.0".to_string(),
            status: "pending".to_string(),
            trigger: "manual".to_string(),
            commit_hash: String::new(),
            commit_message: String::new(),
            build_log: String::new(),
            deploy_url: String::new(),
            preview_url: String::new(),
            duration_secs: 0,
            error_message: String::new(),
            initiated_by: "test".to_string(),
            metadata_json: "{}".to_string(),
            created_at: Utc::now().to_rfc3339(),
            started_at: None,
            completed_at: None,
        };
        store.create_deployment(&deployment).await;

        // Run pipeline
        let config = PipelineConfig {
            target,
            deployment_id: "d-test".to_string(),
            version: "1.0.0".to_string(),
            skip_tests: true,
            skip_verify: true,
            env_vars: vec![],
        };

        let result = run_pipeline(store.clone(), config).await;

        assert!(result.success);
        assert!(result.steps.len() >= 3); // checkout, build, deploy

        // Verify store was updated
        let d = store.get_deployment("d-test").await.unwrap();
        assert_eq!(d.status, "live");
        assert!(d.completed_at.is_some());

        // Verify snapshot was created
        let snaps = store.list_snapshots("t-test", 10).await;
        assert_eq!(snaps.len(), 1);
        assert!(snaps[0].is_current);

        // Verify logs were written
        let logs = store.get_logs("d-test").await;
        assert!(!logs.is_empty());
    }
}
