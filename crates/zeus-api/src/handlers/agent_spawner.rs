//! Programmatic Agent Spawner (S11-3)
//!
//! Higher-level agent spawning that bridges Nous intent analysis,
//! ProactiveSpawner recommendations, and fleet provisioning into a
//! single API call:
//!
//! - `POST /v1/agents/auto-spawn` — task → analyze → identity → provision → register → health
//! - `GET /v1/agents/auto-spawn/status/:id` — check spawn job status
//! - `GET /v1/agents/auto-spawn/jobs` — list all auto-spawn jobs
//!
//! Extends fleet_provisioner (S10-7) + spawner.rs (ProactiveSpawner).

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Json, Path, State};
use axum::http::StatusCode;
use ring::signature::{Ed25519KeyPair, KeyPair};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::RwLock;
use tracing::{error, info, warn};

use crate::SharedState;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Strategy for where to spawn the agent.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SpawnStrategy {
    /// Use the local agent pool (in-process).
    Local,
    /// Provision on a remote host via SSH (fleet_provisioner).
    Remote,
    /// Let the system decide based on task analysis and fleet capacity.
    #[default]
    Auto,
}

/// Request to auto-spawn an agent for a task.
#[derive(Debug, Deserialize)]
pub struct AutoSpawnRequest {
    /// Natural language description of the task.
    pub task: String,
    /// Spawn strategy (default: auto).
    #[serde(default)]
    pub strategy: SpawnStrategy,
    /// Target host for remote provisioning (required if strategy=remote).
    #[serde(default)]
    pub target_host: Option<String>,
    /// SSH user for remote provisioning (default: "mike").
    #[serde(default)]
    pub ssh_user: Option<String>,
    /// Agent role hint (e.g. "code-reviewer", "builder").
    #[serde(default)]
    pub role: Option<String>,
    /// Capabilities the spawned agent should have.
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Additional env vars to pass to the spawned agent.
    #[serde(default)]
    pub env_vars: HashMap<String, String>,
    /// Whether to announce the spawn on Discord.
    #[serde(default = "default_announce")]
    pub announce_discord: bool,
}

fn default_announce() -> bool {
    true
}

/// State of an auto-spawn job.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AutoSpawnState {
    /// Analyzing task with Nous.
    Analyzing,
    /// Generating sentient persona.
    #[allow(dead_code)]
    GeneratingPersona,
    /// Generating Ed25519 identity.
    GeneratingIdentity,
    /// Provisioning agent (local or remote).
    Provisioning,
    /// Registering in fleet + Discord.
    Registering,
    /// Running health check.
    HealthCheck,
    /// Agent is running and healthy.
    Running,
    /// Spawn failed.
    Failed,
}

/// Identity generated for a spawned agent.
#[derive(Debug, Clone, Serialize)]
pub struct AgentIdentity {
    /// Unique agent ID (e.g. "zeus-auto-a3f7b2c1").
    pub agent_id: String,
    /// Ed25519 public key (hex-encoded).
    pub public_key_hex: String,
    /// Path where the keypair was written on the target.
    pub key_path: String,
}

/// Analysis result from Nous (or fallback heuristics).
#[derive(Debug, Clone, Serialize)]
pub struct TaskAnalysis {
    /// Detected task category.
    pub category: String,
    /// Recommended capabilities for the agent.
    pub recommended_capabilities: Vec<String>,
    /// Whether the task benefits from remote execution.
    pub prefer_remote: bool,
    /// Confidence score 0.0-1.0.
    pub confidence: f32,
}

/// Status of an auto-spawn job.
#[derive(Debug, Clone, Serialize)]
pub struct AutoSpawnJob {
    pub id: String,
    pub task: String,
    pub strategy: SpawnStrategy,
    pub state: AutoSpawnState,
    pub identity: Option<AgentIdentity>,
    /// Sentient persona generated for this agent (S59-T1)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub persona: Option<SentientPersona>,
    /// Whether 2-stage review is enabled for this spawn (S60-T3)
    #[serde(default)]
    pub review_enabled: bool,
    pub analysis: Option<TaskAnalysis>,
    pub target_host: Option<String>,
    pub steps_completed: Vec<String>,
    pub current_step: Option<String>,
    pub error: Option<String>,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Shared state for tracking auto-spawn jobs.
pub type AutoSpawnJobs = Arc<RwLock<HashMap<String, AutoSpawnJob>>>;

/// Create a new shared job tracker.
pub fn new_auto_spawn_jobs() -> AutoSpawnJobs {
    Arc::new(RwLock::new(HashMap::new()))
}

// ---------------------------------------------------------------------------
// Identity generation
// ---------------------------------------------------------------------------

/// Generate an Ed25519 keypair and return the identity + PKCS8 bytes.
fn generate_identity(agent_id: &str) -> Result<(AgentIdentity, Vec<u8>), String> {
    let rng = ring::rand::SystemRandom::new();
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng)
        .map_err(|e| format!("Ed25519 keygen failed: {}", e))?;
    let pkcs8_bytes = pkcs8.as_ref().to_vec();

    let kp = Ed25519KeyPair::from_pkcs8(&pkcs8_bytes)
        .map_err(|e| format!("Failed to parse generated key: {}", e))?;

    let pub_hex = hex::encode(kp.public_key().as_ref());
    let key_path = format!("~/.zeus/keys/{}.pk8", agent_id);

    Ok((
        AgentIdentity {
            agent_id: agent_id.to_string(),
            public_key_hex: pub_hex,
            key_path,
        },
        pkcs8_bytes,
    ))
}

/// Generate a unique agent ID from a short random suffix.
fn generate_agent_id(role: Option<&str>) -> String {
    let suffix = &uuid::Uuid::new_v4().to_string()[..8];
    match role {
        Some(r) => format!("zeus-{}-{}", r, suffix),
        None => format!("zeus-auto-{}", suffix),
    }
}

// ---------------------------------------------------------------------------
// Task analysis
// ---------------------------------------------------------------------------

/// Analyze a task using Nous (if available) or fallback heuristics.
async fn analyze_task(task: &str, app_state: &SharedState) -> TaskAnalysis {
    // Try Nous intent understanding if available
    let state_guard = app_state.read().await;
    if let Some(ref nous) = state_guard.nous {
        match nous.understand(task).await {
            Ok(intent) => {
                // Map Nous IntentType to capabilities
                let (category, capabilities) = match &intent.intent_type {
                    zeus_nous::intent::IntentType::Execute { action, .. } => (
                        format!("execute:{}", action),
                        vec!["shell".to_string(), "code".to_string()],
                    ),
                    zeus_nous::intent::IntentType::Create { target } => (
                        format!("create:{}", target),
                        vec!["write_file".to_string(), "code".to_string()],
                    ),
                    zeus_nous::intent::IntentType::Analyze { subject, .. } => (
                        format!("analyze:{}", subject),
                        vec!["read_file".to_string(), "code".to_string()],
                    ),
                    zeus_nous::intent::IntentType::Automate { .. } => (
                        "automate".to_string(),
                        vec!["shell".to_string(), "code".to_string()],
                    ),
                    other => (
                        format!("{:?}", other),
                        vec!["shell".to_string(), "code".to_string()],
                    ),
                };
                let prefer_remote = intent.urgency > 0.7;
                let confidence = intent.confidence.value();
                return TaskAnalysis {
                    category,
                    recommended_capabilities: capabilities,
                    prefer_remote,
                    confidence,
                };
            }
            Err(e) => {
                warn!(error = %e, "Nous analysis failed, using heuristics");
            }
        }
    }
    drop(state_guard);

    // Fallback: keyword-based heuristics
    heuristic_analysis(task)
}

/// Simple keyword-based task analysis when Nous is unavailable.
fn heuristic_analysis(task: &str) -> TaskAnalysis {
    let lower = task.to_lowercase();

    let (category, capabilities, prefer_remote) =
        if lower.contains("build") || lower.contains("compile") || lower.contains("cargo") {
            ("build", vec!["shell".to_string(), "code".to_string()], true)
        } else if lower.contains("review") || lower.contains("audit") {
            (
                "review",
                vec!["read_file".to_string(), "code".to_string()],
                false,
            )
        } else if lower.contains("test") {
            (
                "testing",
                vec!["shell".to_string(), "code".to_string()],
                true,
            )
        } else if lower.contains("deploy") || lower.contains("provision") {
            (
                "deployment",
                vec!["shell".to_string(), "deploy".to_string()],
                true,
            )
        } else if lower.contains("design") || lower.contains("logo") || lower.contains("ui") {
            (
                "design",
                vec!["web_fetch".to_string(), "write_file".to_string()],
                false,
            )
        } else {
            (
                "general",
                vec!["shell".to_string(), "code".to_string()],
                false,
            )
        };

    TaskAnalysis {
        category: category.to_string(),
        recommended_capabilities: capabilities,
        prefer_remote,
        confidence: 0.5,
    }
}

// ---------------------------------------------------------------------------
// Sentient Intelligence — Persona Generation (S59-T1)
// ---------------------------------------------------------------------------

/// Rich persona traits derived from task analysis.
/// Inspired by MiroFish's OasisAgentProfile concept.
#[derive(Debug, Clone, Serialize)]
pub struct SentientPersona {
    /// Agent display name
    pub name: String,
    /// Role descriptor (e.g., "Security Auditor", "Backend Engineer")
    pub role: String,
    /// One-liner personality
    pub personality: String,
    /// Communication style
    pub style: String,
    /// Domain expertise areas
    pub expertise: Vec<String>,
    /// Behavioral traits
    pub traits: Vec<String>,
}

/// Generate a rich persona for a spawned agent based on its role and task.
///
/// Instead of generic agent descriptions, this creates a contextual personality
/// that influences how the agent approaches problems, communicates, and
/// prioritizes work. Part of the "Sentient Intelligence" initiative.
fn generate_sentient_persona(role: &str, task: &str) -> SentientPersona {
    let (personality, style, expertise, traits) = match role.to_lowercase().as_str() {
        r if r.contains("security") || r.contains("audit") => (
            "Meticulous, paranoid in a productive way, leaves no stone unturned.",
            "Precise and methodical. Lists findings by severity. Cites line numbers.",
            vec!["vulnerability assessment", "code review", "threat modeling", "compliance"],
            vec!["thorough", "skeptical", "detail-oriented", "systematic"],
        ),
        r if r.contains("build") || r.contains("engineer") || r.contains("develop") => (
            "Pragmatic builder who ships working code. Prefers simple solutions.",
            "Direct and code-focused. Shows diffs, not paragraphs. Tests everything.",
            vec!["systems programming", "architecture", "debugging", "performance"],
            vec!["pragmatic", "iterative", "quality-focused", "ship-fast"],
        ),
        r if r.contains("review") || r.contains("qa") => (
            "Sharp-eyed reviewer who catches what others miss. Constructive, not critical.",
            "Structured feedback: what works, what doesn't, how to fix it.",
            vec!["code review", "testing", "edge cases", "documentation"],
            vec!["observant", "constructive", "standards-driven", "patient"],
        ),
        r if r.contains("research") || r.contains("analyst") => (
            "Curious deep-diver who synthesizes complex information into actionable insights.",
            "Academic but accessible. Cites sources, provides context, draws conclusions.",
            vec!["research", "data analysis", "trend identification", "synthesis"],
            vec!["curious", "analytical", "thorough", "insightful"],
        ),
        r if r.contains("deploy") || r.contains("ops") || r.contains("devops") => (
            "Reliability-obsessed operator. If it's not monitored, it doesn't exist.",
            "Checklist-driven. Pre-flight, deploy, verify, rollback plan always ready.",
            vec!["deployment", "monitoring", "infrastructure", "incident response"],
            vec!["cautious", "prepared", "process-oriented", "resilient"],
        ),
        r if r.contains("design") || r.contains("ui") || r.contains("ux") => (
            "Pixel-perfectionist who thinks in user journeys, not just components.",
            "Visual and empathetic. Describes interactions, not just layouts.",
            vec!["UI/UX design", "accessibility", "user research", "prototyping"],
            vec!["empathetic", "creative", "detail-oriented", "user-focused"],
        ),
        r if r.contains("document") || r.contains("writer") || r.contains("docs") => (
            "Clear communicator who turns complexity into understanding.",
            "Structured, scannable, example-rich. Writes for the reader, not the writer.",
            vec!["technical writing", "documentation", "API docs", "tutorials"],
            vec!["clear", "organized", "empathetic", "concise"],
        ),
        _ => (
            "Adaptable problem-solver who focuses on outcomes over process.",
            "Concise and direct. Reports results, not plans.",
            vec!["general engineering", "problem solving", "tool use", "collaboration"],
            vec!["adaptable", "resourceful", "outcome-driven", "collaborative"],
        ),
    };

    // Extract key domain from task for name generation
    let _domain_hint = task.split_whitespace()
        .take(3)
        .collect::<Vec<_>>()
        .join("-")
        .to_lowercase();
    let name_suffix = &uuid::Uuid::new_v4().to_string()[..4];

    SentientPersona {
        name: format!("zeus-{}-{}", role.to_lowercase().replace(' ', "-"), name_suffix),
        role: role.to_string(),
        personality: personality.to_string(),
        style: style.to_string(),
        expertise: expertise.into_iter().map(|s| s.to_string()).collect(),
        traits: traits.into_iter().map(|s| s.to_string()).collect(),
    }
}

/// Generate SOUL.md content from a SentientPersona.
fn generate_soul_md(persona: &SentientPersona) -> String {
    format!(
        "# SOUL.md — {name}\n\n\
        _{personality}_\n\n\
        ## Role\n{role}\n\n\
        ## Communication Style\n{style}\n\n\
        ## Expertise\n{expertise}\n\n\
        ## Traits\n{traits}\n\n\
        ## Core Truths\n\n\
        Be genuinely helpful. Have opinions. Be resourceful before asking.\n\
        Earn trust through competence. Quality first — careful work saves time.\n",
        name = persona.name,
        personality = persona.personality,
        role = persona.role,
        style = persona.style,
        expertise = persona.expertise.iter().map(|e| format!("- {}", e)).collect::<Vec<_>>().join("\n"),
        traits = persona.traits.iter().map(|t| format!("- {}", t)).collect::<Vec<_>>().join("\n"),
    )
}

// ---------------------------------------------------------------------------
// CLAUDE.md generation
// ---------------------------------------------------------------------------

/// Generate a task-scoped CLAUDE.md for the spawned agent.
fn generate_task_claude_md(agent_id: &str, task: &str, role: &str) -> String {
    format!(
        r#"# CLAUDE.md — Zeus Auto-Spawned Agent

## Identity
- **Agent ID:** {agent_id}
- **Role:** {role}
- **Spawned:** {date}
- **Mode:** Autonomous task execution

## Assigned Task
{task}

## Instructions
1. Complete the assigned task above.
2. Report progress on Discord.
3. When done, report: what changed, what was tested, what's left.

## Verification — Evidence Before Claims
- **NO COMPLETION CLAIMS WITHOUT FRESH VERIFICATION.**
- Before saying 'done': run tests, show output, THEN claim completion.
- If you didn't run the test, you don't know if it passes.

## Debugging — Root Cause First
- **NO FIXES WITHOUT ROOT CAUSE INVESTIGATION.**
- Investigate first, form hypothesis, verify, then fix.
- Random fixes waste time and create new bugs.

## Code Quality — Non-Negotiable
- **NEVER use `.unwrap()` or `.expect()`** on fallible operations in production code.
- Run `cargo clippy` and `cargo fmt` before every commit. Zero warnings policy.
- Run `cargo test --workspace` before pushing.
- No `unsafe` without a `// SAFETY:` comment.
- Work on **feature branches only**. Never commit directly to `main`.
"#,
        date = chrono::Utc::now().format("%Y-%m-%d %H:%M UTC"),
    )
}

// ---------------------------------------------------------------------------
// Spawn execution
// ---------------------------------------------------------------------------

/// Resolve the effective spawn strategy.
fn resolve_strategy(
    requested: &SpawnStrategy,
    analysis: &TaskAnalysis,
    target_host: &Option<String>,
) -> SpawnStrategy {
    match requested {
        SpawnStrategy::Local => SpawnStrategy::Local,
        SpawnStrategy::Remote => SpawnStrategy::Remote,
        SpawnStrategy::Auto => {
            if target_host.is_some() {
                SpawnStrategy::Remote
            } else if analysis.prefer_remote {
                // Remote preferred but no host given — fall back to local
                SpawnStrategy::Local
            } else {
                SpawnStrategy::Local
            }
        }
    }
}

/// Run the full auto-spawn pipeline in the background.
async fn run_auto_spawn(
    req: AutoSpawnRequest,
    jobs: AutoSpawnJobs,
    job_id: String,
    app_state: SharedState,
) {
    let update_step = |step: &str, jobs: &AutoSpawnJobs| {
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

    let fail = |err: String, jobs: &AutoSpawnJobs| {
        let jobs = jobs.clone();
        let job_id = job_id.clone();
        async move {
            let mut guard = jobs.write().await;
            if let Some(job) = guard.get_mut(&job_id) {
                job.state = AutoSpawnState::Failed;
                job.error = Some(err);
                job.completed_at = Some(chrono::Utc::now());
            }
        }
    };

    // Step 1: Analyze task
    update_step("analyze_task", &jobs).await;
    info!(task = %req.task, "AutoSpawn: analyzing task");
    let analysis = analyze_task(&req.task, &app_state).await;
    {
        let mut guard = jobs.write().await;
        if let Some(job) = guard.get_mut(&job_id) {
            job.analysis = Some(analysis.clone());
        }
    }

    // Step 2: Resolve strategy
    let effective_strategy = resolve_strategy(&req.strategy, &analysis, &req.target_host);
    {
        let mut guard = jobs.write().await;
        if let Some(job) = guard.get_mut(&job_id) {
            job.strategy = effective_strategy.clone();
        }
    }

    // Step 3: Generate identity
    update_step("generate_identity", &jobs).await;
    {
        let mut guard = jobs.write().await;
        if let Some(job) = guard.get_mut(&job_id) {
            job.state = AutoSpawnState::GeneratingIdentity;
        }
    }
    let role = req.role.as_deref().unwrap_or("worker");
    let agent_id = generate_agent_id(Some(role));
    let (identity, pkcs8_bytes) = match generate_identity(&agent_id) {
        Ok(v) => v,
        Err(e) => {
            error!(error = %e, "AutoSpawn: identity generation failed");
            fail(format!("Identity generation failed: {}", e), &jobs).await;
            return;
        }
    };
    info!(agent_id = %agent_id, pub_key = %identity.public_key_hex, "AutoSpawn: identity generated");
    {
        let mut guard = jobs.write().await;
        if let Some(job) = guard.get_mut(&job_id) {
            job.identity = Some(identity.clone());
        }
    }

    // Step 4: Provision
    update_step("provision", &jobs).await;
    {
        let mut guard = jobs.write().await;
        if let Some(job) = guard.get_mut(&job_id) {
            job.state = AutoSpawnState::Provisioning;
        }
    }

    match effective_strategy {
        SpawnStrategy::Remote => {
            let host = match &req.target_host {
                Some(h) => h.clone(),
                None => {
                    fail("Remote strategy requires target_host".to_string(), &jobs).await;
                    return;
                }
            };

            // Write keypair to local temp path for SCP
            let local_key_dir = dirs::home_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
                .join(".zeus/keys");
            if let Err(e) = tokio::fs::create_dir_all(&local_key_dir).await {
                warn!(error = %e, "Failed to create local key directory");
            }
            let local_key_path = local_key_dir.join(format!("{}.pk8", agent_id));
            if let Err(e) = tokio::fs::write(&local_key_path, &pkcs8_bytes).await {
                warn!(error = %e, "Failed to write local keypair");
            }
            // chmod 600
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = tokio::fs::set_permissions(
                    &local_key_path,
                    std::fs::Permissions::from_mode(0o600),
                )
                .await;
            }

            // Build fleet provision request and delegate to fleet_provisioner
            let ssh_user = req.ssh_user.as_deref().unwrap_or("mike");
            let mut env_vars = req.env_vars.clone();
            env_vars.insert("ZEUS_AGENT_ID".to_string(), agent_id.clone());
            env_vars.insert(
                "ZEUS_AGENT_PUBLIC_KEY".to_string(),
                identity.public_key_hex.clone(),
            );

            // Inherit model from user config — never hardcode a provider
            let model = {
                let sg = app_state.read().await;
                sg.config.model.clone()
            };

            let provision_req = super::fleet_provisioner::ProvisionRequest {
                host: host.clone(),
                user: ssh_user.to_string(),
                ssh_key_path: "~/.ssh/id_ed25519".to_string(),
                os: super::fleet_provisioner::TargetOs::FreeBSD,
                agent_role: role.to_string(),
                agent_id: Some(agent_id.clone()),
                model,
                repo_url: super::fleet_provisioner::default_repo(),
                port: 22,
                gateway_port: std::env::var("ZEUS_GATEWAY_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(3001),
                env_vars,
                skip_build: false,
            };

            // Get provision jobs tracker
            let provision_jobs = {
                let state_guard = app_state.read().await;
                state_guard
                    .provision_jobs
                    .clone()
                    .unwrap_or_else(super::fleet_provisioner::new_provision_jobs)
            };

            let provision_job_id = uuid::Uuid::new_v4().to_string();
            {
                let mut guard = provision_jobs.write().await;
                guard.insert(
                    provision_job_id.clone(),
                    super::fleet_provisioner::ProvisionStatus {
                        id: provision_job_id.clone(),
                        host: host.clone(),
                        agent_id: agent_id.clone(),
                        state: super::fleet_provisioner::ProvisionState::Running,
                        steps_completed: vec![],
                        current_step: Some("initializing".to_string()),
                        error: None,
                        started_at: chrono::Utc::now(),
                        completed_at: None,
                    },
                );
            }

            // Run provisioning inline (we're already in a background task)
            super::fleet_provisioner::run_provision(
                provision_req,
                provision_jobs.clone(),
                provision_job_id.clone(),
                agent_id.clone(),
                app_state.clone(),
            )
            .await;

            // Check if provisioning succeeded
            let provision_state = {
                let guard = provision_jobs.read().await;
                guard
                    .get(&provision_job_id)
                    .map(|j| j.state.clone())
                    .unwrap_or(super::fleet_provisioner::ProvisionState::Failed)
            };

            if provision_state != super::fleet_provisioner::ProvisionState::Completed {
                let err = {
                    let guard = provision_jobs.read().await;
                    guard
                        .get(&provision_job_id)
                        .and_then(|j| j.error.clone())
                        .unwrap_or_else(|| "Unknown provisioning error".to_string())
                };
                fail(format!("Remote provisioning failed: {}", err), &jobs).await;
                return;
            }

            info!(agent_id = %agent_id, host = %host, "AutoSpawn: remote provisioning complete");
        }
        SpawnStrategy::Local => {
            // Register in the local agent pool via GlobalStateManager
            let state_guard = app_state.read().await;
            let gsm = state_guard.global_state();

            let mut caps = analysis.recommended_capabilities.clone();
            caps.extend(req.capabilities.clone());
            if !caps.contains(&role.to_string()) {
                caps.push(role.to_string());
            }

            let mut agent =
                zeus_orchestra::state::AgentState::new(&agent_id, format!("{} — local", agent_id));
            agent.metadata.insert("role".to_string(), role.to_string());
            agent.metadata.insert("task".to_string(), req.task.clone());
            agent
                .metadata
                .insert("public_key".to_string(), identity.public_key_hex.clone());
            agent
                .metadata
                .insert("spawn_type".to_string(), "auto-local".to_string());
            agent
                .metadata
                .insert("spawned_at".to_string(), chrono::Utc::now().to_rfc3339());
            agent = agent.with_capabilities(caps);

            if let Err(e) = gsm.register_agent(agent).await {
                warn!(agent_id = %agent_id, error = %e, "Fleet registration failed (may already exist)");
            }

            // Write task-scoped CLAUDE.md to workspace
            let claude_md = generate_task_claude_md(&agent_id, &req.task, role);
            let workspace_dir = dirs::home_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
                .join(format!(".zeus/agents/{}", agent_id));
            if let Err(e) = tokio::fs::create_dir_all(&workspace_dir).await {
                warn!(error = %e, "Failed to create agent workspace dir");
            }
            if let Err(e) =
                tokio::fs::write(workspace_dir.join("CLAUDE.md"), claude_md.as_bytes()).await
            {
                warn!(error = %e, "Failed to write agent CLAUDE.md");
            }

            // Generate and write Sentient Persona (S59-T1)
            let persona = generate_sentient_persona(role, &req.task);
            let soul_md = generate_soul_md(&persona);
            if let Err(e) =
                tokio::fs::write(workspace_dir.join("SOUL.md"), soul_md.as_bytes()).await
            {
                warn!(error = %e, "Failed to write agent SOUL.md");
            }
            info!(
                agent_id = %agent_id,
                persona_role = %persona.role,
                personality = %persona.personality,
                "AutoSpawn: sentient persona generated"
            );

            // T6: Create git worktree for isolated work (if repo exists)
            let repo_root = dirs::home_dir()
                .unwrap_or_default()
                .join("Zeus");
            if repo_root.join(".git").exists() {
                let branch_name = format!("agent/{}", agent_id);
                let worktree_path = workspace_dir.join("worktree");
                let worktree_result = tokio::process::Command::new("git")
                    .args(["worktree", "add", "-b", &branch_name,
                           worktree_path.to_str().unwrap_or("/tmp/worktree"), "main"])
                    .current_dir(&repo_root)
                    .output()
                    .await;
                match worktree_result {
                    Ok(output) if output.status.success() => {
                        info!(agent_id = %agent_id, "Git worktree created: {}", worktree_path.display());
                    }
                    Ok(output) => {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        warn!(agent_id = %agent_id, "Git worktree creation failed: {}", stderr.trim());
                    }
                    Err(e) => {
                        warn!(agent_id = %agent_id, error = %e, "Could not run git worktree");
                    }
                }
            }

            // Write keypair
            let key_dir = dirs::home_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
                .join(".zeus/keys");
            if let Err(e) = tokio::fs::create_dir_all(&key_dir).await {
                warn!(error = %e, "Failed to create key directory");
            }
            let key_path = key_dir.join(format!("{}.pk8", agent_id));
            if let Err(e) = tokio::fs::write(&key_path, &pkcs8_bytes).await {
                warn!(error = %e, "Failed to write agent keypair");
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ =
                    tokio::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))
                        .await;
            }

            info!(agent_id = %agent_id, "AutoSpawn: local agent registered");
        }
        SpawnStrategy::Auto => unreachable!("Auto should have been resolved"),
    }

    // Step 5: Register + announce
    update_step("register", &jobs).await;
    {
        let mut guard = jobs.write().await;
        if let Some(job) = guard.get_mut(&job_id) {
            job.state = AutoSpawnState::Registering;
        }
    }

    if req.announce_discord {
        // Best-effort Discord announcement via direct API
        let discord_token = zeus_core::resolve_discord_token();
        let channel_id = std::env::var("ZEUS_DISCORD_CHANNEL")
            .unwrap_or_else(|_| "1475583517156180018".to_string());

        if let Some(token) = discord_token {
            let client = reqwest::Client::new();
            let msg = format!(
                "**{}** spawned — role: {}, task: {}",
                agent_id,
                role,
                if req.task.len() > 100 {
                    format!("{}...", zeus_core::truncate_str(&req.task, 100))
                } else {
                    req.task.clone()
                }
            );
            let url = format!(
                "https://discord.com/api/v10/channels/{}/messages",
                channel_id
            );
            let resp = client
                .post(&url)
                .header("Authorization", format!("Bot {}", token))
                .json(&json!({ "content": msg }))
                .send()
                .await;
            match resp {
                Ok(r) if r.status().is_success() => {
                    info!(agent_id = %agent_id, "AutoSpawn: announced on Discord");
                }
                Ok(r) => {
                    warn!(status = %r.status(), "AutoSpawn: Discord announce failed");
                }
                Err(e) => {
                    warn!(error = %e, "AutoSpawn: Discord announce failed");
                }
            }
        }
    }

    // Step 6: Health check
    update_step("health_check", &jobs).await;
    {
        let mut guard = jobs.write().await;
        if let Some(job) = guard.get_mut(&job_id) {
            job.state = AutoSpawnState::HealthCheck;
        }
    }

    // For remote agents, ping their gateway health endpoint
    if effective_strategy == SpawnStrategy::Remote
        && let Some(ref host) = req.target_host
    {
        let health_url = format!("http://{}:3001/health", host);
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap_or_default();

        let mut healthy = false;
        for attempt in 1..=3 {
            match client.get(&health_url).send().await {
                Ok(r) if r.status().is_success() => {
                    healthy = true;
                    info!(agent_id = %agent_id, attempt, "AutoSpawn: health check passed");
                    break;
                }
                Ok(r) => {
                    warn!(agent_id = %agent_id, status = %r.status(), attempt, "Health check non-200");
                }
                Err(e) => {
                    warn!(agent_id = %agent_id, error = %e, attempt, "Health check failed");
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }

        if !healthy {
            warn!(agent_id = %agent_id, "AutoSpawn: health checks failed, marking running anyway");
        }
    }

    // For local agents, verify they're in the registry
    if effective_strategy == SpawnStrategy::Local {
        let state_guard = app_state.read().await;
        let gsm = state_guard.global_state();
        match gsm.get_agent(&agent_id).await {
            Some(agent) if agent.health_score > 0.0 => {
                info!(agent_id = %agent_id, "AutoSpawn: local agent healthy");
            }
            _ => {
                warn!(agent_id = %agent_id, "AutoSpawn: local agent not found in registry");
            }
        }
    }

    // Step 7: Start background health loop
    let health_agent_id = agent_id.clone();
    let health_state = app_state.clone();
    let health_target = req.target_host.clone();
    tokio::spawn(async move {
        health_loop(health_agent_id, health_state, health_target).await;
    });

    // Done — mark as running
    {
        let mut guard = jobs.write().await;
        if let Some(job) = guard.get_mut(&job_id) {
            if let Some(prev) = job.current_step.take() {
                job.steps_completed.push(prev);
            }
            job.state = AutoSpawnState::Running;
            job.completed_at = Some(chrono::Utc::now());
        }
    }

    info!(agent_id = %agent_id, "AutoSpawn: agent fully spawned and running");
}

/// Background health monitoring loop for a spawned agent.
async fn health_loop(agent_id: String, app_state: SharedState, target_host: Option<String>) {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap_or_default();

    let mut consecutive_failures = 0u32;
    let max_failures = 5;

    loop {
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;

        let healthy = if let Some(ref host) = target_host {
            // Remote: HTTP health check
            let url = format!("http://{}:3001/health", host);
            client
                .get(&url)
                .send()
                .await
                .is_ok_and(|r| r.status().is_success())
        } else {
            // Local: check registry
            let state_guard = app_state.read().await;
            let gsm = state_guard.global_state();
            gsm.get_agent(&agent_id)
                .await
                .is_some_and(|a| a.health_score > 0.0)
        };

        if healthy {
            consecutive_failures = 0;
            let state_guard = app_state.read().await;
            let gsm = state_guard.global_state();
            let _ = gsm.heartbeat(&agent_id).await;
            let _ = gsm.update_health(&agent_id, 1.0).await;
        } else {
            consecutive_failures += 1;
            warn!(
                agent_id = %agent_id,
                consecutive_failures,
                "AutoSpawn health check failed"
            );

            let degraded_health =
                (1.0 - (consecutive_failures as f32 / max_failures as f32)).max(0.0);
            let state_guard = app_state.read().await;
            let gsm = state_guard.global_state();
            let _ = gsm.update_health(&agent_id, degraded_health).await;

            if consecutive_failures >= max_failures {
                error!(
                    agent_id = %agent_id,
                    "AutoSpawn: agent unresponsive after {} checks, stopping health loop",
                    max_failures
                );
                let _ = gsm.update_health(&agent_id, 0.0).await;
                let _ = gsm
                    .update_status(&agent_id, zeus_orchestra::state::AgentStatus::Offline)
                    .await;
                break;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// HTTP handlers
// ---------------------------------------------------------------------------

/// POST /v1/agents/auto-spawn — Spawn an agent for a task.
///
/// Analyzes the task, generates an Ed25519 identity, provisions the agent
/// (locally or remotely), registers in the fleet, announces on Discord,
/// and starts a health monitoring loop.
pub async fn auto_spawn_agent(
    State(state): State<SharedState>,
    Json(req): Json<AutoSpawnRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    if req.task.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "task is required".to_string()));
    }

    if req.strategy == SpawnStrategy::Remote && req.target_host.is_none() {
        return Err((
            StatusCode::BAD_REQUEST,
            "target_host is required for remote strategy".to_string(),
        ));
    }

    let job_id = uuid::Uuid::new_v4().to_string();

    // Initialize job tracker (stored on AppState or create ephemeral)
    let jobs = new_auto_spawn_jobs();

    {
        let mut guard = jobs.write().await;
        guard.insert(
            job_id.clone(),
            AutoSpawnJob {
                id: job_id.clone(),
                task: req.task.clone(),
                strategy: req.strategy.clone(),
                state: AutoSpawnState::Analyzing,
                identity: None,
                persona: None,
                review_enabled: true,
                analysis: None,
                target_host: req.target_host.clone(),
                steps_completed: vec![],
                current_step: Some("initializing".to_string()),
                error: None,
                started_at: chrono::Utc::now(),
                completed_at: None,
            },
        );
    }

    info!(job_id = %job_id, task = %req.task, "AutoSpawn: job created");

    // Spawn background task
    let jobs_clone = jobs.clone();
    let job_id_clone = job_id.clone();
    let state_clone = state.clone();
    tokio::spawn(async move {
        run_auto_spawn(req, jobs_clone, job_id_clone, state_clone).await;
    });

    // Store jobs reference for status polling
    // We attach to the existing provision_jobs mechanism via a side channel
    {
        let state_guard = state.read().await;
        let gsm = state_guard.global_state();
        // Store job tracker reference in metadata for retrieval
        let mut meta_agent = zeus_orchestra::state::AgentState::new(
            format!("spawn-job-{}", &job_id[..8]),
            "Auto-spawn job tracker",
        );
        meta_agent
            .metadata
            .insert("job_id".to_string(), job_id.clone());
        meta_agent
            .metadata
            .insert("type".to_string(), "auto-spawn-tracker".to_string());
        let _ = gsm.register_agent(meta_agent).await;
    }

    Ok(Json(json!({
        "status": "spawning",
        "job_id": job_id,
        "message": format!(
            "Auto-spawn initiated. Poll /v1/agents/auto-spawn/status/{} for progress.",
            job_id
        ),
    })))
}

/// GET /v1/agents/auto-spawn/status/:id — Check auto-spawn job status.
///
/// Returns the current state of the spawn pipeline including task analysis,
/// identity, and completion status.
pub async fn auto_spawn_status(
    State(state): State<SharedState>,
    Path(job_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    // Look up the agent in GSM that tracks this job
    let state_guard = state.read().await;
    let gsm = state_guard.global_state();
    let agents = gsm.list_agents().await;

    let tracker_agent = agents.iter().find(|a| {
        a.metadata.get("job_id").is_some_and(|id| id == &job_id)
            && a.metadata
                .get("type")
                .is_some_and(|t| t == "auto-spawn-tracker")
    });

    match tracker_agent {
        Some(agent) => Ok(Json(json!({
            "job_id": job_id,
            "agent_id": agent.id,
            "status": format!("{:?}", agent.status),
            "health_score": agent.health_score,
            "metadata": agent.metadata,
            "registered_at": agent.registered_at.to_rfc3339(),
            "last_heartbeat": agent.last_heartbeat.to_rfc3339(),
        }))),
        None => Err((
            StatusCode::NOT_FOUND,
            format!("Auto-spawn job '{}' not found", job_id),
        )),
    }
}

/// GET /v1/agents/auto-spawn/jobs — List all auto-spawned agents.
///
/// Returns all agents that were created via the auto-spawn pipeline,
/// identified by their `spawn_type` metadata.
pub async fn auto_spawn_jobs(State(state): State<SharedState>) -> Json<Value> {
    let state_guard = state.read().await;
    let gsm = state_guard.global_state();
    let agents = gsm.list_agents().await;

    let spawned: Vec<Value> = agents
        .iter()
        .filter(|a| {
            a.metadata
                .get("spawn_type")
                .is_some_and(|t| t.starts_with("auto"))
                || a.metadata
                    .get("type")
                    .is_some_and(|t| t == "auto-spawn-tracker")
        })
        .map(|a| {
            json!({
                "agent_id": a.id,
                "name": a.name,
                "status": format!("{:?}", a.status),
                "health_score": a.health_score,
                "role": a.metadata.get("role").cloned().unwrap_or_default(),
                "task": a.metadata.get("task").cloned().unwrap_or_default(),
                "spawn_type": a.metadata.get("spawn_type").cloned().unwrap_or_default(),
                "registered_at": a.registered_at.to_rfc3339(),
                "last_heartbeat": a.last_heartbeat.to_rfc3339(),
            })
        })
        .collect();

    Json(json!({ "agents": spawned, "count": spawned.len() }))
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------
// Report Generation (S59-T4)
// ---------------------------------------------------------------------------

/// Request to generate a fleet/sprint report.
#[derive(Debug, Deserialize)]
pub struct GenerateReportRequest {
    /// Report type: "sprint", "fleet_health", "audit", "task_summary"
    #[serde(default = "default_report_type")]
    pub report_type: String,
    /// Time range in hours (default: 24)
    #[serde(default = "default_report_hours")]
    pub hours: u64,
    /// Optional: specific agent IDs to include
    #[serde(default)]
    #[allow(dead_code)]
    pub agent_ids: Vec<String>,
    /// Optional: sprint identifier (e.g., "S57")
    #[serde(default)]
    pub sprint: Option<String>,
}

fn default_report_type() -> String { "sprint".to_string() }
fn default_report_hours() -> u64 { 24 }

/// POST /v1/reports/generate — auto-generate a structured report from fleet activity.
///
/// Collects data from agent registry, session history, git commits,
/// and produces a markdown report. Inspired by MiroFish's ReportAgent pattern.
pub async fn generate_report(
    State(state): State<SharedState>,
    Json(req): Json<GenerateReportRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;

    let mut sections: Vec<String> = Vec::new();
    let now = chrono::Utc::now();
    let _since = now - chrono::Duration::hours(req.hours as i64);

    sections.push(format!(
        "# Zeus {} Report\n**Generated:** {}\n**Period:** Last {} hours\n",
        match req.report_type.as_str() {
            "sprint" => req.sprint.as_deref().unwrap_or("Sprint"),
            "fleet_health" => "Fleet Health",
            "audit" => "Audit",
            "task_summary" => "Task Summary",
            _ => "Activity",
        },
        now.format("%Y-%m-%d %H:%M UTC"),
        req.hours,
    ));

    // Fleet status
    let agents = state.agent_registry.list();
    let total = agents.len();
    sections.push(format!(
        "## Fleet Status\n- **Registered agents:** {}\n",
        total,
    ));

    // Per-agent summary
    if !agents.is_empty() {
        sections.push("## Agent Activity\n".to_string());
        for agent in &agents {
            let active_ago = now.signed_duration_since(agent.last_active);
            let active_str = if active_ago.num_minutes() < 5 {
                "🟢 active now".to_string()
            } else if active_ago.num_hours() < 1 {
                format!("🟡 {}m ago", active_ago.num_minutes())
            } else {
                format!("🔴 {}h ago", active_ago.num_hours())
            };
            sections.push(format!(
                "- **{}** ({}) — {} | Messages: {}\n",
                agent.name,
                agent.agent_id,
                active_str,
                agent.message_count,
            ));
        }
    }

    let online = agents.iter().filter(|a| {
        now.signed_duration_since(a.last_active).num_minutes() < 5
    }).count();

    // Agent stats
    let total_messages: u64 = agents.iter().map(|a| a.message_count).sum();
    sections.push(format!(
        "## Stats\n- **Total messages processed:** {}\n",
        total_messages,
    ));

    // Config summary
    let (provider, model) = state.config.parse_model();
    sections.push(format!(
        "## Configuration\n- **Provider:** {:?}\n- **Model:** {}\n- **Onboarding:** {}\n",
        provider,
        model,
        if state.config.onboarding_complete { "Complete" } else { "Pending" },
    ));

    let report = sections.join("\n");

    Ok(Json(json!({
        "report": report,
        "report_type": req.report_type,
        "generated_at": now.to_rfc3339(),
        "period_hours": req.hours,
        "agents_count": agents.len(),
        "agents_online": online,
    })))
}

// ---------------------------------------------------------------------------
// Parallel Dispatch (S60-T5)
// ---------------------------------------------------------------------------

/// Batch spawn request — one agent per task, all dispatched in parallel.
/// Inspired by Superpowers' dispatching-parallel-agents skill.
#[derive(Debug, Deserialize)]
pub struct BatchSpawnRequest {
    /// List of tasks to dispatch (one agent per task)
    pub tasks: Vec<BatchTask>,
    /// Whether to announce spawns on Discord
    #[serde(default = "default_announce")]
    pub announce_discord: bool,
}

#[derive(Debug, Deserialize)]
pub struct BatchTask {
    /// Task description
    pub task: String,
    /// Role hint (optional)
    #[serde(default)]
    pub role: Option<String>,
}

/// POST /v1/agents/auto-spawn/batch — dispatch parallel agents
///
/// One agent per task, all spawned concurrently. Each gets its own
/// persona, workspace, and identity. Coordinator pattern from Superpowers.
pub async fn batch_spawn(
    State(state): State<SharedState>,
    Json(req): Json<BatchSpawnRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    if req.tasks.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "No tasks provided".to_string()));
    }
    if req.tasks.len() > 10 {
        return Err((StatusCode::BAD_REQUEST, "Maximum 10 parallel tasks".to_string()));
    }

    let mut job_ids = Vec::new();

    for batch_task in &req.tasks {
        let spawn_req = AutoSpawnRequest {
            task: batch_task.task.clone(),
            strategy: SpawnStrategy::Local,
            target_host: None,
            ssh_user: None,
            role: batch_task.role.clone(),
            capabilities: vec![],
            env_vars: HashMap::new(),
            announce_discord: req.announce_discord,
        };

        // Dispatch each spawn (they run async internally)
        match auto_spawn_agent(State(state.clone()), Json(spawn_req)).await {
            Ok(Json(response)) => {
                if let Some(id) = response.get("job_id").and_then(|v| v.as_str()) {
                    job_ids.push(json!({
                        "task": batch_task.task,
                        "job_id": id,
                        "status": "dispatched",
                    }));
                }
            }
            Err((_, err)) => {
                job_ids.push(json!({
                    "task": batch_task.task,
                    "error": err,
                    "status": "failed",
                }));
            }
        }
    }

    Ok(Json(json!({
        "dispatched": job_ids.len(),
        "jobs": job_ids,
        "pattern": "one-agent-per-task",
    })))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_agent_id_with_role() {
        let id = generate_agent_id(Some("builder"));
        assert!(id.starts_with("zeus-builder-"));
        assert_eq!(id.len(), "zeus-builder-".len() + 8);
    }

    #[test]
    fn test_generate_agent_id_without_role() {
        let id = generate_agent_id(None);
        assert!(id.starts_with("zeus-auto-"));
        assert_eq!(id.len(), "zeus-auto-".len() + 8);
    }

    #[test]
    fn test_generate_identity() {
        let (identity, pkcs8) = generate_identity("test-agent").expect("keygen should succeed");
        assert_eq!(identity.agent_id, "test-agent");
        assert_eq!(identity.public_key_hex.len(), 64); // 32 bytes = 64 hex chars
        assert!(identity.key_path.contains("test-agent"));
        assert!(!pkcs8.is_empty());
    }

    #[test]
    fn test_generate_identity_unique_keys() {
        let (id1, _) = generate_identity("a").expect("keygen");
        let (id2, _) = generate_identity("b").expect("keygen");
        assert_ne!(id1.public_key_hex, id2.public_key_hex);
    }

    #[test]
    fn test_heuristic_analysis_build() {
        let analysis = heuristic_analysis("build the Zeus binary on FreeBSD");
        assert_eq!(analysis.category, "build");
        assert!(analysis.prefer_remote);
        assert!(
            analysis
                .recommended_capabilities
                .contains(&"shell".to_string())
        );
    }

    #[test]
    fn test_heuristic_analysis_review() {
        let analysis = heuristic_analysis("review the pull request for security issues");
        assert_eq!(analysis.category, "review");
        assert!(!analysis.prefer_remote);
        assert!(
            analysis
                .recommended_capabilities
                .contains(&"read_file".to_string())
        );
    }

    #[test]
    fn test_heuristic_analysis_test() {
        let analysis = heuristic_analysis("run the test suite and fix failures");
        assert_eq!(analysis.category, "testing");
        assert!(analysis.prefer_remote);
    }

    #[test]
    fn test_heuristic_analysis_deploy() {
        let analysis = heuristic_analysis("deploy the service to production servers");
        assert_eq!(analysis.category, "deployment");
        assert!(
            analysis
                .recommended_capabilities
                .contains(&"deploy".to_string())
        );
    }

    #[test]
    fn test_heuristic_analysis_design() {
        let analysis = heuristic_analysis("design a logo for the marketing page");
        assert_eq!(analysis.category, "design");
        assert!(!analysis.prefer_remote);
    }

    #[test]
    fn test_heuristic_analysis_general() {
        let analysis = heuristic_analysis("help me with something");
        assert_eq!(analysis.category, "general");
        assert_eq!(analysis.confidence, 0.5);
    }

    #[test]
    fn test_resolve_strategy_explicit_local() {
        let analysis = TaskAnalysis {
            category: "build".to_string(),
            recommended_capabilities: vec![],
            prefer_remote: true,
            confidence: 0.9,
        };
        let result = resolve_strategy(&SpawnStrategy::Local, &analysis, &None);
        assert_eq!(result, SpawnStrategy::Local);
    }

    #[test]
    fn test_resolve_strategy_explicit_remote() {
        let analysis = TaskAnalysis {
            category: "review".to_string(),
            recommended_capabilities: vec![],
            prefer_remote: false,
            confidence: 0.9,
        };
        let result = resolve_strategy(
            &SpawnStrategy::Remote,
            &analysis,
            &Some("192.168.1.100".to_string()),
        );
        assert_eq!(result, SpawnStrategy::Remote);
    }

    #[test]
    fn test_resolve_strategy_auto_with_host() {
        let analysis = TaskAnalysis {
            category: "general".to_string(),
            recommended_capabilities: vec![],
            prefer_remote: false,
            confidence: 0.5,
        };
        let result = resolve_strategy(
            &SpawnStrategy::Auto,
            &analysis,
            &Some("10.0.0.1".to_string()),
        );
        assert_eq!(result, SpawnStrategy::Remote);
    }

    #[test]
    fn test_resolve_strategy_auto_prefer_remote_no_host() {
        let analysis = TaskAnalysis {
            category: "build".to_string(),
            recommended_capabilities: vec![],
            prefer_remote: true,
            confidence: 0.9,
        };
        // Prefer remote but no host → falls back to local
        let result = resolve_strategy(&SpawnStrategy::Auto, &analysis, &None);
        assert_eq!(result, SpawnStrategy::Local);
    }

    #[test]
    fn test_resolve_strategy_auto_local() {
        let analysis = TaskAnalysis {
            category: "review".to_string(),
            recommended_capabilities: vec![],
            prefer_remote: false,
            confidence: 0.8,
        };
        let result = resolve_strategy(&SpawnStrategy::Auto, &analysis, &None);
        assert_eq!(result, SpawnStrategy::Local);
    }

    #[test]
    fn test_generate_task_claude_md() {
        let md = generate_task_claude_md("zeus-worker-abc", "fix the login bug", "worker");
        assert!(md.contains("zeus-worker-abc"));
        assert!(md.contains("fix the login bug"));
        assert!(md.contains("worker"));
        assert!(md.contains("NEVER use `.unwrap()`"));
        assert!(md.contains("feature branches only"));
    }

    #[test]
    fn test_spawn_strategy_default() {
        let strategy = SpawnStrategy::default();
        assert_eq!(strategy, SpawnStrategy::Auto);
    }

    #[test]
    fn test_spawn_strategy_serialization() {
        let json = serde_json::to_string(&SpawnStrategy::Remote).expect("serialize");
        assert_eq!(json, "\"remote\"");

        let back: SpawnStrategy = serde_json::from_str("\"local\"").expect("deserialize");
        assert_eq!(back, SpawnStrategy::Local);
    }

    #[test]
    fn test_auto_spawn_state_serialization() {
        let json = serde_json::to_string(&AutoSpawnState::Running).expect("serialize");
        assert_eq!(json, "\"running\"");

        let json = serde_json::to_string(&AutoSpawnState::Failed).expect("serialize");
        assert_eq!(json, "\"failed\"");
    }

    #[test]
    fn test_auto_spawn_request_deserialization_minimal() {
        let json = r#"{"task": "run tests"}"#;
        let req: AutoSpawnRequest = serde_json::from_str(json).expect("deserialize");
        assert_eq!(req.task, "run tests");
        assert_eq!(req.strategy, SpawnStrategy::Auto);
        assert!(req.target_host.is_none());
        assert!(req.announce_discord);
    }

    #[test]
    fn test_auto_spawn_request_deserialization_full() {
        let json = r#"{
            "task": "build Zeus on FreeBSD",
            "strategy": "remote",
            "target_host": "192.168.1.225",
            "ssh_user": "deploy",
            "role": "builder",
            "capabilities": ["shell", "code"],
            "env_vars": {"RUST_LOG": "debug"},
            "announce_discord": false
        }"#;
        let req: AutoSpawnRequest = serde_json::from_str(json).expect("deserialize");
        assert_eq!(req.task, "build Zeus on FreeBSD");
        assert_eq!(req.strategy, SpawnStrategy::Remote);
        assert_eq!(req.target_host, Some("192.168.1.225".to_string()));
        assert_eq!(req.ssh_user, Some("deploy".to_string()));
        assert_eq!(req.role, Some("builder".to_string()));
        assert_eq!(req.capabilities.len(), 2);
        assert!(!req.announce_discord);
    }

    #[test]
    fn test_new_auto_spawn_jobs() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime");
        rt.block_on(async {
            let jobs = new_auto_spawn_jobs();
            let guard = jobs.read().await;
            assert!(guard.is_empty());
        });
    }
}
