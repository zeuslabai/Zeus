// Session detail, replay, branching, search, delete

use super::*;

pub async fn fetch_sessions() -> Result<SessionsResponse, String> {
    fetch_json("/v1/sessions").await
}

pub async fn fetch_session(id: &str) -> Result<SessionDetail, String> {
    fetch_json(&format!("/v1/sessions/{}", id)).await
}

pub async fn fetch_session_stats(id: &str) -> Result<SessionStatsDetail, String> {
    fetch_json(&format!("/v1/sessions/{}/stats", id)).await
}

pub async fn fetch_session_replay(id: &str) -> Result<Vec<ReplayTurn>, String> {
    let resp: ReplayResponse = fetch_json(&format!("/v1/sessions/{}/replay", id)).await?;
    Ok(resp.entries)
}

pub async fn fetch_replay_stats(id: &str) -> Result<ReplayStats, String> {
    fetch_json(&format!("/v1/sessions/{}/stats", id)).await
}

pub async fn delete_session(id: &str) -> Result<(), String> {
    delete_endpoint(&format!("/v1/sessions/{}", id)).await
}

pub async fn fetch_session_raw(id: &str) -> Result<SessionRawResponse, String> {
    fetch_json(&format!("/v1/sessions/{}/raw", id)).await
}

pub async fn fetch_session_audit(id: &str) -> Result<SessionAuditResponse, String> {
    fetch_json(&format!("/v1/sessions/{}/audit", id)).await
}

pub async fn fetch_session_tools(id: &str) -> Result<SessionToolsResponse, String> {
    fetch_json(&format!("/v1/sessions/{}/tools", id)).await
}

pub async fn fetch_branches(session_id: &str) -> Result<BranchesResponse, String> {
    fetch_json(&format!("/v1/sessions/{}/branches", session_id)).await
}

pub async fn create_branch(session_id: &str, turn_index: u32, label: &str) -> Result<MsgResponse, String> {
    post_json(
        &format!("/v1/sessions/{}/branch", session_id),
        &serde_json::json!({"branch_point": turn_index, "label": label}),
    ).await
}

pub async fn search_sessions(query: &str) -> Result<SessionsResponse, String> {
    fetch_json(&format!("/v1/sessions/search?q={}", query)).await
}

pub async fn fetch_session_replay_turn(id: &str, turn: u32) -> Result<serde_json::Value, String> {
    fetch_json(&format!("/v1/sessions/{}/replay/{}", id, turn)).await
}
