// Agent CRUD, spawn, status, fleet, discover, teams

use super::*;

pub async fn fetch_network_agents() -> Result<NetworkAgentsResponse, String> {
    fetch_json("/v1/network/agents").await
}

pub async fn fetch_agents() -> Result<NetworkAgentsResponse, String> {
    fetch_json("/v1/agents").await
}

pub async fn fetch_agent(id: &str) -> Result<NetworkAgent, String> {
    fetch_json(&format!("/v1/agents/{}", id)).await
}

pub async fn create_agent(req: &CreateAgentReq) -> Result<MsgResponse, String> {
    post_json("/v1/agents", req).await
}

pub async fn delete_agent(id: &str) -> Result<(), String> {
    delete_endpoint(&format!("/v1/agents/{}", id)).await
}

pub async fn update_agent(id: &str, req: &UpdateAgentReq) -> Result<MsgResponse, String> {
    put_json(&format!("/v1/agents/{}", id), req).await
}

pub async fn dispatch_mission(req: &DispatchMissionReq) -> Result<ChatResponse, String> {
    post_json("/v1/chat", req).await
}

pub async fn spawn_agent(req: &SpawnAgentReq) -> Result<MsgResponse, String> {
    post_json("/v1/agents/spawn", req).await
}

pub async fn fetch_agent_status(id: &str) -> Result<AgentStatusResponse, String> {
    fetch_json(&format!("/v1/agents/{}/status", id)).await
}

pub async fn agent_chat(id: &str, message: &str) -> Result<serde_json::Value, String> {
    post_json(&format!("/v1/agents/{}/chat", id), &serde_json::json!({ "message": message })).await
}

pub async fn agent_send(id: &str, message: &str) -> Result<MsgResponse, String> {
    post_json(&format!("/v1/agents/{}/send", id), &serde_json::json!({ "message": message })).await
}

pub async fn hire_agent(caller_id: &str, task: &str, skill_name: Option<&str>, max_credits: u64) -> Result<serde_json::Value, String> {
    post_json("/v1/agents/hire", &serde_json::json!({
        "caller_id": caller_id, "task": task, "skill_name": skill_name, "input": {}, "max_credits": max_credits,
    })).await
}

pub async fn run_agent_task(task: &str, context: Option<&str>, model: Option<&str>, wait: bool) -> Result<serde_json::Value, String> {
    post_json("/v1/agents/run-task", &serde_json::json!({
        "task": task, "context": context, "model": model, "wait": wait, "max_iterations": 10,
    })).await
}

// Teams

pub async fn fetch_teams() -> Result<TeamsResponse, String> {
    fetch_json("/v1/teams").await
}

pub async fn create_team(name: &str, description: &str, routing_strategy: &str) -> Result<MsgResponse, String> {
    let body = serde_json::json!({
        "name": name,
        "description": description,
        "routing_strategy": routing_strategy,
    });
    post_json("/v1/teams", &body).await
}

pub async fn create_agent_team(body: &serde_json::Value) -> Result<MsgResponse, String> {
    post_json("/v1/agents/team", body).await
}

pub async fn fetch_team(id: &str) -> Result<serde_json::Value, String> {
    fetch_json(&format!("/v1/teams/{}", id)).await
}

pub async fn update_team(id: &str, body: &serde_json::Value) -> Result<serde_json::Value, String> {
    put_json(&format!("/v1/teams/{}", id), body).await
}

pub async fn delete_team(id: &str) -> Result<MsgResponse, String> {
    delete_json(&format!("/v1/teams/{}", id)).await
}

pub async fn recommend_team(goal: &str) -> Result<TeamRecommendation, String> {
    post_json("/v1/teams/recommend", &serde_json::json!({ "goal": goal })).await
}

// Fleet & Discovery

pub async fn fetch_fleet_agents() -> Result<Vec<FleetAgent>, String> {
    fetch_json("/v1/fleet").await
}

pub async fn discover_agents(capability: Option<&str>, status: Option<&str>, q: Option<&str>) -> Result<AgentDiscoverResponse, String> {
    let mut params = vec![];
    if let Some(c) = capability { params.push(format!("capability={}", c)); }
    if let Some(s) = status { params.push(format!("status={}", s)); }
    if let Some(q) = q { params.push(format!("q={}", q)); }
    let url = if params.is_empty() {
        "/v1/agents/discover".to_string()
    } else {
        format!("/v1/agents/discover?{}", params.join("&"))
    };
    fetch_json(&url).await
}

// Predictive Spawning

pub async fn fetch_spawner_status() -> Result<SpawnerStatus, String> {
    fetch_json("/v1/spawner/status").await
}

pub async fn fetch_spawner_active() -> Result<Vec<ActiveSpawn>, String> {
    fetch_json("/v1/spawner/active").await
}

pub async fn fetch_spawner_history() -> Result<Vec<SpawnHistoryEntry>, String> {
    fetch_json("/v1/spawner/history").await
}

pub async fn spawner_analyze(task: &str, tools: Vec<String>) -> Result<SpawnAnalyzeResponse, String> {
    post_json::<serde_json::Value, SpawnAnalyzeResponse>(
        "/v1/spawner/analyze",
        &serde_json::json!({"task": task, "tools": tools}),
    ).await
}
