//! Documentation Site Generator
//!
//! Auto-generates HTML documentation pages served at GET /docs:
//! - /docs              — Index page linking to all sections
//! - /docs/openapi.json — OpenAPI 3.0 spec from route handlers
//! - /docs/tools        — Tool reference from ToolSchema definitions
//! - /docs/config       — Configuration guide from Config struct
//! - /docs/getting-started — Getting Started guide from onboarding flow

use axum::{
    extract::State,
    http::{StatusCode, header},
    response::{Html, IntoResponse, Response},
};
use serde_json::json;

use crate::SharedState;

// ============================================================================
// Shared HTML scaffolding
// ============================================================================

const CSS: &str = r#"
:root {
    --bg: #0f172a; --surface: #1e293b; --border: #334155;
    --text: #e2e8f0; --muted: #94a3b8; --accent: #818cf8;
    --green: #34d399; --amber: #fbbf24; --red: #f87171;
}
* { margin: 0; padding: 0; box-sizing: border-box; }
body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, monospace;
       background: var(--bg); color: var(--text); line-height: 1.6; }
.container { max-width: 960px; margin: 0 auto; padding: 2rem 1.5rem; }
h1 { font-size: 2rem; margin-bottom: 0.5rem; color: var(--accent); }
h2 { font-size: 1.4rem; margin: 2rem 0 0.75rem; color: var(--green);
     border-bottom: 1px solid var(--border); padding-bottom: 0.25rem; }
h3 { font-size: 1.1rem; margin: 1.5rem 0 0.5rem; color: var(--amber); }
p, li { color: var(--text); margin-bottom: 0.5rem; }
a { color: var(--accent); text-decoration: none; }
a:hover { text-decoration: underline; }
code { background: var(--surface); padding: 0.15em 0.4em; border-radius: 4px;
       font-size: 0.9em; color: var(--green); }
pre { background: var(--surface); padding: 1rem; border-radius: 8px;
      overflow-x: auto; margin: 1rem 0; border: 1px solid var(--border); }
pre code { background: none; padding: 0; }
table { width: 100%; border-collapse: collapse; margin: 1rem 0; }
th, td { text-align: left; padding: 0.5rem 0.75rem; border-bottom: 1px solid var(--border); }
th { color: var(--accent); font-weight: 600; background: var(--surface); }
tr:hover td { background: rgba(129,140,248,0.05); }
.badge { display: inline-block; padding: 0.15em 0.5em; border-radius: 4px;
         font-size: 0.8em; font-weight: 600; }
.badge-get { background: rgba(52,211,153,0.2); color: var(--green); }
.badge-post { background: rgba(129,140,248,0.2); color: var(--accent); }
.badge-put { background: rgba(251,191,36,0.2); color: var(--amber); }
.badge-delete { background: rgba(248,113,113,0.2); color: var(--red); }
.nav { display: flex; gap: 1.5rem; margin-bottom: 2rem; flex-wrap: wrap; }
.nav a { padding: 0.4rem 0.8rem; border: 1px solid var(--border); border-radius: 6px;
         transition: all 0.2s; }
.nav a:hover { background: var(--surface); border-color: var(--accent); text-decoration: none; }
.subtitle { color: var(--muted); font-size: 1rem; margin-bottom: 1.5rem; }
.card { background: var(--surface); border: 1px solid var(--border); border-radius: 8px;
        padding: 1.25rem; margin: 1rem 0; }
.param-table td:first-child { font-family: monospace; color: var(--green); white-space: nowrap; }
.param-table td:nth-child(2) { color: var(--muted); font-style: italic; }
.required { color: var(--red); font-weight: bold; }
.optional { color: var(--muted); }
ul { padding-left: 1.5rem; }
"#;

fn page(title: &str, body: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html lang="en"><head>
<meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>{title} — Zeus Docs</title>
<style>{CSS}</style>
</head><body>
<div class="container">
<nav class="nav">
  <a href="/docs">Home</a>
  <a href="/docs/openapi.json">OpenAPI</a>
  <a href="/docs/tools">Tools</a>
  <a href="/docs/config">Config</a>
  <a href="/docs/getting-started">Getting Started</a>
</nav>
{body}
</div></body></html>"#
    )
}

// ============================================================================
// GET /docs — Index
// ============================================================================

pub async fn docs_index(State(state): State<SharedState>) -> Html<String> {
    let st = state.read().await;
    let tool_count = st.tools.schemas().len();
    let body = format!(
        r#"<h1>Zeus Documentation</h1>
<p class="subtitle">Autonomous AI assistant — {tool_count} tools, 28 crates, 200+ API routes</p>

<div class="card">
<h3>API Reference</h3>
<p><a href="/docs/openapi.json">OpenAPI 3.0 Specification</a> — machine-readable spec for all REST endpoints.
Import into Postman, Swagger UI, or any OpenAPI-compatible tool.</p>
</div>

<div class="card">
<h3>Tool Reference</h3>
<p><a href="/docs/tools">All {tool_count} Tools</a> — complete reference for every tool available to the agent,
including parameters, types, and descriptions.</p>
</div>

<div class="card">
<h3>Configuration Guide</h3>
<p><a href="/docs/config">config.toml Reference</a> — every configuration section and field,
with types, defaults, and examples.</p>
</div>

<div class="card">
<h3>Getting Started</h3>
<p><a href="/docs/getting-started">Quick Start Guide</a> — install Zeus, run the onboarding wizard,
connect your first LLM provider, and start chatting.</p>
</div>

<h2>Quick Links</h2>
<table>
<tr><td><code>GET /health</code></td><td>Health check</td></tr>
<tr><td><code>POST /v1/chat</code></td><td>Send a message to the agent</td></tr>
<tr><td><code>GET /v1/ws</code></td><td>WebSocket streaming</td></tr>
<tr><td><code>POST /v1/chat/completions</code></td><td>OpenAI-compatible API</td></tr>
<tr><td><code>GET /v1/tools</code></td><td>List available tools</td></tr>
<tr><td><code>GET /v1/sessions</code></td><td>List sessions</td></tr>
<tr><td><code>GET /v1/status</code></td><td>Server status</td></tr>
<tr><td><code>GET /v1/doctor</code></td><td>Run diagnostics</td></tr>
</table>"#
    );
    Html(page("Home", &body))
}

// ============================================================================
// GET /docs/openapi.json — OpenAPI 3.0 Spec
// ============================================================================

pub async fn docs_openapi(State(state): State<SharedState>) -> Response {
    let st = state.read().await;
    let tool_schemas = st.tools.schemas();

    let mut paths = serde_json::Map::new();

    // --- hand-coded route catalogue (extracted from routes.rs) ---
    let routes: Vec<(&str, &str, &str, &str)> = vec![
        // (method, path, operationId, summary)
        ("get", "/", "healthRoot", "Health check"),
        ("get", "/health", "health", "Health check"),
        ("post", "/v1/chat", "chat", "Send message to agent"),
        (
            "post",
            "/v1/chat/completions",
            "openaiChatCompletions",
            "OpenAI-compatible chat completions",
        ),
        (
            "get",
            "/v1/models",
            "openaiListModels",
            "List models (OpenAI format)",
        ),
        // Sessions
        ("get", "/v1/sessions", "listSessions", "List sessions"),
        (
            "post",
            "/v1/sessions",
            "createSession",
            "Create new session",
        ),
        (
            "get",
            "/v1/sessions/{id}",
            "getSession",
            "Get session messages",
        ),
        (
            "delete",
            "/v1/sessions/{id}",
            "deleteSession",
            "Delete a session",
        ),
        (
            "post",
            "/v1/sessions/search",
            "searchSessions",
            "Search sessions",
        ),
        (
            "get",
            "/v1/sessions/{id}/stats",
            "getSessionStats",
            "Session statistics",
        ),
        (
            "get",
            "/v1/sessions/{id}/replay",
            "sessionReplay",
            "Full session replay",
        ),
        (
            "get",
            "/v1/sessions/{id}/replay/{turn}",
            "sessionReplayTurn",
            "Single turn replay",
        ),
        (
            "get",
            "/v1/sessions/{id}/raw",
            "getSessionRaw",
            "Raw JSONL session data",
        ),
        (
            "get",
            "/v1/sessions/{id}/audit",
            "getSessionAudit",
            "Session audit trail",
        ),
        (
            "get",
            "/v1/sessions/{id}/tools",
            "getSessionTools",
            "Session tool call chain",
        ),
        (
            "post",
            "/v1/sessions/{id}/branch",
            "createBranch",
            "Create session branch",
        ),
        (
            "get",
            "/v1/sessions/{id}/branches",
            "listBranches",
            "List session branches",
        ),
        // Tools
        ("get", "/v1/tools", "listTools", "List available tools"),
        ("post", "/v1/tools/{name}", "executeTool", "Execute a tool"),
        // Memory
        ("get", "/v1/memory", "getMemory", "Get workspace context"),
        (
            "post",
            "/v1/memory/remember",
            "remember",
            "Add fact to memory",
        ),
        ("post", "/v1/memory/note", "addNote", "Add daily note"),
        (
            "get",
            "/v1/memory/files",
            "listMemoryFiles",
            "List workspace files",
        ),
        (
            "post",
            "/v1/memory/files",
            "createMemoryFile",
            "Create workspace file",
        ),
        (
            "get",
            "/v1/memory/files/{path}",
            "readMemoryFile",
            "Read workspace file",
        ),
        (
            "put",
            "/v1/memory/files/{path}",
            "writeMemoryFile",
            "Write workspace file",
        ),
        (
            "delete",
            "/v1/memory/files/{path}",
            "deleteMemoryFile",
            "Delete workspace file",
        ),
        ("post", "/v1/memory/search", "searchMemory", "Search memory"),
        (
            "post",
            "/v1/memory/graph/search",
            "graphSearch",
            "Graph-augmented search",
        ),
        (
            "get",
            "/v1/memory/graph/{entity_id}",
            "getEntityGraph",
            "Get entity graph neighborhood",
        ),
        (
            "get",
            "/v1/memory/communities",
            "listCommunities",
            "List detected communities",
        ),
        ("post", "/v1/memory/sync", "syncMemory", "Sync memory"),
        (
            "get",
            "/v1/memory/timeline",
            "memoryTimeline",
            "Memory timeline",
        ),
        // Config
        ("get", "/v1/config", "getConfig", "Get config (sanitized)"),
        ("put", "/v1/config", "updateConfig", "Update config"),
        (
            "post",
            "/v1/config/test",
            "testProvider",
            "Test LLM provider",
        ),
        (
            "get",
            "/v1/config/providers",
            "getProviders",
            "List configured providers",
        ),
        (
            "post",
            "/v1/config/reload",
            "reloadConfig",
            "Reload config from disk",
        ),
        (
            "get",
            "/v1/config/history",
            "configHistory",
            "Config change history",
        ),
        // Activity & Stats
        ("get", "/v1/activity", "getActivity", "Activity feed"),
        ("get", "/v1/stats", "getStats", "Resource overview"),
        ("get", "/v1/status", "status", "Server status"),
        ("get", "/v1/doctor", "doctor", "Run diagnostics"),
        // Skills
        ("get", "/v1/skills", "listSkills", "List installed skills"),
        ("post", "/v1/skills", "installSkill", "Install skill"),
        ("put", "/v1/skills/{id}", "updateSkill", "Update skill"),
        ("delete", "/v1/skills/{id}", "deleteSkill", "Delete skill"),
        // MCP
        (
            "get",
            "/v1/mcp/servers",
            "listMcpServers",
            "List MCP connections",
        ),
        (
            "post",
            "/v1/mcp/servers",
            "addMcpServer",
            "Connect MCP server",
        ),
        (
            "delete",
            "/v1/mcp/servers/{id}",
            "deleteMcpServer",
            "Disconnect MCP server",
        ),
        (
            "get",
            "/v1/mcp/servers/{id}/tools",
            "listMcpServerTools",
            "List MCP server tools",
        ),
        (
            "post",
            "/v1/mcp/tools/{tool}/test",
            "testMcpTool",
            "Test an MCP tool",
        ),
        // Channels
        ("get", "/v1/channels", "listChannels", "List channels"),
        (
            "get",
            "/v1/channels/health",
            "channelHealth",
            "Channel health",
        ),
        ("post", "/v1/channels", "createChannel", "Create channel"),
        ("get", "/v1/channels/{id}", "getChannel", "Get channel"),
        (
            "put",
            "/v1/channels/{id}",
            "updateChannel",
            "Update channel",
        ),
        (
            "delete",
            "/v1/channels/{id}",
            "deleteChannel",
            "Delete channel",
        ),
        (
            "post",
            "/v1/channels/{id}/test",
            "testChannel",
            "Test channel",
        ),
        (
            "get",
            "/v1/channels/{id}/status",
            "channelStatus",
            "Channel status",
        ),
        ("post", "/v1/channels/{id}/poll", "sendPoll", "Send poll"),
        (
            "delete",
            "/v1/channels/{id}/poll/{message_id}",
            "stopPoll",
            "Stop poll",
        ),
        // Analytics
        (
            "get",
            "/v1/analytics/costs",
            "analyticsCosts",
            "Cost aggregation",
        ),
        (
            "get",
            "/v1/analytics/tokens",
            "analyticsTokens",
            "Token usage breakdown",
        ),
        (
            "get",
            "/v1/analytics/providers",
            "analyticsProviders",
            "Per-provider costs",
        ),
        (
            "get",
            "/v1/analytics/budgets",
            "analyticsBudgets",
            "Budget thresholds",
        ),
        (
            "get",
            "/v1/analytics/sessions",
            "analyticsSessions",
            "Session analytics",
        ),
        (
            "get",
            "/v1/analytics/daily",
            "analyticsDaily",
            "Daily analytics",
        ),
        (
            "get",
            "/v1/analytics/models",
            "analyticsModels",
            "Model usage analytics",
        ),
        // Security
        (
            "get",
            "/v1/security/threats",
            "securityThreats",
            "Threat log",
        ),
        (
            "get",
            "/v1/security/permissions",
            "securityPermissions",
            "Permission matrix",
        ),
        (
            "put",
            "/v1/security/permissions",
            "updateSecurityPermissions",
            "Update permissions",
        ),
        (
            "get",
            "/v1/security/keys",
            "securityKeys",
            "API key inventory",
        ),
        (
            "get",
            "/v1/security/allowlist",
            "securityAllowlist",
            "Shell command allowlist",
        ),
        (
            "put",
            "/v1/security/allowlist",
            "updateSecurityAllowlist",
            "Update allowlist",
        ),
        (
            "get",
            "/v1/security/audit",
            "securityAudit",
            "Security audit log",
        ),
        (
            "post",
            "/v1/security/rotate-key",
            "rotateKey",
            "Rotate auth key",
        ),
        (
            "get",
            "/v1/security/rotation-status",
            "rotationStatus",
            "Key rotation status",
        ),
        // Pipeline
        (
            "get",
            "/v1/pipeline/stats",
            "pipelineStats",
            "Pipeline stage metrics",
        ),
        // Context
        (
            "get",
            "/v1/context/journals",
            "listContextJournals",
            "List context journals",
        ),
        // Projects
        ("get", "/v1/projects", "listProjects", "List projects"),
        ("post", "/v1/projects", "createProject", "Create project"),
        ("get", "/v1/projects/{id}", "getProject", "Get project"),
        (
            "put",
            "/v1/projects/{id}",
            "updateProject",
            "Update project",
        ),
        (
            "delete",
            "/v1/projects/{id}",
            "deleteProject",
            "Delete project",
        ),
        (
            "put",
            "/v1/projects/{id}/agents",
            "assignProjectAgents",
            "Assign agents to project",
        ),
        // Agents
        ("get", "/v1/agents", "listAgents", "List agents"),
        ("post", "/v1/agents", "createAgent", "Create agent"),
        ("post", "/v1/agents/spawn", "spawnAgent", "Spawn agent"),
        ("get", "/v1/agents/{id}", "getAgent", "Get agent"),
        ("put", "/v1/agents/{id}", "updateAgent", "Update agent"),
        ("delete", "/v1/agents/{id}", "deleteAgent", "Delete agent"),
        (
            "post",
            "/v1/agents/{id}/chat",
            "agentChat",
            "Chat with agent",
        ),
        (
            "post",
            "/v1/agents/{id}/send",
            "sendToAgent",
            "Send message to agent",
        ),
        (
            "get",
            "/v1/agents/{id}/status",
            "agentStatus",
            "Agent runtime status",
        ),
        // Network
        (
            "get",
            "/v1/network/agents",
            "networkAgents",
            "Discovered agents",
        ),
        (
            "get",
            "/v1/network/discover",
            "networkDiscover",
            "Discovery results",
        ),
        (
            "get",
            "/v1/network/messages",
            "networkMessages",
            "Inter-agent messages",
        ),
        // Auth
        ("post", "/v1/auth/login", "authLogin", "Login"),
        ("get", "/v1/auth/status", "authStatus", "Auth status"),
        ("post", "/v1/auth/token", "authToken", "Create token"),
        ("post", "/v1/auth/logout", "authLogout", "Logout"),
        (
            "get",
            "/v1/auth/anthropic/login",
            "anthropicOAuthLogin",
            "Anthropic OAuth login",
        ),
        (
            "get",
            "/v1/auth/anthropic/callback",
            "anthropicOAuthCallback",
            "Anthropic OAuth callback",
        ),
        (
            "get",
            "/v1/auth/anthropic/status",
            "anthropicOAuthStatus",
            "Anthropic OAuth status",
        ),
        // Onboarding
        (
            "get",
            "/v1/onboarding/status",
            "onboardingStatus",
            "Onboarding status",
        ),
        (
            "post",
            "/v1/onboarding/complete",
            "onboardingComplete",
            "Complete onboarding",
        ),
        // Approvals
        (
            "get",
            "/v1/approvals",
            "listApprovals",
            "List pending approvals",
        ),
        (
            "post",
            "/v1/approvals/{id}/approve",
            "approveExecution",
            "Approve execution",
        ),
        (
            "post",
            "/v1/approvals/{id}/deny",
            "denyExecution",
            "Deny execution",
        ),
        // TTS
        (
            "get",
            "/v1/tts/providers",
            "listTtsProviders",
            "List TTS providers",
        ),
        (
            "post",
            "/v1/tts/synthesize",
            "ttsSynthesize",
            "Synthesize speech",
        ),
        (
            "post",
            "/v1/tts/synthesize/stream",
            "ttsSynthesizeStream",
            "Stream TTS",
        ),
        ("get", "/v1/tts/voices", "listTtsVoices", "List TTS voices"),
        // Sandbox
        (
            "get",
            "/v1/sandbox/policies",
            "listSandboxPolicies",
            "List sandbox policies",
        ),
        (
            "post",
            "/v1/sandbox/policies",
            "createSandboxPolicy",
            "Create sandbox policy",
        ),
        (
            "post",
            "/v1/sandbox/execute",
            "sandboxExecute",
            "Execute in sandbox",
        ),
        // Teams
        ("get", "/v1/teams", "listTeams", "List teams"),
        ("post", "/v1/teams", "createTeam", "Create team"),
        ("get", "/v1/teams/{id}", "getTeam", "Get team"),
        ("put", "/v1/teams/{id}", "updateTeam", "Update team"),
        ("delete", "/v1/teams/{id}", "deleteTeam", "Delete team"),
        (
            "post",
            "/v1/teams/recommend",
            "teamRecommend",
            "Team recommendation",
        ),
        // Delegations
        (
            "get",
            "/v1/delegations",
            "listDelegations",
            "List delegations",
        ),
        (
            "post",
            "/v1/delegations",
            "createDelegation",
            "Create delegation",
        ),
        (
            "post",
            "/v1/routing/recommend",
            "smartRoute",
            "Smart routing recommendation",
        ),
        // Schedules
        ("get", "/v1/schedules", "listSchedules", "List schedules"),
        ("post", "/v1/schedules", "createSchedule", "Create schedule"),
        ("get", "/v1/schedules/{id}", "getSchedule", "Get schedule"),
        (
            "put",
            "/v1/schedules/{id}",
            "updateSchedule",
            "Update schedule",
        ),
        (
            "delete",
            "/v1/schedules/{id}",
            "deleteSchedule",
            "Delete schedule",
        ),
        (
            "post",
            "/v1/schedules/{id}/pause",
            "pauseSchedule",
            "Pause schedule",
        ),
        (
            "post",
            "/v1/schedules/{id}/resume",
            "resumeSchedule",
            "Resume schedule",
        ),
        (
            "get",
            "/v1/schedules/{id}/runs",
            "listScheduleRuns",
            "List schedule runs",
        ),
        // Cost routing
        ("get", "/v1/routing/costs", "routingCosts", "Routing costs"),
        (
            "get",
            "/v1/routing/budget",
            "routingBudget",
            "Routing budget",
        ),
        (
            "post",
            "/v1/routing/cost-recommend",
            "routingRecommend",
            "Cost-based recommendation",
        ),
        // Extensions
        ("get", "/v1/extensions", "listExtensions", "List extensions"),
        (
            "post",
            "/v1/extensions",
            "installExtension",
            "Install extension",
        ),
        (
            "get",
            "/v1/extensions/{id}",
            "getExtension",
            "Get extension",
        ),
        (
            "put",
            "/v1/extensions/{id}",
            "updateExtension",
            "Update extension",
        ),
        (
            "delete",
            "/v1/extensions/{id}",
            "deleteExtension",
            "Delete extension",
        ),
        (
            "post",
            "/v1/extensions/{id}/start",
            "startExtension",
            "Start extension",
        ),
        (
            "post",
            "/v1/extensions/{id}/stop",
            "stopExtension",
            "Stop extension",
        ),
        // Images
        (
            "post",
            "/v1/images/generate",
            "generateImage",
            "Generate image",
        ),
        ("get", "/v1/images", "listImages", "List images"),
        ("get", "/v1/images/{id}", "getImage", "Get image"),
        // Prometheus
        (
            "post",
            "/v1/prometheus/plan",
            "prometheusCreatePlan",
            "Create strategic plan",
        ),
        (
            "get",
            "/v1/prometheus/plan/{id}",
            "prometheusGetPlan",
            "Get plan",
        ),
        (
            "post",
            "/v1/prometheus/execute",
            "prometheusExecute",
            "Execute plan",
        ),
        (
            "get",
            "/v1/prometheus/state",
            "prometheusState",
            "Prometheus state",
        ),
        // Workflows
        ("get", "/v1/workflows", "listWorkflows", "List workflows"),
        (
            "post",
            "/v1/workflows",
            "workflowFromChat",
            "Create workflow from chat",
        ),
        ("get", "/v1/workflows/{id}", "getWorkflow", "Get workflow"),
        (
            "get",
            "/v1/workflows/{id}/artifacts",
            "workflowArtifacts",
            "Workflow artifacts",
        ),
        (
            "get",
            "/v1/workflows/{id}/download",
            "workflowDownload",
            "Download workflow",
        ),
        // Orchestration
        (
            "post",
            "/v1/orchestrate/start",
            "orchestrateStart",
            "Start orchestration",
        ),
        (
            "post",
            "/v1/orchestrate/respond",
            "orchestrateRespond",
            "Respond to orchestration",
        ),
        (
            "get",
            "/v1/orchestrate/{id}",
            "orchestrateStatus",
            "Orchestration status",
        ),
        (
            "post",
            "/v1/orchestrate/{id}/confirm",
            "orchestrateConfirm",
            "Confirm orchestration",
        ),
        // Goals
        ("get", "/v1/goals", "goalsList", "List goals"),
        ("post", "/v1/goals", "goalsCreate", "Create goal"),
        ("post", "/v1/goals/analyze", "goalsAnalyze", "Analyze goal"),
        ("get", "/v1/goals/{id}", "goalsGet", "Get goal"),
        (
            "put",
            "/v1/goals/{id}/status",
            "goalsUpdateStatus",
            "Update goal status",
        ),
        // Peer review
        ("get", "/v1/reviews", "listReviews", "List reviews"),
        ("post", "/v1/reviews", "submitReview", "Submit review"),
        ("get", "/v1/reviews/{id}", "getReview", "Get review"),
        (
            "post",
            "/v1/reviews/{id}/approve",
            "approveReview",
            "Approve review",
        ),
        (
            "post",
            "/v1/reviews/{id}/reject",
            "rejectReview",
            "Reject review",
        ),
        // Marketplace
        (
            "get",
            "/v1/marketplace/listings",
            "marketplaceList",
            "List marketplace",
        ),
        (
            "post",
            "/v1/marketplace/listings",
            "marketplacePublish",
            "Publish to marketplace",
        ),
        (
            "post",
            "/v1/marketplace/trade",
            "marketplaceTrade",
            "Marketplace trade",
        ),
        (
            "get",
            "/v1/marketplace/ledger/{agent_id}",
            "marketplaceLedger",
            "Marketplace ledger",
        ),
        (
            "get",
            "/v1/marketplace/reputation/{agent_id}",
            "marketplaceReputation",
            "Agent reputation",
        ),
        // Economy
        (
            "get",
            "/v1/economy/wallets",
            "economyWallets",
            "List wallets",
        ),
        (
            "get",
            "/v1/economy/wallets/{agent_id}",
            "economyWallet",
            "Get wallet",
        ),
        (
            "get",
            "/v1/economy/transactions",
            "economyTransactions",
            "List transactions",
        ),
        // On-chain wallet (Solana)
        (
            "get",
            "/v1/wallet/onchain",
            "onchainWalletInfo",
            "On-chain wallet info (address, SOL balance, token balance, cluster)",
        ),
        (
            "get",
            "/v1/wallet/onchain/transactions",
            "onchainTransactions",
            "Recent on-chain transactions",
        ),
        (
            "post",
            "/v1/wallet/onchain/transfer",
            "onchainTransfer",
            "Execute devnet SPL token transfer (preflight + submit)",
        ),
        // Uploads
        ("post", "/v1/uploads", "uploadFile", "Upload file"),
        ("get", "/v1/uploads", "listUploads", "List uploads"),
        (
            "get",
            "/v1/uploads/{id}",
            "getUploadMetadata",
            "Get upload metadata",
        ),
        (
            "get",
            "/v1/uploads/{id}/download",
            "downloadFile",
            "Download file",
        ),
        (
            "get",
            "/v1/uploads/{id}/thumbnail",
            "getThumbnail",
            "Get thumbnail",
        ),
        (
            "delete",
            "/v1/uploads/{id}",
            "deleteUpload",
            "Delete upload",
        ),
        // Webhooks
        ("get", "/v1/webhooks", "webhookHealth", "Webhook health"),
        ("post", "/v1/webhooks", "receiveWebhook", "Receive webhook"),
        (
            "post",
            "/v1/webhooks/{source}",
            "receiveWebhookSource",
            "Receive from source",
        ),
        (
            "get",
            "/v1/webhooks/outbound",
            "listOutboundWebhooks",
            "List outbound webhooks",
        ),
        (
            "post",
            "/v1/webhooks/outbound",
            "registerOutboundWebhook",
            "Register webhook",
        ),
        (
            "delete",
            "/v1/webhooks/outbound/{id}",
            "deleteOutboundWebhook",
            "Delete webhook",
        ),
        (
            "get",
            "/v1/webhooks/triggers",
            "listTriggers",
            "List triggers",
        ),
        (
            "post",
            "/v1/webhooks/triggers",
            "createTrigger",
            "Create trigger",
        ),
        (
            "delete",
            "/v1/webhooks/triggers/{id}",
            "deleteTrigger",
            "Delete trigger",
        ),
        (
            "put",
            "/v1/webhooks/triggers/{id}/enable",
            "enableTrigger",
            "Enable trigger",
        ),
        (
            "put",
            "/v1/webhooks/triggers/{id}/disable",
            "disableTrigger",
            "Disable trigger",
        ),
        // Providers
        (
            "get",
            "/v1/providers",
            "listProviders",
            "List provider catalog",
        ),
        // Cron
        ("get", "/v1/cron/jobs", "listCronJobs", "List cron jobs"),
        ("post", "/v1/cron/jobs", "createCronJob", "Create cron job"),
        (
            "get",
            "/v1/cron/jobs/running",
            "listRunningCronJobs",
            "List currently running cron jobs with concurrency info",
        ),
        (
            "delete",
            "/v1/cron/jobs/{id}",
            "deleteCronJob",
            "Delete cron job",
        ),
        (
            "post",
            "/v1/cron/jobs/{id}/abort",
            "abortCronJob",
            "Abort a running cron job by cancellation token",
        ),
        (
            "get",
            "/v1/cron/jobs/{id}/history",
            "cronJobHistory",
            "Cron job history",
        ),
        (
            "get",
            "/v1/cron/templates",
            "listCronTemplates",
            "List cron templates",
        ),
        // Observatory
        (
            "get",
            "/v1/observatory/active-tasks",
            "observatoryActiveTasks",
            "Active tasks",
        ),
        (
            "get",
            "/v1/observatory/agent-stats",
            "observatoryAgentStats",
            "Agent stats",
        ),
        (
            "get",
            "/v1/observatory/channel-health",
            "observatoryChannelHealth",
            "Channel health",
        ),
        (
            "get",
            "/v1/observatory/cost-live",
            "observatoryCostLive",
            "Live costs",
        ),
        // Voice
        (
            "get",
            "/v1/tts/providers",
            "listTtsProviders2",
            "List TTS providers",
        ),
        (
            "get",
            "/v1/voice/inbound",
            "voiceInboundHealth",
            "Voice inbound health",
        ),
        (
            "post",
            "/v1/voice/inbound",
            "receiveVoiceInbound",
            "Receive voice inbound",
        ),
        (
            "post",
            "/v1/voice/recording-status",
            "receiveRecordingStatus",
            "Recording status",
        ),
        // Studio
        (
            "post",
            "/v1/studio",
            "studioChat",
            "Studio chat with custom system prompt",
        ),
        // OpenAI Responses API
        (
            "post",
            "/v1/responses",
            "createResponse",
            "Create response (OpenAI format)",
        ),
        ("get", "/v1/responses/{id}", "getResponse", "Get response"),
        (
            "delete",
            "/v1/responses/{id}",
            "deleteResponse",
            "Delete response",
        ),
        // Vector Stores
        (
            "post",
            "/v1/vector_stores",
            "createVectorStore",
            "Create vector store",
        ),
        (
            "get",
            "/v1/vector_stores",
            "listVectorStores",
            "List vector stores",
        ),
        (
            "get",
            "/v1/vector_stores/{id}",
            "getVectorStore",
            "Get vector store",
        ),
        (
            "delete",
            "/v1/vector_stores/{id}",
            "deleteVectorStore",
            "Delete vector store",
        ),
        (
            "post",
            "/v1/vector_stores/{id}/search",
            "searchVectorStore",
            "Search vector store",
        ),
        (
            "post",
            "/v1/vector_stores/{id}/files",
            "addFileToStore",
            "Add file to vector store",
        ),
        (
            "get",
            "/v1/vector_stores/{id}/files",
            "listStoreFiles",
            "List vector store files",
        ),
        // Batch API
        ("post", "/v1/batches", "createBatch", "Create batch request"),
        ("get", "/v1/batches/{id}", "getBatch", "Get batch status"),
        (
            "get",
            "/v1/batches/{id}/results",
            "getBatchResults",
            "Get batch results",
        ),
        // Embeddings
        (
            "post",
            "/v1/embeddings",
            "createEmbeddings",
            "Create embeddings",
        ),
        // Token counting
        (
            "post",
            "/v1/tokens/count",
            "countTokens",
            "Count tokens in text",
        ),
        // Credentials
        (
            "get",
            "/v1/credentials",
            "listCredentials",
            "List stored credentials",
        ),
        (
            "post",
            "/v1/credentials",
            "storeCredential",
            "Store a credential",
        ),
        (
            "delete",
            "/v1/credentials/{name}",
            "deleteCredential",
            "Delete a credential",
        ),
        // Skills (enriched endpoints)
        (
            "get",
            "/v1/skills/search",
            "searchSkills",
            "Search skills by query and category",
        ),
        (
            "get",
            "/v1/skills/categories",
            "listSkillCategories",
            "List skill categories with counts",
        ),
        (
            "post",
            "/v1/skills/clawhub/install",
            "installClawhubSkill",
            "Install skill from ClawHub",
        ),
        (
            "get",
            "/v1/skills/{id}",
            "getSkill",
            "Get full skill detail",
        ),
        (
            "get",
            "/v1/skills/{id}/schema",
            "getSkillSchema",
            "Get skill schema and raw SKILL.md",
        ),
        // Channels (extended)
        (
            "post",
            "/v1/channels/{id}/connect",
            "connectChannel",
            "Connect channel",
        ),
        (
            "post",
            "/v1/channels/{id}/disconnect",
            "disconnectChannel",
            "Disconnect channel",
        ),
        (
            "post",
            "/v1/channels/{id}/pair",
            "pairChannel",
            "Pair DM channel",
        ),
        (
            "post",
            "/v1/channels/{id}/verify",
            "verifyChannel",
            "Verify channel pairing",
        ),
        (
            "get",
            "/v1/channels/{id}/pairings",
            "listChannelPairings",
            "List channel pairings",
        ),
        // WhatsApp webhooks
        (
            "get",
            "/v1/webhooks/whatsapp",
            "whatsappWebhookHealth",
            "WhatsApp webhook health",
        ),
        (
            "post",
            "/v1/webhooks/whatsapp",
            "receiveWhatsappWebhook",
            "Receive WhatsApp webhook",
        ),
        // Agents (extended)
        (
            "post",
            "/v1/agents/team",
            "createAgentTeam",
            "Create agent team",
        ),
        (
            "post",
            "/v1/agents/{id}/chat",
            "agentChat",
            "Chat with specific agent",
        ),
        // Network (extended)
        (
            "post",
            "/v1/network/messages",
            "receiveNetworkMessage",
            "Receive inter-agent message",
        ),
        (
            "post",
            "/v1/network/send",
            "networkSend",
            "Send message to network peer",
        ),
        (
            "post",
            "/v1/network/broadcast",
            "networkBroadcast",
            "Broadcast to all network peers",
        ),
        // Fleet
        ("get", "/v1/fleet", "listFleetAgents", "List fleet agents"),
        (
            "post",
            "/v1/fleet/register",
            "registerFleetAgent",
            "Register agent in fleet",
        ),
        ("get", "/v1/fleet/{id}", "getFleetAgent", "Get fleet agent"),
        (
            "delete",
            "/v1/fleet/{id}",
            "deregisterFleetAgent",
            "Deregister fleet agent",
        ),
        (
            "post",
            "/v1/fleet/{id}/heartbeat",
            "fleetHeartbeat",
            "Fleet agent heartbeat",
        ),
        // Pantheon (multi-agent missions)
        (
            "post",
            "/v1/pantheon/missions",
            "createMission",
            "Create collaborative mission",
        ),
        (
            "get",
            "/v1/pantheon/missions",
            "listMissions",
            "List missions",
        ),
        (
            "get",
            "/v1/pantheon/missions/{id}",
            "getMission",
            "Get mission detail",
        ),
        (
            "post",
            "/v1/pantheon/missions/{id}/intervene",
            "interveneMission",
            "Intervene in mission",
        ),
        (
            "get",
            "/v1/pantheon/missions/{id}/feed",
            "getMissionFeed",
            "Mission activity feed",
        ),
        (
            "get",
            "/v1/pantheon/missions/{id}/artifacts",
            "getMissionArtifacts",
            "Mission artifacts",
        ),
        (
            "get",
            "/v1/pantheon/missions/{id}/artifacts/{name}/download",
            "downloadMissionArtifact",
            "Download mission artifact file",
        ),
        (
            "get",
            "/v1/pantheon/missions/{id}/events",
            "missionEvents",
            "Mission SSE events",
        ),
        (
            "post",
            "/v1/pantheon/missions/{id}/review",
            "reviewTask",
            "Review mission task",
        ),
        // Canvas (A2UI)
        (
            "post",
            "/v1/canvas/render",
            "canvasRender",
            "Render UI canvas",
        ),
        (
            "get",
            "/v1/canvas/components",
            "canvasComponents",
            "List canvas components",
        ),
        // Nodes (fleet agent connections)
        ("get", "/v1/nodes", "listNodes", "List connected nodes"),
        (
            "post",
            "/v1/nodes/broadcast",
            "broadcastNodes",
            "Broadcast to nodes",
        ),
        ("get", "/v1/nodes/{id}", "getNode", "Get node detail"),
        ("post", "/v1/nodes/{id}/invoke", "invokeNode", "Invoke node"),
        (
            "post",
            "/v1/nodes/{id}/event",
            "sendNodeEvent",
            "Send event to node",
        ),
        // WebSocket
        ("get", "/v1/ws", "wsHandler", "WebSocket streaming"),
        (
            "get",
            "/v1/ws/pubkey",
            "wsPubkeyHandler",
            "WebSocket public key",
        ),
        (
            "get",
            "/v1/ws/nodes",
            "nodeWsHandler",
            "Node WebSocket (fleet agent connections)",
        ),
        // Docs (self-referential)
        ("get", "/docs", "docsIndex", "Documentation index"),
        ("get", "/docs/openapi.json", "docsOpenapi", "OpenAPI spec"),
        ("get", "/docs/tools", "docsTools", "Tool reference"),
        ("get", "/docs/config", "docsConfig", "Config guide"),
        (
            "get",
            "/docs/getting-started",
            "docsGettingStarted",
            "Getting started",
        ),
    ];

    // Build request/response schemas for important endpoints
    let endpoint_schemas = build_endpoint_schemas();

    for (method, path, op_id, summary) in &routes {
        let path_key = path.to_string();
        let entry = paths
            .entry(path_key)
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
        if let Some(obj) = entry.as_object_mut() {
            let mut operation = json!({
                "operationId": op_id,
                "summary": summary,
                "tags": [tag_from_path(path)],
                "responses": {
                    "200": { "description": "Success" }
                }
            });

            // Enrich with request/response schemas if available
            if let Some(schema) = endpoint_schemas.get(*op_id)
                && let Some(op_obj) = operation.as_object_mut()
            {
                if let Some(req_body) = schema.get("requestBody") {
                    op_obj.insert("requestBody".to_string(), req_body.clone());
                }
                if let Some(responses) = schema.get("responses") {
                    op_obj.insert("responses".to_string(), responses.clone());
                }
                if let Some(params) = schema.get("parameters") {
                    op_obj.insert("parameters".to_string(), params.clone());
                }
            }

            obj.insert(method.to_string(), operation);
        }
    }

    // Add tool schemas as components
    let mut all_schemas = serde_json::Map::new();
    for schema in &tool_schemas {
        all_schemas.insert(
            format!("Tool_{}", schema.name),
            json!({
                "type": "object",
                "description": schema.description,
                "properties": schema.parameters.get("properties").unwrap_or(&json!({})),
                "required": schema.parameters.get("required").unwrap_or(&json!([]))
            }),
        );
    }

    // Add API model schemas
    for (name, schema) in build_model_schemas() {
        all_schemas.insert(name, schema);
    }

    let spec = json!({
        "openapi": "3.0.3",
        "info": {
            "title": "Zeus API",
            "description": "Autonomous AI assistant REST API — 21 crates, 212 tools, 5 frontends",
            "version": crate::VERSION,
            "license": {
                "name": "MIT OR Apache-2.0"
            }
        },
        "servers": [
            { "url": "http://127.0.0.1:8080", "description": "Local gateway" }
        ],
        "paths": paths,
        "components": {
            "schemas": all_schemas,
            "securitySchemes": {
                "bearerAuth": {
                    "type": "http",
                    "scheme": "bearer"
                },
                "apiKeyAuth": {
                    "type": "apiKey",
                    "in": "header",
                    "name": "X-Zeus-Api-Key"
                }
            }
        },
        "security": [
            { "bearerAuth": [] },
            { "apiKeyAuth": [] }
        ]
    });

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        serde_json::to_string_pretty(&spec).unwrap_or_default(),
    )
        .into_response()
}

/// Build request/response schemas for the top ~20 endpoints.
fn build_endpoint_schemas() -> std::collections::HashMap<&'static str, serde_json::Value> {
    use std::collections::HashMap;
    let mut m: HashMap<&str, serde_json::Value> = HashMap::new();

    // POST /v1/chat
    m.insert("chat", json!({
        "requestBody": {
            "required": true,
            "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ChatRequest" } } }
        },
        "responses": {
            "200": {
                "description": "Agent response",
                "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ChatResponse" } } }
            }
        }
    }));

    // POST /v1/chat/completions (OpenAI compat)
    m.insert("openaiChatCompletions", json!({
        "requestBody": {
            "required": true,
            "content": { "application/json": { "schema": { "$ref": "#/components/schemas/OpenAIChatRequest" } } }
        },
        "responses": {
            "200": {
                "description": "OpenAI ChatCompletion response (streaming or non-streaming)",
                "content": { "application/json": { "schema": { "$ref": "#/components/schemas/OpenAIChatResponse" } } }
            }
        }
    }));

    // GET /v1/sessions
    m.insert("listSessions", json!({
        "parameters": [
            { "name": "limit", "in": "query", "schema": { "type": "integer", "default": 20 }, "description": "Max results" },
            { "name": "offset", "in": "query", "schema": { "type": "integer", "default": 0 }, "description": "Offset" }
        ],
        "responses": {
            "200": {
                "description": "Session list",
                "content": { "application/json": { "schema": { "$ref": "#/components/schemas/SessionList" } } }
            }
        }
    }));

    // GET /v1/tools
    m.insert("listTools", json!({
        "responses": {
            "200": {
                "description": "Available tools",
                "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ToolList" } } }
            }
        }
    }));

    // POST /v1/tools/{name}
    m.insert("executeTool", json!({
        "requestBody": {
            "required": true,
            "content": { "application/json": { "schema": { "type": "object", "description": "Tool-specific parameters" } } }
        },
        "responses": {
            "200": {
                "description": "Tool execution result",
                "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ToolResult" } } }
            }
        }
    }));

    // GET /v1/memory
    m.insert("getMemory", json!({
        "responses": {
            "200": {
                "description": "Workspace context",
                "content": { "application/json": { "schema": { "$ref": "#/components/schemas/MemoryContext" } } }
            }
        }
    }));

    // POST /v1/memory/remember
    m.insert("remember", json!({
        "requestBody": {
            "required": true,
            "content": { "application/json": { "schema": {
                "type": "object",
                "required": ["fact"],
                "properties": { "fact": { "type": "string", "description": "Fact to remember" } }
            } } }
        }
    }));

    // POST /v1/memory/search
    m.insert("searchMemory", json!({
        "requestBody": {
            "required": true,
            "content": { "application/json": { "schema": {
                "type": "object",
                "required": ["query"],
                "properties": {
                    "query": { "type": "string" },
                    "limit": { "type": "integer", "default": 10 },
                    "mode": { "type": "string", "enum": ["text", "semantic", "hybrid"], "default": "hybrid" }
                }
            } } }
        }
    }));

    // GET /v1/config
    m.insert("getConfig", json!({
        "responses": {
            "200": {
                "description": "Current configuration (secrets redacted)",
                "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Config" } } }
            }
        }
    }));

    // GET /v1/status
    m.insert("status", json!({
        "responses": {
            "200": {
                "description": "Server status",
                "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ServerStatus" } } }
            }
        }
    }));

    // GET /v1/skills
    m.insert("listSkills", json!({
        "responses": {
            "200": {
                "description": "Installed skills with metadata",
                "content": { "application/json": { "schema": { "$ref": "#/components/schemas/SkillList" } } }
            }
        }
    }));

    // GET /v1/skills/search
    m.insert("searchSkills", json!({
        "parameters": [
            { "name": "q", "in": "query", "schema": { "type": "string" }, "description": "Search query" },
            { "name": "category", "in": "query", "schema": { "type": "string" }, "description": "Category filter" },
            { "name": "enabled", "in": "query", "schema": { "type": "boolean" }, "description": "Enabled filter" }
        ]
    }));

    // GET /v1/channels
    m.insert("listChannels", json!({
        "responses": {
            "200": {
                "description": "Channel list",
                "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ChannelList" } } }
            }
        }
    }));

    // POST /v1/agents/spawn
    m.insert("spawnAgent", json!({
        "requestBody": {
            "required": true,
            "content": { "application/json": { "schema": { "$ref": "#/components/schemas/SpawnAgentRequest" } } }
        }
    }));

    // POST /v1/cron/jobs
    m.insert("createCronJob", json!({
        "requestBody": {
            "required": true,
            "content": { "application/json": { "schema": { "$ref": "#/components/schemas/CronJobRequest" } } }
        }
    }));

    // POST /v1/pantheon/missions
    m.insert("createMission", json!({
        "requestBody": {
            "required": true,
            "content": { "application/json": { "schema": { "$ref": "#/components/schemas/MissionRequest" } } }
        }
    }));

    // GET /v1/doctor
    m.insert("doctor", json!({
        "responses": {
            "200": {
                "description": "Diagnostic results",
                "content": { "application/json": { "schema": { "$ref": "#/components/schemas/DiagnosticReport" } } }
            }
        }
    }));

    // POST /v1/uploads
    m.insert(
        "uploadFile",
        json!({
            "requestBody": {
                "required": true,
                "content": { "multipart/form-data": { "schema": {
                    "type": "object",
                    "required": ["file"],
                    "properties": { "file": { "type": "string", "format": "binary" } }
                } } }
            }
        }),
    );

    m
}

/// Build reusable model schemas for the components section.
fn build_model_schemas() -> Vec<(String, serde_json::Value)> {
    vec![
        (
            "ChatRequest".into(),
            json!({
                "type": "object",
                "required": ["message"],
                "properties": {
                    "message": { "type": "string", "description": "User message" },
                    "session_id": { "type": "string", "description": "Session ID (omit for new session)" },
                    "stream": { "type": "boolean", "default": false, "description": "Enable streaming" }
                }
            }),
        ),
        (
            "ChatResponse".into(),
            json!({
                "type": "object",
                "properties": {
                    "response": { "type": "string", "description": "Agent response text" },
                    "session_id": { "type": "string" },
                    "tool_calls": { "type": "array", "items": { "$ref": "#/components/schemas/ToolCall" } }
                }
            }),
        ),
        (
            "OpenAIChatRequest".into(),
            json!({
                "type": "object",
                "required": ["messages"],
                "properties": {
                    "model": { "type": "string", "description": "Model identifier (provider/model)" },
                    "messages": { "type": "array", "items": { "$ref": "#/components/schemas/Message" } },
                    "stream": { "type": "boolean", "default": false },
                    "temperature": { "type": "number", "minimum": 0, "maximum": 2 },
                    "max_tokens": { "type": "integer" }
                }
            }),
        ),
        (
            "OpenAIChatResponse".into(),
            json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "object": { "type": "string", "enum": ["chat.completion"] },
                    "model": { "type": "string" },
                    "choices": { "type": "array", "items": {
                        "type": "object",
                        "properties": {
                            "index": { "type": "integer" },
                            "message": { "$ref": "#/components/schemas/Message" },
                            "finish_reason": { "type": "string" }
                        }
                    }},
                    "usage": { "$ref": "#/components/schemas/TokenUsage" }
                }
            }),
        ),
        (
            "Message".into(),
            json!({
                "type": "object",
                "required": ["role", "content"],
                "properties": {
                    "role": { "type": "string", "enum": ["system", "user", "assistant"] },
                    "content": { "type": "string" }
                }
            }),
        ),
        (
            "TokenUsage".into(),
            json!({
                "type": "object",
                "properties": {
                    "prompt_tokens": { "type": "integer" },
                    "completion_tokens": { "type": "integer" },
                    "total_tokens": { "type": "integer" }
                }
            }),
        ),
        (
            "SessionList".into(),
            json!({
                "type": "object",
                "properties": {
                    "sessions": { "type": "array", "items": { "$ref": "#/components/schemas/SessionSummary" } },
                    "total": { "type": "integer" }
                }
            }),
        ),
        (
            "SessionSummary".into(),
            json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "created_at": { "type": "string", "format": "date-time" },
                    "message_count": { "type": "integer" },
                    "preview": { "type": "string", "description": "First message preview" }
                }
            }),
        ),
        (
            "ToolList".into(),
            json!({
                "type": "object",
                "properties": {
                    "tools": { "type": "array", "items": { "$ref": "#/components/schemas/ToolSchema" } }
                }
            }),
        ),
        (
            "ToolSchema".into(),
            json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "description": { "type": "string" },
                    "parameters": { "type": "object", "description": "JSON Schema for parameters" }
                }
            }),
        ),
        (
            "ToolCall".into(),
            json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "arguments": { "type": "object" }
                }
            }),
        ),
        (
            "ToolResult".into(),
            json!({
                "type": "object",
                "properties": {
                    "success": { "type": "boolean" },
                    "output": { "type": "string" },
                    "error": { "type": "string" }
                }
            }),
        ),
        (
            "MemoryContext".into(),
            json!({
                "type": "object",
                "properties": {
                    "context": { "type": "string", "description": "Full workspace context" },
                    "memory_facts": { "type": "array", "items": { "type": "string" } }
                }
            }),
        ),
        (
            "Config".into(),
            json!({
                "type": "object",
                "properties": {
                    "model": { "type": "string", "description": "Active model (provider/model)" },
                    "workspace": { "type": "string" },
                    "sessions": { "type": "string" },
                    "max_iterations": { "type": "integer" }
                }
            }),
        ),
        (
            "ServerStatus".into(),
            json!({
                "type": "object",
                "properties": {
                    "model": { "type": "string" },
                    "provider": { "type": "string" },
                    "sessions_count": { "type": "integer" },
                    "tools_count": { "type": "integer" },
                    "uptime_seconds": { "type": "integer" },
                    "version": { "type": "string" }
                }
            }),
        ),
        (
            "SkillList".into(),
            json!({
                "type": "object",
                "properties": {
                    "skills": { "type": "array", "items": { "$ref": "#/components/schemas/SkillDetail" } },
                    "total": { "type": "integer" }
                }
            }),
        ),
        (
            "SkillDetail".into(),
            json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "name": { "type": "string" },
                    "description": { "type": "string" },
                    "enabled": { "type": "boolean" },
                    "category": { "type": "string" },
                    "tags": { "type": "array", "items": { "type": "string" } },
                    "emoji": { "type": "string" },
                    "version": { "type": "string" },
                    "tools_count": { "type": "integer" }
                }
            }),
        ),
        (
            "ChannelList".into(),
            json!({
                "type": "object",
                "properties": {
                    "channels": { "type": "array", "items": { "$ref": "#/components/schemas/Channel" } }
                }
            }),
        ),
        (
            "Channel".into(),
            json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "type": { "type": "string", "enum": ["telegram", "discord", "slack", "email", "imessage", "whatsapp", "signal", "matrix"] },
                    "name": { "type": "string" },
                    "status": { "type": "string", "enum": ["connected", "disconnected", "error"] }
                }
            }),
        ),
        (
            "SpawnAgentRequest".into(),
            json!({
                "type": "object",
                "required": ["name"],
                "properties": {
                    "name": { "type": "string", "description": "Agent name" },
                    "model": { "type": "string", "description": "Model override (provider/model)" },
                    "system_prompt": { "type": "string" },
                    "tools": { "type": "array", "items": { "type": "string" }, "description": "Allowed tool names" }
                }
            }),
        ),
        (
            "CronJobRequest".into(),
            json!({
                "type": "object",
                "required": ["name", "cron", "task_type"],
                "properties": {
                    "name": { "type": "string", "description": "Job name" },
                    "cron": { "type": "string", "description": "Cron expression or human-readable (e.g. 'every 5 minutes', 'daily at 3pm')" },
                    "enabled": { "type": "boolean", "default": true },
                    "task_type": { "$ref": "#/components/schemas/TaskType" }
                }
            }),
        ),
        (
            "TaskType".into(),
            json!({
                "oneOf": [
                    { "type": "object", "required": ["type", "frequency"], "properties": {
                        "type": { "type": "string", "enum": ["heartbeat"] },
                        "frequency": { "type": "string" }
                    }},
                    { "type": "object", "required": ["type", "prompt"], "properties": {
                        "type": { "type": "string", "enum": ["llm_prompt"] },
                        "prompt": { "type": "string" }
                    }},
                    { "type": "object", "required": ["type", "command"], "properties": {
                        "type": { "type": "string", "enum": ["shell"] },
                        "command": { "type": "string" }
                    }},
                    { "type": "object", "required": ["type", "content"], "properties": {
                        "type": { "type": "string", "enum": ["workspace_note"] },
                        "content": { "type": "string" }
                    }}
                ]
            }),
        ),
        (
            "MissionRequest".into(),
            json!({
                "type": "object",
                "required": ["goal"],
                "properties": {
                    "goal": { "type": "string", "description": "Mission objective" },
                    "agents": { "type": "array", "items": { "type": "string" }, "description": "Agent IDs to include" },
                    "max_steps": { "type": "integer", "default": 20 }
                }
            }),
        ),
        (
            "DiagnosticReport".into(),
            json!({
                "type": "object",
                "properties": {
                    "checks": { "type": "array", "items": {
                        "type": "object",
                        "properties": {
                            "name": { "type": "string" },
                            "status": { "type": "string", "enum": ["ok", "warn", "error"] },
                            "message": { "type": "string" }
                        }
                    }}
                }
            }),
        ),
    ]
}

fn tag_from_path(path: &str) -> &str {
    if path.starts_with("/v1/sessions") {
        return "Sessions";
    }
    if path.starts_with("/v1/tools") {
        return "Tools";
    }
    if path.starts_with("/v1/memory") {
        return "Memory";
    }
    if path.starts_with("/v1/config") {
        return "Config";
    }
    if path.starts_with("/v1/channels") {
        return "Channels";
    }
    if path.starts_with("/v1/analytics") {
        return "Analytics";
    }
    if path.starts_with("/v1/security") {
        return "Security";
    }
    if path.starts_with("/v1/skills") {
        return "Skills";
    }
    if path.starts_with("/v1/mcp") {
        return "MCP";
    }
    if path.starts_with("/v1/agents") {
        return "Agents";
    }
    if path.starts_with("/v1/projects") {
        return "Projects";
    }
    if path.starts_with("/v1/network") {
        return "Network";
    }
    if path.starts_with("/v1/auth") {
        return "Auth";
    }
    if path.starts_with("/v1/onboarding") {
        return "Onboarding";
    }
    if path.starts_with("/v1/approvals") {
        return "Approvals";
    }
    if path.starts_with("/v1/tts") || path.starts_with("/v1/voice") {
        return "Voice";
    }
    if path.starts_with("/v1/sandbox") {
        return "Sandbox";
    }
    if path.starts_with("/v1/teams") || path.starts_with("/v1/delegations") {
        return "Teams";
    }
    if path.starts_with("/v1/schedules") {
        return "Schedules";
    }
    if path.starts_with("/v1/routing") {
        return "Routing";
    }
    if path.starts_with("/v1/extensions") {
        return "Extensions";
    }
    if path.starts_with("/v1/images") {
        return "Images";
    }
    if path.starts_with("/v1/prometheus") {
        return "Prometheus";
    }
    if path.starts_with("/v1/wallet/onchain") {
        return "On-chain Wallet";
    }
    if path.starts_with("/v1/workflows") {
        return "Workflows";
    }
    if path.starts_with("/v1/orchestrate") {
        return "Orchestration";
    }
    if path.starts_with("/v1/goals") {
        return "Goals";
    }
    if path.starts_with("/v1/reviews") {
        return "Reviews";
    }
    if path.starts_with("/v1/marketplace") {
        return "Marketplace";
    }
    if path.starts_with("/v1/economy") {
        return "Economy";
    }
    if path.starts_with("/v1/uploads") {
        return "Uploads";
    }
    if path.starts_with("/v1/webhooks") {
        return "Webhooks";
    }
    if path.starts_with("/v1/cron") {
        return "Cron";
    }
    if path.starts_with("/v1/observatory") {
        return "Observatory";
    }
    if path.starts_with("/v1/ws") {
        return "WebSocket";
    }
    if path.starts_with("/v1/fleet") {
        return "Fleet";
    }
    if path.starts_with("/v1/pantheon") {
        return "Pantheon";
    }
    if path.starts_with("/v1/canvas") {
        return "Canvas";
    }
    if path.starts_with("/v1/nodes") {
        return "Nodes";
    }
    if path.starts_with("/v1/credentials") {
        return "Credentials";
    }
    if path.starts_with("/v1/vector_stores") {
        return "VectorStores";
    }
    if path.starts_with("/v1/batches") {
        return "Batch";
    }
    if path.starts_with("/v1/embeddings") {
        return "Embeddings";
    }
    if path.starts_with("/v1/tokens") {
        return "Tokens";
    }
    if path.starts_with("/v1/responses") {
        return "Responses";
    }
    if path.starts_with("/v1/studio") {
        return "Studio";
    }
    if path.starts_with("/docs") {
        return "Docs";
    }
    "General"
}

// ============================================================================
// GET /docs/tools — Tool Reference
// ============================================================================

pub async fn docs_tools(State(state): State<SharedState>) -> Html<String> {
    let st = state.read().await;
    let schemas = st.tools.schemas();

    let mut rows = String::new();
    for (i, schema) in schemas.iter().enumerate() {
        let params = schema
            .parameters
            .get("properties")
            .and_then(|p| p.as_object())
            .cloned()
            .unwrap_or_default();
        let required: Vec<String> = schema
            .parameters
            .get("required")
            .and_then(|r| r.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let mut param_rows = String::new();
        for (pname, pval) in &params {
            let ptype = pval
                .get("type")
                .and_then(|t| t.as_str())
                .unwrap_or("string");
            let pdesc = pval
                .get("description")
                .and_then(|d| d.as_str())
                .unwrap_or("");
            let req_badge = if required.contains(pname) {
                r#"<span class="required">required</span>"#
            } else {
                r#"<span class="optional">optional</span>"#
            };
            param_rows.push_str(&format!(
                "<tr><td><code>{pname}</code></td><td>{ptype}</td><td>{req_badge}</td><td>{pdesc}</td></tr>"
            ));
        }

        let param_table = if param_rows.is_empty() {
            "<p><em>No parameters</em></p>".to_string()
        } else {
            format!(
                r#"<table class="param-table"><tr><th>Name</th><th>Type</th><th></th><th>Description</th></tr>{param_rows}</table>"#
            )
        };

        rows.push_str(&format!(
            r#"<div class="card">
<h3>{idx}. <code>{name}</code></h3>
<p>{desc}</p>
{param_table}
</div>"#,
            idx = i + 1,
            name = schema.name,
            desc = schema.description,
        ));
    }

    let body = format!(
        r#"<h1>Tool Reference</h1>
<p class="subtitle">{count} tools available — 14 core + macOS automation (Talos)</p>
{rows}"#,
        count = schemas.len(),
    );
    Html(page("Tool Reference", &body))
}

// ============================================================================
// GET /docs/config — Configuration Guide
// ============================================================================

pub async fn docs_config(_state: State<SharedState>) -> Html<String> {
    let body = r#"<h1>Configuration Guide</h1>
<p class="subtitle">All settings for <code>~/.zeus/config.toml</code></p>

<h2>Top-Level Settings</h2>
<table>
<tr><th>Field</th><th>Type</th><th>Default</th><th>Description</th></tr>
<tr><td><code>model</code></td><td>string</td><td><code>"ollama/llama3.2"</code></td><td>Model string: <code>provider/model-name</code></td></tr>
<tr><td><code>workspace</code></td><td>path</td><td><code>~/.zeus/workspace</code></td><td>Workspace directory for memory files</td></tr>
<tr><td><code>sessions</code></td><td>path</td><td><code>~/.zeus/sessions</code></td><td>Sessions directory for JSONL files</td></tr>
<tr><td><code>max_iterations</code></td><td>integer</td><td><code>20</code></td><td>Maximum agent iterations per request</td></tr>
<tr><td><code>max_subagent_iterations</code></td><td>integer</td><td><code>15</code></td><td>Maximum subagent iterations</td></tr>
<tr><td><code>suppress_tool_errors</code></td><td>bool</td><td><code>false</code></td><td>Show generic error instead of detailed tool errors</td></tr>
<tr><td><code>thinking_level</code></td><td>string?</td><td><code>null</code></td><td>Extended thinking level: low/medium/high/xhigh</td></tr>
<tr><td><code>onboarding_complete</code></td><td>bool</td><td><code>false</code></td><td>Whether onboarding wizard has been completed</td></tr>
</table>

<h2>[tui]</h2>
<table>
<tr><th>Field</th><th>Type</th><th>Default</th><th>Description</th></tr>
<tr><td><code>theme</code></td><td>string</td><td><code>"dark"</code></td><td>TUI color theme</td></tr>
<tr><td><code>vim_mode</code></td><td>bool</td><td><code>false</code></td><td>Enable vim-style keybindings</td></tr>
</table>

<h2>[auth]</h2>
<table>
<tr><th>Field</th><th>Type</th><th>Default</th><th>Description</th></tr>
<tr><td><code>use_oauth</code></td><td>bool</td><td><code>false</code></td><td>Use OAuth instead of API keys for Anthropic</td></tr>
<tr><td><code>anthropic_client_id</code></td><td>string?</td><td><code>null</code></td><td>OAuth client ID (defaults to built-in)</td></tr>
<tr><td><code>anthropic_redirect_uri</code></td><td>string?</td><td><code>null</code></td><td>OAuth redirect URI</td></tr>
</table>

<h2>[ollama]</h2>
<table>
<tr><th>Field</th><th>Type</th><th>Default</th><th>Description</th></tr>
<tr><td><code>url</code></td><td>string</td><td><code>"http://localhost:11434"</code></td><td>Ollama server URL (or <code>OLLAMA_HOST</code> env)</td></tr>
<tr><td><code>preferred_model</code></td><td>string?</td><td><code>null</code></td><td>Preferred model for auto-detection</td></tr>
</table>

<h2>[mnemosyne]</h2>
<p>Advanced memory with SQLite FTS5 + vector embeddings.</p>
<table>
<tr><th>Field</th><th>Type</th><th>Default</th><th>Description</th></tr>
<tr><td><code>db_path</code></td><td>path</td><td><code>~/.zeus/memory.db</code></td><td>SQLite database file</td></tr>
<tr><td><code>enable_fts</code></td><td>bool</td><td><code>true</code></td><td>Enable FTS5 full-text search</td></tr>
<tr><td><code>max_messages_per_session</code></td><td>integer</td><td><code>10000</code></td><td>Max messages per session</td></tr>
<tr><td><code>enable_embeddings</code></td><td>bool</td><td><code>false</code></td><td>Enable vector embeddings</td></tr>
<tr><td><code>embedding_dim</code></td><td>integer</td><td><code>768</code></td><td>Embedding dimensions</td></tr>
<tr><td><code>embedding_model</code></td><td>string</td><td><code>"nomic-embed-text"</code></td><td>Model for embeddings</td></tr>
<tr><td><code>vector_weight</code></td><td>float</td><td><code>0.7</code></td><td>Weight for vector score in hybrid search</td></tr>
<tr><td><code>text_weight</code></td><td>float</td><td><code>0.3</code></td><td>Weight for BM25 score in hybrid search</td></tr>
<tr><td><code>enable_session_indexing</code></td><td>bool</td><td><code>true</code></td><td>Index session transcripts</td></tr>
<tr><td><code>enable_qmd</code></td><td>bool</td><td><code>false</code></td><td>Enable QMD reranking sidecar</td></tr>
</table>

<h2>[athena]</h2>
<p>Documentation engine — Obsidian integration.</p>
<table>
<tr><th>Field</th><th>Type</th><th>Default</th><th>Description</th></tr>
<tr><td><code>vault_path</code></td><td>path</td><td><code>~/Documents/Zeus</code></td><td>Path to Obsidian vault</td></tr>
</table>

<h2>[aegis]</h2>
<p>Security sandboxing and permissions.</p>
<table>
<tr><th>Field</th><th>Type</th><th>Default</th><th>Description</th></tr>
<tr><td><code>keychain_service</code></td><td>string</td><td><code>"zeus"</code></td><td>macOS Keychain service name</td></tr>
<tr><td><code>sandbox_level</code></td><td>string</td><td><code>"none"</code></td><td>none / basic / standard / strict / paranoid</td></tr>
<tr><td><code>audit_path</code></td><td>path</td><td><code>~/.zeus/audit.log</code></td><td>Audit log file</td></tr>
<tr><td><code>permissions</code></td><td>string[]</td><td><code>["*"]</code></td><td>Allowed operations</td></tr>
<tr><td><code>network_allowlist</code></td><td>string[]</td><td><code>["*"]</code></td><td>Allowed network targets</td></tr>
<tr><td><code>require_confirmation_for</code></td><td>string[]</td><td><code>[]</code></td><td>Tools requiring approval</td></tr>
<tr><td><code>approval_timeout_secs</code></td><td>integer</td><td><code>1800</code></td><td>Approval timeout (30 min)</td></tr>
</table>

<h2>[hermes]</h2>
<p>Notification routing.</p>
<table>
<tr><th>Field</th><th>Type</th><th>Default</th><th>Description</th></tr>
<tr><td><code>default_channels</code></td><td>string[]</td><td><code>[]</code></td><td>Default notification channels</td></tr>
<tr><td><code>batch_low_priority</code></td><td>bool</td><td><code>false</code></td><td>Batch low-priority notifications</td></tr>
</table>

<h2>[prometheus]</h2>
<p>Brain/orchestration — planning, heartbeat, cron.</p>
<table>
<tr><th>Field</th><th>Type</th><th>Default</th><th>Description</th></tr>
<tr><td><code>enable_heartbeat</code></td><td>bool</td><td><code>false</code></td><td>Enable heartbeat tasks</td></tr>
<tr><td><code>heartbeat_interval_secs</code></td><td>integer</td><td><code>3600</code></td><td>Heartbeat check interval</td></tr>
<tr><td><code>enable_cognitive</code></td><td>bool</td><td><code>false</code></td><td>Enable Nous cognitive integration</td></tr>
<tr><td><code>max_iterations</code></td><td>integer</td><td><code>20</code></td><td>Max iterations per request</td></tr>
</table>

<h2>[nous]</h2>
<p>Cognitive engine — intent recognition and learning.</p>
<table>
<tr><th>Field</th><th>Type</th><th>Default</th><th>Description</th></tr>
<tr><td><code>enable_intent</code></td><td>bool</td><td><code>true</code></td><td>Enable intent recognition</td></tr>
<tr><td><code>enable_learning</code></td><td>bool</td><td><code>false</code></td><td>Learn from interactions</td></tr>
</table>

<h2>[talos]</h2>
<p>macOS automation tools — enable/disable categories.</p>
<table>
<tr><th>Field</th><th>Type</th><th>Default</th><th>Description</th></tr>
<tr><td><code>calendar</code></td><td>bool</td><td><code>true</code></td><td>Calendar tools</td></tr>
<tr><td><code>notes</code></td><td>bool</td><td><code>true</code></td><td>Notes tools</td></tr>
<tr><td><code>reminders</code></td><td>bool</td><td><code>true</code></td><td>Reminders tools</td></tr>
<tr><td><code>contacts</code></td><td>bool</td><td><code>true</code></td><td>Contacts tools</td></tr>
<tr><td><code>browser</code></td><td>bool</td><td><code>true</code></td><td>Browser tools</td></tr>
<tr><td><code>system</code></td><td>bool</td><td><code>true</code></td><td>System tools</td></tr>
<tr><td><code>network</code></td><td>bool</td><td><code>true</code></td><td>Network tools</td></tr>
</table>

<h2>[gateway]</h2>
<p>Unified daemon configuration.</p>
<table>
<tr><th>Field</th><th>Type</th><th>Default</th><th>Description</th></tr>
<tr><td><code>host</code></td><td>string</td><td><code>"127.0.0.1"</code></td><td>Bind address</td></tr>
<tr><td><code>port</code></td><td>integer</td><td><code>8080</code></td><td>Listen port</td></tr>
<tr><td><code>enable_channels</code></td><td>bool</td><td><code>true</code></td><td>Enable channel adapters</td></tr>
<tr><td><code>enable_cron</code></td><td>bool</td><td><code>true</code></td><td>Enable cron scheduler</td></tr>
<tr><td><code>enable_heartbeat</code></td><td>bool</td><td><code>true</code></td><td>Enable heartbeat</td></tr>
<tr><td><code>enable_api</code></td><td>bool</td><td><code>true</code></td><td>Enable API server</td></tr>
<tr><td><code>enable_mcp</code></td><td>bool</td><td><code>true</code></td><td>Enable MCP server</td></tr>
<tr><td><code>mcp_port</code></td><td>integer</td><td><code>3002</code></td><td>MCP server port</td></tr>
</table>

<h2>[channels.telegram]</h2>
<table>
<tr><th>Field</th><th>Type</th><th>Default</th><th>Description</th></tr>
<tr><td><code>api_id</code></td><td>integer</td><td>—</td><td>Telegram API ID</td></tr>
<tr><td><code>api_hash</code></td><td>string</td><td>—</td><td>Telegram API hash</td></tr>
<tr><td><code>phone</code></td><td>string</td><td>—</td><td>Phone number</td></tr>
</table>

<h2>[channels.discord]</h2>
<table>
<tr><th>Field</th><th>Type</th><th>Default</th><th>Description</th></tr>
<tr><td><code>token</code></td><td>string</td><td>—</td><td>Discord bot token</td></tr>
</table>

<h2>[channels.slack]</h2>
<table>
<tr><th>Field</th><th>Type</th><th>Default</th><th>Description</th></tr>
<tr><td><code>bot_token</code></td><td>string</td><td>—</td><td>Slack bot token (xoxb-...)</td></tr>
<tr><td><code>app_token</code></td><td>string</td><td>—</td><td>Slack app token (xapp-...)</td></tr>
</table>

<h2>[channels.email]</h2>
<table>
<tr><th>Field</th><th>Type</th><th>Default</th><th>Description</th></tr>
<tr><td><code>smtp_host</code></td><td>string</td><td>—</td><td>SMTP server host</td></tr>
<tr><td><code>imap_host</code></td><td>string</td><td>—</td><td>IMAP server host</td></tr>
<tr><td><code>username</code></td><td>string</td><td>—</td><td>Email username</td></tr>
<tr><td><code>password</code></td><td>string</td><td>—</td><td>Email password</td></tr>
</table>

<h2>Model String Format</h2>
<p>Format: <code>provider/model-name</code></p>
<pre><code>ollama/llama3.2
anthropic/claude-sonnet-4-20250514
openai/gpt-4o
openrouter/anthropic/claude-3.5-sonnet
google/gemini-2.0-flash
groq/llama-3.3-70b-versatile
mistral/mistral-large-latest
together/meta-llama/Meta-Llama-3.1-405B-Instruct-Turbo
fireworks/accounts/fireworks/models/llama-v3p1-405b-instruct
azure/gpt-4o
bedrock/anthropic.claude-3-5-sonnet-20241022-v2:0</code></pre>

<h2>Environment Variables</h2>
<table>
<tr><th>Variable</th><th>Required For</th></tr>
<tr><td><code>ANTHROPIC_API_KEY</code></td><td>Anthropic Claude</td></tr>
<tr><td><code>OPENAI_API_KEY</code></td><td>OpenAI GPT</td></tr>
<tr><td><code>OPENROUTER_API_KEY</code></td><td>OpenRouter</td></tr>
<tr><td><code>GOOGLE_API_KEY</code></td><td>Google Gemini</td></tr>
<tr><td><code>GROQ_API_KEY</code></td><td>Groq</td></tr>
<tr><td><code>MISTRAL_API_KEY</code></td><td>Mistral AI</td></tr>
<tr><td><code>TOGETHER_API_KEY</code></td><td>Together AI</td></tr>
<tr><td><code>FIREWORKS_API_KEY</code></td><td>Fireworks AI</td></tr>
<tr><td><code>AZURE_OPENAI_API_KEY</code></td><td>Azure OpenAI</td></tr>
<tr><td><code>AZURE_OPENAI_ENDPOINT</code></td><td>Azure resource URL</td></tr>
<tr><td><code>AZURE_OPENAI_DEPLOYMENT</code></td><td>Azure deployment</td></tr>
<tr><td><code>AWS_ACCESS_KEY_ID</code></td><td>AWS Bedrock</td></tr>
<tr><td><code>AWS_SECRET_ACCESS_KEY</code></td><td>AWS Bedrock</td></tr>
<tr><td><code>AWS_REGION</code></td><td>AWS Bedrock (default: us-east-1)</td></tr>
<tr><td><code>OLLAMA_HOST</code></td><td>Custom Ollama server</td></tr>
<tr><td><code>TWILIO_ACCOUNT_SID</code></td><td>Twilio voice calls</td></tr>
<tr><td><code>TWILIO_AUTH_TOKEN</code></td><td>Twilio voice calls</td></tr>
<tr><td><code>TWILIO_PHONE_NUMBER</code></td><td>Twilio caller ID</td></tr>
<tr><td><code>ELEVENLABS_API_KEY</code></td><td>ElevenLabs TTS</td></tr>
<tr><td><code>ZEUS_API_TOKEN</code></td><td>API auth token</td></tr>
<tr><td><code>ZEUS_API_KEYS</code></td><td>API key list (comma-separated)</td></tr>
</table>"#.to_string();

    Html(page("Configuration Guide", &body))
}

// ============================================================================
// GET /docs/getting-started — Getting Started Guide
// ============================================================================

pub async fn docs_getting_started(_state: State<SharedState>) -> Html<String> {
    let body = r#"<h1>Getting Started</h1>
<p class="subtitle">Install Zeus and start chatting in under 5 minutes.</p>

<h2>1. Install</h2>
<pre><code># Clone and build
git clone https://github.com/your-org/zeus.git
cd zeus
cargo build --release

# Install binary
sudo cp target/release/zeus /usr/local/bin/</code></pre>

<h2>2. Run the Setup Wizard</h2>
<pre><code>zeus onboard</code></pre>
<p>The wizard detects your environment and guides you through:</p>
<ul>
<li><strong>OpenClaw migration</strong> — imports API keys from existing <code>.openclaw/config.toml</code></li>
<li><strong>Environment detection</strong> — finds API keys already in your shell (ANTHROPIC_API_KEY, OPENAI_API_KEY, etc.)</li>
<li><strong>Ollama auto-detect</strong> — discovers local Ollama with available models</li>
<li><strong>Provider selection</strong> — choose from 11 providers: Anthropic, OpenAI, Ollama, OpenRouter, Google, Groq, Mistral, Together, Fireworks, Azure, Bedrock</li>
<li><strong>Model configuration</strong> — writes <code>~/.zeus/config.toml</code></li>
</ul>

<h2>3. Start Chatting</h2>
<pre><code># Interactive TUI
zeus

# Single message
zeus chat "Hello, Zeus!"

# With streaming
zeus chat -s "Explain Rust ownership"</code></pre>

<h2>4. Launch the Gateway</h2>
<pre><code># Full gateway: API + channels + heartbeat + cron
zeus gateway

# Minimal gateway (API only)
zeus gateway --no-channels --no-cron</code></pre>
<p>The gateway starts on <code>http://127.0.0.1:8080</code>. Try:</p>
<pre><code>curl http://127.0.0.1:8080/health
curl http://127.0.0.1:8080/v1/status</code></pre>

<h2>5. Connect Channels (Optional)</h2>
<p>Add messaging adapters to <code>~/.zeus/config.toml</code>:</p>
<pre><code>[channels.telegram]
api_id = 12345
api_hash = "your_api_hash"
phone = "+1234567890"

[channels.discord]
token = "your_bot_token"

[channels.slack]
bot_token = "xoxb-..."
app_token = "xapp-..."</code></pre>

<h2>6. Install as Daemon (macOS)</h2>
<pre><code># Install and start launchd service
zeus daemon install
zeus daemon start

# Check status
zeus daemon status</code></pre>

<h2>7. Verify Installation</h2>
<pre><code># Run diagnostics
zeus doctor</code></pre>
<p>This checks: config validity, workspace paths, API credentials, Ollama connectivity, and channel configs.</p>

<h2>CLI Commands Reference</h2>
<table>
<tr><th>Command</th><th>Description</th></tr>
<tr><td><code>zeus</code></td><td>Launch TUI (default)</td></tr>
<tr><td><code>zeus tui</code></td><td>Launch TUI explicitly</td></tr>
<tr><td><code>zeus serve</code></td><td>Run HTTP API server</td></tr>
<tr><td><code>zeus serve -p 3000</code></td><td>Custom port</td></tr>
<tr><td><code>zeus gateway</code></td><td>Run unified daemon</td></tr>
<tr><td><code>zeus chat "msg"</code></td><td>Single message mode</td></tr>
<tr><td><code>zeus chat -s "msg"</code></td><td>Streaming mode</td></tr>
<tr><td><code>zeus tool list_dir '{"path":"."}'</code></td><td>Execute a tool directly</td></tr>
<tr><td><code>zeus config</code></td><td>Show configuration</td></tr>
<tr><td><code>zeus config --show-secrets</code></td><td>Show with API keys</td></tr>
<tr><td><code>zeus memory show</code></td><td>Show workspace context</td></tr>
<tr><td><code>zeus memory remember "fact"</code></td><td>Add to memory</td></tr>
<tr><td><code>zeus session list</code></td><td>List sessions</td></tr>
<tr><td><code>zeus doctor</code></td><td>Run diagnostics</td></tr>
<tr><td><code>zeus onboard</code></td><td>Setup wizard</td></tr>
<tr><td><code>zeus daemon install</code></td><td>Install launchd service</td></tr>
<tr><td><code>zeus completion bash</code></td><td>Generate bash completions</td></tr>
</table>

<h2>Architecture Overview</h2>
<p>Zeus is a Cargo workspace with 21 crates:</p>
<pre><code>crates/
├── zeus-core/        Types, errors, config
├── zeus-llm/         Unified LLM (11 providers)
├── zeus-memory/      Workspace file-based memory
├── zeus-session/     JSONL session storage
├── zeus-agent/       Agent loop + 14 core tools
├── zeus-tui/         Ratatui TUI (10 screens)
├── zeus-api/         REST API gateway + WebSocket
├── zeus-mcp/         Model Context Protocol
├── zeus-nous/        Cognitive engine
├── zeus-prometheus/  Orchestration + cron
├── zeus-channels/    8 messaging adapters
├── zeus-hermes/      Notification router
├── zeus-mnemosyne/   SQLite FTS5 + vector memory
├── zeus-athena/      Documentation engine
├── zeus-aegis/       Security sandboxing
├── zeus-talos/       macOS automation (193 tools)
├── zeus-browser/     Chrome CDP automation
├── zeus-skills/      SKILL.md parser
├── zeus-voice/       Voice calls + STT/TTS
├── zeus-ffi/         UniFFI Swift bindings
└── zeus-agora/       Agent skill marketplace</code></pre>"#.to_string();

    Html(page("Getting Started", &body))
}
