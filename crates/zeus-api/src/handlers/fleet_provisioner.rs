//! Fleet Auto-Provisioner (S10-7)
//!
//! Programmatically provisions new Zeus agents on remote machines via SSH.
//! Replaces manual deploy workflow with a single API call:
//!
//! - `POST /v1/fleet/provision` — SSH to target, install Zeus, configure, start gateway, register
//! - `GET /v1/fleet/provision/status/:id` — check provisioning job status
//!
//! Uses `tokio::process::Command` for SSH/SCP operations (no external SSH crate).
//! Mirrors patterns from `scripts/deploy.sh`.

use axum::extract::{Json, Path, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::process::Command;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

use crate::SharedState;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Supported target OS for provisioning
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TargetOs {
    Darwin,
    FreeBSD,
    Linux,
}

impl std::fmt::Display for TargetOs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Darwin => write!(f, "darwin"),
            Self::FreeBSD => write!(f, "freebsd"),
            Self::Linux => write!(f, "linux"),
        }
    }
}

/// Request to provision a new fleet agent
#[derive(Debug, Deserialize)]
pub struct ProvisionRequest {
    /// Target machine hostname or IP (e.g. "192.168.1.225")
    pub host: String,
    /// SSH username (default: "mike")
    #[serde(default = "default_user")]
    pub user: String,
    /// Path to SSH private key (default: ~/.ssh/id_ed25519)
    #[serde(default = "default_ssh_key")]
    pub ssh_key_path: String,
    /// Target OS
    #[serde(default = "default_os")]
    pub os: TargetOs,
    /// Agent role name (e.g. "worker", "reviewer", "builder")
    #[serde(default = "default_role")]
    pub agent_role: String,
    /// Agent ID (auto-generated if not provided)
    #[serde(default)]
    pub agent_id: Option<String>,
    /// LLM model to configure (default: "ollama/llama3.2")
    #[serde(default = "default_model")]
    pub model: String,
    /// Git repo URL to clone (default: Zeus repo)
    #[serde(default = "default_repo")]
    pub repo_url: String,
    /// SSH port (default: 22)
    #[serde(default = "default_port")]
    pub port: u16,
    /// Gateway port on remote machine (default: 3001)
    #[serde(default = "default_gateway_port")]
    pub gateway_port: u16,
    /// Additional env vars to write to .env on remote
    #[serde(default)]
    pub env_vars: HashMap<String, String>,
    /// Skip build step (use existing binary)
    #[serde(default)]
    pub skip_build: bool,
}

fn default_user() -> String {
    "mike".to_string()
}
fn default_ssh_key() -> String {
    "~/.ssh/id_ed25519".to_string()
}
fn default_os() -> TargetOs {
    TargetOs::FreeBSD
}
fn default_role() -> String {
    "worker".to_string()
}
fn default_model() -> String {
    "ollama/llama3.2".to_string()
}
pub(crate) fn default_repo() -> String {
    "git@github.com:zeuslabai/Zeus.git".to_string()
}
fn default_port() -> u16 {
    22
}
fn default_gateway_port() -> u16 {
    3001
}

/// Status of a provisioning job
#[derive(Debug, Clone, Serialize)]
pub struct ProvisionStatus {
    pub id: String,
    pub host: String,
    pub agent_id: String,
    pub state: ProvisionState,
    pub steps_completed: Vec<String>,
    pub current_step: Option<String>,
    pub error: Option<String>,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ProvisionState {
    Running,
    Completed,
    Failed,
}

/// Shared state for tracking provisioning jobs
pub type ProvisionJobs = Arc<RwLock<HashMap<String, ProvisionStatus>>>;

/// Create a new shared job tracker
pub fn new_provision_jobs() -> ProvisionJobs {
    Arc::new(RwLock::new(HashMap::new()))
}

// ---------------------------------------------------------------------------
// SSH helpers
// ---------------------------------------------------------------------------

/// Build base SSH command args with common options
fn ssh_base_args(user: &str, host: &str, key: &str, port: u16) -> Vec<String> {
    vec![
        "-o".to_string(),
        "ConnectTimeout=10".to_string(),
        "-o".to_string(),
        "BatchMode=yes".to_string(),
        "-o".to_string(),
        "StrictHostKeyChecking=accept-new".to_string(),
        "-i".to_string(),
        key.to_string(),
        "-p".to_string(),
        port.to_string(),
        format!("{}@{}", user, host),
    ]
}

/// Execute a remote command via SSH, returning stdout
async fn ssh_exec(
    user: &str,
    host: &str,
    key: &str,
    port: u16,
    command: &str,
) -> Result<String, String> {
    let mut args = ssh_base_args(user, host, key, port);
    args.push(command.to_string());

    let output = Command::new("ssh")
        .args(&args)
        .output()
        .await
        .map_err(|e| format!("SSH exec failed: {}", e))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!(
            "SSH command failed (exit {}): {}",
            output.status.code().unwrap_or(-1),
            stderr.trim()
        ))
    }
}

/// Test SSH connectivity to a host
async fn ssh_test(user: &str, host: &str, key: &str, port: u16) -> Result<(), String> {
    ssh_exec(user, host, key, port, "echo ok").await.map(|_| ())
}

// ---------------------------------------------------------------------------
// Provisioning steps
// ---------------------------------------------------------------------------

/// Generate config.toml content for the remote agent
fn generate_config_toml(model: &str, gateway_port: u16) -> String {
    format!(
        r#"model = "{model}"
workspace = "~/.zeus/workspace"
sessions = "~/.zeus/sessions"
max_iterations = 20

[gateway]
host = "0.0.0.0"
port = {gateway_port}
enable_api = true
enable_mcp = true
enable_channels = true
enable_heartbeat = true
enable_cron = false

[mnemosyne]
db_path = "~/.zeus/memory.db"
enable_fts = true

[talos]
enable_applescript = false

[nous]
enable_learning = true

[hermes]
default_channel = "console"
"#
    )
}

/// Generate CLAUDE.md for the remote agent
fn generate_claude_md(agent_id: &str, role: &str, host: &str) -> String {
    format!(
        r#"# CLAUDE.md — Zeus Fleet Agent

## Identity
- **Agent ID:** {agent_id}
- **Role:** {role}
- **Host:** {host}
- **Provisioned:** {date}

## Zeus Overview

Zeus is a 21-crate Rust AI assistant with TUI, macOS Desktop, iOS, Web frontends,
unified LLM provider, cognitive engine, multi-channel chat, security sandboxing,
browser automation, and voice calls.

- **Config**: `~/.zeus/config.toml` (behavior) + `~/.zeus/.env` (secrets)
- **Binary**: `/usr/local/bin/zeus`
- **Workspace**: `~/.zeus/workspace/`
- **Gateway**: `zeus gateway`
- **GitHub**: `git@github.com:zeuslabai/Zeus.git`

## Tools — Zeus MCP

Use Zeus MCP tools instead of native Claude Code tools where available.

## Code Quality — Non-Negotiable

- **NEVER use `.unwrap()` or `.expect()`** on fallible operations in production code.
- Run `cargo clippy` and `cargo fmt` before every commit. Zero warnings policy.
- Run `cargo test --workspace` before pushing.
- No `unsafe` without a `// SAFETY:` comment.

## Coordination Protocol

- Work on **feature branches only**. Never commit directly to `main`.
- Report progress on **Discord**.
- When done, report: what changed, what was tested, what's left.
"#,
        date = chrono::Utc::now().format("%Y-%m-%d"),
    )
}

/// Generate MCP settings JSON for Claude Code
fn generate_mcp_json() -> String {
    r#"{"mcpServers":{"zeus":{"command":"/usr/local/bin/zeus","args":["mcp"],"env":{}}}}"#
        .to_string()
}

/// Generate .env file content
fn generate_env(env_vars: &HashMap<String, String>) -> String {
    let mut lines = vec!["# Zeus environment — auto-provisioned".to_string()];
    for (key, value) in env_vars {
        lines.push(format!("{}={}", key, value));
    }
    lines.join("\n")
}

/// Run the full provisioning sequence.
///
/// Public within the crate so `agent_spawner` can invoke it directly.
pub(crate) async fn run_provision(
    req: ProvisionRequest,
    jobs: ProvisionJobs,
    job_id: String,
    agent_id: String,
    app_state: SharedState,
) {
    let user = &req.user;
    let host = &req.host;
    let key = &req.ssh_key_path;
    let port = req.port;

    // Helper to update job status
    let update_step = |step: &str, jobs: &ProvisionJobs| {
        let step = step.to_string();
        let jobs = jobs.clone();
        let job_id = job_id.clone();
        async move {
            let mut guard = jobs.write().await;
            if let Some(job) = guard.get_mut(&job_id) {
                if let Some(prev) = job.current_step.take() {
                    job.steps_completed.push(prev);
                }
                job.current_step = Some(step);
            }
        }
    };

    let fail = |err: String, jobs: &ProvisionJobs| {
        let jobs = jobs.clone();
        let job_id = job_id.clone();
        async move {
            let mut guard = jobs.write().await;
            if let Some(job) = guard.get_mut(&job_id) {
                job.state = ProvisionState::Failed;
                job.error = Some(err);
                job.completed_at = Some(chrono::Utc::now());
            }
        }
    };

    // Step 1: Test SSH connectivity
    update_step("ssh_test", &jobs).await;
    info!(host = %host, "Provisioner: testing SSH connectivity");
    if let Err(e) = ssh_test(user, host, key, port).await {
        error!(host = %host, error = %e, "Provisioner: SSH test failed");
        fail(format!("SSH connectivity failed: {}", e), &jobs).await;
        return;
    }

    // Step 2: Detect OS and validate
    update_step("detect_os", &jobs).await;
    let detected_os = match ssh_exec(user, host, key, port, "uname -s").await {
        Ok(out) => out.trim().to_lowercase(),
        Err(e) => {
            fail(format!("Failed to detect OS: {}", e), &jobs).await;
            return;
        }
    };
    let expected = match req.os {
        TargetOs::Darwin => "darwin",
        TargetOs::FreeBSD => "freebsd",
        TargetOs::Linux => "linux",
    };
    if detected_os != expected {
        warn!(
            host = %host,
            detected = %detected_os,
            expected = %expected,
            "OS mismatch — proceeding anyway"
        );
    }

    // Step 3: Ensure Zeus directory structure
    update_step("create_dirs", &jobs).await;
    let mkdir_cmd = "mkdir -p ~/.zeus/workspace ~/.zeus/sessions ~/.zeus/logs ~/.claude";
    if let Err(e) = ssh_exec(user, host, key, port, mkdir_cmd).await {
        fail(format!("Failed to create directories: {}", e), &jobs).await;
        return;
    }

    // Step 4: Clone or update repo
    update_step("clone_repo", &jobs).await;
    let clone_cmd = format!(
        r#"if [ -d ~/Zeus/.git ]; then cd ~/Zeus && git pull --ff-only 2>&1 | tail -3; else git clone {} ~/Zeus 2>&1 | tail -3; fi"#,
        req.repo_url
    );
    match ssh_exec(user, host, key, port, &clone_cmd).await {
        Ok(out) => info!(host = %host, output = %out.trim(), "Provisioner: repo ready"),
        Err(e) => {
            fail(format!("Failed to clone/update repo: {}", e), &jobs).await;
            return;
        }
    }

    // Step 5: Build Zeus binary (unless skip_build)
    if !req.skip_build {
        update_step("build", &jobs).await;
        info!(host = %host, "Provisioner: building Zeus (this may take a while)");
        let build_cmd = match req.os {
            TargetOs::FreeBSD | TargetOs::Linux => {
                "cd ~/Zeus && CORES=$(sysctl -n hw.ncpu 2>/dev/null || nproc 2>/dev/null || echo 4) && cargo build --release --bin zeus -j $CORES 2>&1 | tail -5"
            }
            TargetOs::Darwin => "cd ~/Zeus && cargo build --release --bin zeus 2>&1 | tail -5",
        };
        match ssh_exec(user, host, key, port, build_cmd).await {
            Ok(out) => {
                if out.contains("error") {
                    fail(format!("Build failed: {}", out.trim()), &jobs).await;
                    return;
                }
                info!(host = %host, output = %out.trim(), "Provisioner: build complete");
            }
            Err(e) => {
                fail(format!("Build failed: {}", e), &jobs).await;
                return;
            }
        }
    }

    // Step 6: Install binary
    update_step("install_binary", &jobs).await;
    let install_cmd = "cd ~/Zeus && sudo cp target/release/zeus /usr/local/bin/zeus && sudo chmod +x /usr/local/bin/zeus && echo INSTALL_OK";
    match ssh_exec(user, host, key, port, install_cmd).await {
        Ok(out) if out.contains("INSTALL_OK") => {
            info!(host = %host, "Provisioner: binary installed");
        }
        Ok(out) => {
            fail(format!("Install unclear: {}", out.trim()), &jobs).await;
            return;
        }
        Err(e) => {
            fail(format!("Install failed: {}", e), &jobs).await;
            return;
        }
    }

    // Step 7: Write config files
    update_step("write_config", &jobs).await;
    let config_toml = generate_config_toml(&req.model, req.gateway_port);
    let write_config_cmd = format!(
        r#"cat > ~/.zeus/config.toml << 'CFGEOF'
{}
CFGEOF
echo CONFIG_OK"#,
        config_toml.trim()
    );
    if let Err(e) = ssh_exec(user, host, key, port, &write_config_cmd).await {
        fail(format!("Failed to write config.toml: {}", e), &jobs).await;
        return;
    }

    // Step 8: Write .env
    update_step("write_env", &jobs).await;
    if !req.env_vars.is_empty() {
        let env_content = generate_env(&req.env_vars);
        let write_env_cmd = format!(
            r#"cat > ~/.zeus/.env << 'ENVEOF'
{}
ENVEOF
chmod 600 ~/.zeus/.env && echo ENV_OK"#,
            env_content.trim()
        );
        if let Err(e) = ssh_exec(user, host, key, port, &write_env_cmd).await {
            warn!(host = %host, error = %e, "Failed to write .env (non-fatal)");
        }
    }

    // Step 9: Write CLAUDE.md
    update_step("write_claude_md", &jobs).await;
    let claude_md = generate_claude_md(&agent_id, &req.agent_role, host);
    let write_claude_cmd = format!(
        r#"cat > ~/CLAUDE.md << 'MDEOF'
{}
MDEOF
echo CLAUDE_OK"#,
        claude_md.trim()
    );
    if let Err(e) = ssh_exec(user, host, key, port, &write_claude_cmd).await {
        warn!(host = %host, error = %e, "Failed to write CLAUDE.md (non-fatal)");
    }

    // Step 10: Write MCP config for Claude Code
    update_step("write_mcp_config", &jobs).await;
    let mcp_json = generate_mcp_json();
    let write_mcp_cmd = format!(
        r#"printf '{}' > ~/.claude/settings.json && echo MCP_OK"#,
        mcp_json.replace('\'', "'\\''")
    );
    if let Err(e) = ssh_exec(user, host, key, port, &write_mcp_cmd).await {
        warn!(host = %host, error = %e, "Failed to write MCP config (non-fatal)");
    }

    // Step 11: Stop existing gateway + start fresh
    update_step("start_gateway", &jobs).await;
    let start_cmd = match req.os {
        TargetOs::FreeBSD => {
            "sudo service zeus_gateway stop 2>/dev/null; pkill -f 'zeus gateway' 2>/dev/null; sleep 1; nohup /usr/local/bin/zeus gateway > ~/.zeus/logs/gateway.out.log 2>&1 & sleep 3; if fetch -qo - http://127.0.0.1:3001/health 2>/dev/null | grep -q ok; then echo HEALTH_OK; else echo HEALTH_FAIL; fi"
        }
        TargetOs::Darwin => {
            "pkill -f 'zeus gateway' 2>/dev/null; sleep 1; nohup /usr/local/bin/zeus gateway > ~/.zeus/logs/gateway.out.log 2>&1 & sleep 3; if curl -s --max-time 3 http://127.0.0.1:3001/health 2>/dev/null | grep -q ok; then echo HEALTH_OK; else echo HEALTH_FAIL; fi"
        }
        TargetOs::Linux => {
            "sudo systemctl stop zeus-gateway 2>/dev/null; pkill -f 'zeus gateway' 2>/dev/null; sleep 1; nohup /usr/local/bin/zeus gateway > ~/.zeus/logs/gateway.out.log 2>&1 & sleep 3; if curl -s --max-time 3 http://127.0.0.1:3001/health 2>/dev/null | grep -q ok; then echo HEALTH_OK; else echo HEALTH_FAIL; fi"
        }
    };
    match ssh_exec(user, host, key, port, start_cmd).await {
        Ok(out) if out.contains("HEALTH_OK") => {
            info!(host = %host, "Provisioner: gateway healthy");
        }
        Ok(out) => {
            warn!(host = %host, output = %out.trim(), "Provisioner: gateway started but health check unclear");
        }
        Err(e) => {
            fail(format!("Failed to start gateway: {}", e), &jobs).await;
            return;
        }
    }

    // Step 12: Register in fleet via local GlobalStateManager
    update_step("register_fleet", &jobs).await;
    {
        let state_guard = app_state.read().await;
        let gsm = state_guard.global_state();

        let mut agent =
            zeus_orchestra::state::AgentState::new(&agent_id, format!("{} — {}", agent_id, host));
        agent.metadata.insert("host".to_string(), host.to_string());
        agent.metadata.insert("ip".to_string(), host.to_string());
        agent.metadata.insert("os".to_string(), req.os.to_string());
        agent
            .metadata
            .insert("role".to_string(), req.agent_role.clone());
        agent
            .metadata
            .insert("provisioned".to_string(), chrono::Utc::now().to_rfc3339());
        agent = agent.with_capabilities(vec![
            "shell".to_string(),
            "code".to_string(),
            req.agent_role.clone(),
        ]);

        if let Err(e) = gsm.register_agent(agent).await {
            warn!(agent_id = %agent_id, error = %e, "Fleet registration failed (may already exist)");
        } else {
            info!(agent_id = %agent_id, "Provisioner: agent registered in fleet");
        }
    }

    // Done — mark job complete
    {
        let mut guard = jobs.write().await;
        if let Some(job) = guard.get_mut(&job_id) {
            if let Some(prev) = job.current_step.take() {
                job.steps_completed.push(prev);
            }
            job.state = ProvisionState::Completed;
            job.completed_at = Some(chrono::Utc::now());
        }
    }

    info!(
        host = %host,
        agent_id = %agent_id,
        "Provisioner: agent fully provisioned and registered"
    );
}

// ---------------------------------------------------------------------------
// HTTP handlers
// ---------------------------------------------------------------------------

/// POST /v1/fleet/provision — Start provisioning a new fleet agent
///
/// Launches the provisioning sequence in a background task and returns
/// immediately with a job ID for status polling.
pub async fn fleet_provision(
    State(state): State<SharedState>,
    Json(req): Json<ProvisionRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    // Validate host is not empty
    if req.host.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "host is required".to_string()));
    }

    // Generate IDs
    let job_id = uuid::Uuid::new_v4().to_string();
    let agent_id = req
        .agent_id
        .clone()
        .unwrap_or_else(|| format!("fleet-{}", req.host.replace('.', "-")));

    // Get or create the provision jobs tracker from app state
    let jobs = {
        let state_guard = state.read().await;
        state_guard
            .provision_jobs
            .clone()
            .unwrap_or_else(new_provision_jobs)
    };

    // Create initial job status
    {
        let mut guard = jobs.write().await;
        guard.insert(
            job_id.clone(),
            ProvisionStatus {
                id: job_id.clone(),
                host: req.host.clone(),
                agent_id: agent_id.clone(),
                state: ProvisionState::Running,
                steps_completed: vec![],
                current_step: Some("initializing".to_string()),
                error: None,
                started_at: chrono::Utc::now(),
                completed_at: None,
            },
        );
    }

    info!(
        host = %req.host,
        agent_id = %agent_id,
        job_id = %job_id,
        "Provisioner: starting provisioning job"
    );

    // Spawn background task
    let jobs_clone = jobs.clone();
    let job_id_clone = job_id.clone();
    let agent_id_clone = agent_id.clone();
    let state_clone = state.clone();
    tokio::spawn(async move {
        run_provision(req, jobs_clone, job_id_clone, agent_id_clone, state_clone).await;
    });

    Ok(Json(json!({
        "status": "provisioning",
        "job_id": job_id,
        "agent_id": agent_id,
        "message": "Provisioning started in background. Poll /v1/fleet/provision/status/{job_id} for progress.",
    })))
}

/// GET /v1/fleet/provision/status/:id — Check provisioning job status
pub async fn provision_status(
    State(state): State<SharedState>,
    Path(job_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let jobs = {
        let state_guard = state.read().await;
        state_guard
            .provision_jobs
            .clone()
            .unwrap_or_else(new_provision_jobs)
    };

    let guard = jobs.read().await;
    match guard.get(&job_id) {
        Some(status) => Ok(Json(serde_json::to_value(status).unwrap_or_default())),
        None => Err((
            StatusCode::NOT_FOUND,
            format!("Provisioning job '{}' not found", job_id),
        )),
    }
}

/// GET /v1/fleet/provision/jobs — List all provisioning jobs
pub async fn provision_jobs_list(State(state): State<SharedState>) -> Json<Value> {
    let jobs = {
        let state_guard = state.read().await;
        state_guard
            .provision_jobs
            .clone()
            .unwrap_or_else(new_provision_jobs)
    };

    let guard = jobs.read().await;
    let jobs_list: Vec<&ProvisionStatus> = guard.values().collect();
    Json(json!({ "jobs": jobs_list }))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_config_toml() {
        let config = generate_config_toml("ollama/llama3.2", 3001);
        assert!(config.contains("ollama/llama3.2"));
        assert!(config.contains("port = 3001"));
        assert!(config.contains("[gateway]"));
        assert!(config.contains("[mnemosyne]"));
        assert!(config.contains("enable_heartbeat = true"));
    }

    #[test]
    fn test_generate_config_toml_custom_port() {
        let config = generate_config_toml("anthropic/claude-sonnet", 8080);
        assert!(config.contains("anthropic/claude-sonnet"));
        assert!(config.contains("port = 8080"));
    }

    #[test]
    fn test_generate_claude_md() {
        let md = generate_claude_md("fbsd-test", "worker", "192.168.1.100");
        assert!(md.contains("fbsd-test"));
        assert!(md.contains("worker"));
        assert!(md.contains("192.168.1.100"));
        assert!(md.contains("NEVER use `.unwrap()`"));
        assert!(md.contains("feature branches only"));
    }

    #[test]
    fn test_generate_mcp_json() {
        let json_str = generate_mcp_json();
        let parsed: serde_json::Value =
            serde_json::from_str(&json_str).expect("MCP JSON should be valid");
        assert!(
            parsed["mcpServers"]["zeus"]["command"]
                .as_str()
                .is_some_and(|s| s == "/usr/local/bin/zeus")
        );
        assert!(
            parsed["mcpServers"]["zeus"]["args"][0]
                .as_str()
                .is_some_and(|s| s == "mcp")
        );
    }

    #[test]
    fn test_generate_env_empty() {
        let env = generate_env(&HashMap::new());
        assert!(env.contains("auto-provisioned"));
    }

    #[test]
    fn test_generate_env_with_vars() {
        let mut vars = HashMap::new();
        vars.insert("DISCORD_BOT_TOKEN".to_string(), "test-token".to_string());
        vars.insert("ZEUS_API_TOKEN".to_string(), "api-key".to_string());
        let env = generate_env(&vars);
        assert!(env.contains("DISCORD_BOT_TOKEN=test-token"));
        assert!(env.contains("ZEUS_API_TOKEN=api-key"));
    }

    #[test]
    fn test_ssh_base_args() {
        let args = ssh_base_args("user", "host.example", "~/.ssh/id_ed25519", 22);
        assert!(args.contains(&"BatchMode=yes".to_string()));
        assert!(args.contains(&"user@host.example".to_string()));
        assert!(args.contains(&"22".to_string()));
    }

    #[test]
    fn test_ssh_base_args_custom_port() {
        let args = ssh_base_args("root", "10.0.0.1", "/path/to/key", 2222);
        assert!(args.contains(&"root@10.0.0.1".to_string()));
        assert!(args.contains(&"2222".to_string()));
        assert!(args.contains(&"/path/to/key".to_string()));
    }

    #[test]
    fn test_target_os_display() {
        assert_eq!(TargetOs::Darwin.to_string(), "darwin");
        assert_eq!(TargetOs::FreeBSD.to_string(), "freebsd");
        assert_eq!(TargetOs::Linux.to_string(), "linux");
    }

    #[test]
    fn test_provision_state_serialization() {
        let json = serde_json::to_string(&ProvisionState::Running).expect("should serialize");
        assert_eq!(json, "\"running\"");

        let json = serde_json::to_string(&ProvisionState::Completed).expect("should serialize");
        assert_eq!(json, "\"completed\"");

        let json = serde_json::to_string(&ProvisionState::Failed).expect("should serialize");
        assert_eq!(json, "\"failed\"");
    }

    #[test]
    fn test_default_provision_values() {
        assert_eq!(default_user(), "mike");
        assert_eq!(default_os(), TargetOs::FreeBSD);
        assert_eq!(default_role(), "worker");
        assert_eq!(default_port(), 22);
        assert_eq!(default_gateway_port(), 3001);
    }

    #[test]
    fn test_provision_request_deserialization_minimal() {
        let json = r#"{"host": "192.168.1.200"}"#;
        let req: ProvisionRequest =
            serde_json::from_str(json).expect("should deserialize with defaults");
        assert_eq!(req.host, "192.168.1.200");
        assert_eq!(req.user, "mike");
        assert_eq!(req.port, 22);
        assert_eq!(req.os, TargetOs::FreeBSD);
        assert!(req.agent_id.is_none());
        assert!(!req.skip_build);
    }

    #[test]
    fn test_provision_request_deserialization_full() {
        let json = r#"{
            "host": "10.0.0.5",
            "user": "deploy",
            "ssh_key_path": "/root/.ssh/deploy_key",
            "os": "linux",
            "agent_role": "builder",
            "agent_id": "linux-builder-01",
            "model": "anthropic/claude-sonnet",
            "port": 2222,
            "gateway_port": 8080,
            "skip_build": true,
            "env_vars": {"API_KEY": "secret"}
        }"#;
        let req: ProvisionRequest = serde_json::from_str(json).expect("should deserialize");
        assert_eq!(req.host, "10.0.0.5");
        assert_eq!(req.user, "deploy");
        assert_eq!(req.os, TargetOs::Linux);
        assert_eq!(req.agent_id, Some("linux-builder-01".to_string()));
        assert_eq!(req.port, 2222);
        assert!(req.skip_build);
        assert_eq!(
            req.env_vars.get("API_KEY").map(|s| s.as_str()),
            Some("secret")
        );
    }

    #[tokio::test]
    async fn test_new_provision_jobs() {
        let jobs = new_provision_jobs();
        let guard = jobs.read().await;
        assert!(guard.is_empty());
    }

    #[tokio::test]
    async fn test_provision_job_tracking() {
        let jobs = new_provision_jobs();
        let job_id = "test-job-1".to_string();

        {
            let mut guard = jobs.write().await;
            guard.insert(
                job_id.clone(),
                ProvisionStatus {
                    id: job_id.clone(),
                    host: "192.168.1.100".to_string(),
                    agent_id: "test-agent".to_string(),
                    state: ProvisionState::Running,
                    steps_completed: vec!["ssh_test".to_string()],
                    current_step: Some("build".to_string()),
                    error: None,
                    started_at: chrono::Utc::now(),
                    completed_at: None,
                },
            );
        }

        let guard = jobs.read().await;
        let job = guard.get(&job_id).expect("job should exist");
        assert_eq!(job.state, ProvisionState::Running);
        assert_eq!(job.steps_completed.len(), 1);
        assert_eq!(job.current_step, Some("build".to_string()));
        assert!(job.error.is_none());
        assert!(job.completed_at.is_none());
    }

    #[tokio::test]
    async fn test_provision_job_failure() {
        let jobs = new_provision_jobs();
        let job_id = "test-fail".to_string();

        {
            let mut guard = jobs.write().await;
            guard.insert(
                job_id.clone(),
                ProvisionStatus {
                    id: job_id.clone(),
                    host: "10.0.0.99".to_string(),
                    agent_id: "fail-agent".to_string(),
                    state: ProvisionState::Failed,
                    steps_completed: vec!["ssh_test".to_string()],
                    current_step: None,
                    error: Some("Build failed: missing toolchain".to_string()),
                    started_at: chrono::Utc::now(),
                    completed_at: Some(chrono::Utc::now()),
                },
            );
        }

        let guard = jobs.read().await;
        let job = guard.get(&job_id).expect("job should exist");
        assert_eq!(job.state, ProvisionState::Failed);
        assert!(job.error.is_some());
        assert!(job.completed_at.is_some());
    }
}
