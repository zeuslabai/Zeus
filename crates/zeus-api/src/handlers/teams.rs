//! Teams API handlers

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde_json::{Value, json};
use zeus_llm::LlmClient;

use crate::SharedState;

/// GET /v1/teams — List all teams
pub async fn list_teams(State(state): State<SharedState>) -> Json<Value> {
    let state_guard = state.read().await;
    let teams = state_guard.orchestra.list_teams().await;
    let team_values: Vec<Value> = teams
        .iter()
        .map(|t| {
            json!({
                "id": t.id,
                "name": t.name,
                "agent_ids": t.agent_ids,
                "supervisor_id": t.supervisor_id,
                "agent_count": t.agent_count(),
                "created_at": t.created_at.to_rfc3339(),
            })
        })
        .collect();
    let total = team_values.len();
    Json(json!({
        "teams": team_values,
        "total": total,
    }))
}

/// POST /v1/teams — Create a team
pub async fn create_team(
    State(state): State<SharedState>,
    Json(body): Json<Value>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, String)> {
    let name = body
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing 'name' field".to_string()))?;

    let mut team = zeus_orchestra::AgentTeam::new(name);

    if let Some(supervisor) = body.get("supervisor_id").and_then(|v| v.as_str()) {
        if !supervisor.is_empty() {
            team = team.with_supervisor(supervisor.to_string());
        }
    }
    if let Some(agents) = body.get("agent_ids").and_then(|v| v.as_array()) {
        let ids: Vec<String> = agents
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
        team = team.with_agents(ids);
    }
    if let Some(policy_val) = body.get("policy") {
        let mut policy = zeus_orchestra::TeamPolicy::default();
        if let Some(v) = policy_val.get("max_depth").and_then(|v| v.as_u64()) {
            policy.max_depth = v as u32;
        }
        if let Some(v) = policy_val.get("budget_tokens").and_then(|v| v.as_u64()) {
            policy.budget_tokens = v;
        }
        if let Some(v) = policy_val.get("timeout_seconds").and_then(|v| v.as_u64()) {
            policy.timeout_seconds = v;
        }
        if let Some(v) = policy_val.get("loop_detection").and_then(|v| v.as_bool()) {
            policy.loop_detection = v;
        }
        if let Some(v) = policy_val.get("quality_threshold").and_then(|v| v.as_f64()) {
            policy.quality_threshold = v;
        }
        if let Some(v) = policy_val.get("require_verification").and_then(|v| v.as_bool()) {
            policy.require_verification = v;
        }
        team = team.with_policy(policy);
    }

    let state_guard = state.read().await;
    let created = state_guard
        .orchestra
        .create_team(team)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")))?;

    let mut val = serde_json::to_value(&created)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("serialize error: {e}")))?;
    if let Some(obj) = val.as_object_mut() {
        obj.insert("status".to_string(), serde_json::Value::String("created".to_string()));
    }

    Ok((
        StatusCode::CREATED,
        Json(val),
    ))
}

/// GET /v1/teams/:id — Get team details
pub async fn get_team(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state_guard = state.read().await;
    let team = state_guard
        .orchestra
        .get_team(&id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, format!("{e}")))?;
    let val = serde_json::to_value(&team)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("serialize error: {e}")))?;
    Ok(Json(val))
}

/// PUT /v1/teams/:id — Update a team
pub async fn update_team(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state_guard = state.read().await;
    let mut team = state_guard
        .orchestra
        .get_team(&id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, format!("{e}")))?;

    if let Some(name) = body.get("name").and_then(|v| v.as_str()) {
        team.name = name.to_string();
    }
    if let Some(supervisor) = body.get("supervisor_id") {
        team.supervisor_id = supervisor.as_str().map(|s| s.to_string());
    }
    if let Some(agents) = body.get("agent_ids").and_then(|v| v.as_array()) {
        team.agent_ids = agents
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
    }

    state_guard
        .orchestra
        .update_team(team.clone())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")))?;

    let val = serde_json::to_value(&team)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("serialize error: {e}")))?;
    Ok(Json(val))
}

/// DELETE /v1/teams/:id — Delete a team
pub async fn delete_team(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state_guard = state.read().await;
    state_guard
        .orchestra
        .delete_team(&id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, format!("{e}")))?;
    Ok(Json(json!({ "deleted": true, "id": id })))
}

/// POST /v1/teams/recommend — Recommend a team composition based on a goal
pub async fn team_recommend(
    State(state): State<SharedState>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let goal = body
        .get("goal")
        .and_then(|v| v.as_str())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing 'goal' field".to_string()))?;

    let state_guard = state.read().await;
    let llm = LlmClient::from_config(&state_guard.config).map_err(|e| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            format!("LLM required: {e}"),
        )
    })?;
    drop(state_guard);

    let analysis_prompt = format!(
        "Analyze this goal and respond with JSON only.\n\nGoal: \"{goal}\"\n\n\
         {{\"summary\": \"...\", \"scope\": \"...\", \"complexity\": \"low|medium|high|very_high\", \
         \"suggested_approach\": \"...\", \"needs_clarification\": false, \"clarification_questions\": []}}"
    );

    let messages = vec![zeus_core::Message::user(&analysis_prompt)];
    let resp = llm.complete(&messages, &[], None).await.map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, format!("LLM failed: {e}"))
    })?;

    let analysis = parse_goal_analysis(&resp.content);
    let recommendation = generate_team_recommendation(&analysis, goal);

    Ok(Json(serde_json::to_value(&recommendation).unwrap_or_default()))
}

/// Parse LLM response into GoalAnalysis struct.
pub(crate) fn parse_goal_analysis(content: &str) -> zeus_prometheus::orchestrate::GoalAnalysis {
    use zeus_prometheus::orchestrate::{GoalAnalysis, OnboardingQuestion};

    // Strip markdown code blocks if present
    let json_str = content
        .trim()
        .strip_prefix("```json")
        .or_else(|| content.trim().strip_prefix("```"))
        .unwrap_or(content.trim())
        .trim()
        .strip_suffix("```")
        .unwrap_or(content.trim())
        .trim();

    if let Ok(parsed) = serde_json::from_str::<Value>(json_str) {
        let questions: Vec<OnboardingQuestion> = parsed
            .get("clarification_questions")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|q| {
                        Some(OnboardingQuestion {
                            question: q.get("question")?.as_str()?.to_string(),
                            answer: None,
                            purpose: q
                                .get("purpose")
                                .and_then(|v| v.as_str())
                                .unwrap_or("clarification")
                                .to_string(),
                        })
                    })
                    .take(5)
                    .collect()
            })
            .unwrap_or_default();

        let needs_clarification = parsed
            .get("needs_clarification")
            .and_then(|v| v.as_bool())
            .unwrap_or(!questions.is_empty());

        GoalAnalysis {
            summary: parsed
                .get("summary")
                .and_then(|v| v.as_str())
                .unwrap_or("Project analysis")
                .to_string(),
            scope: parsed
                .get("scope")
                .and_then(|v| v.as_str())
                .unwrap_or("general")
                .to_string(),
            complexity: parsed
                .get("complexity")
                .and_then(|v| v.as_str())
                .unwrap_or("medium")
                .to_string(),
            suggested_approach: parsed
                .get("suggested_approach")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            needs_clarification,
            clarification_questions: questions,
        }
    } else {
        // Fallback: treat response as summary, skip onboarding
        GoalAnalysis {
            summary: content.chars().take(200).collect(),
            scope: "general".to_string(),
            complexity: "medium".to_string(),
            suggested_approach: String::new(),
            needs_clarification: false,
            clarification_questions: vec![],
        }
    }
}

/// Generate a team recommendation based on goal analysis.
pub(crate) fn generate_team_recommendation(
    analysis: &zeus_prometheus::orchestrate::GoalAnalysis,
    _goal: &str,
) -> zeus_prometheus::orchestrate::TeamRecommendation {
    use zeus_prometheus::orchestrate::{AgentSuggestion, TeamRecommendation};

    let (coordinators, workers, estimated_steps) = match analysis.complexity.as_str() {
        "low" => (
            vec![AgentSuggestion {
                role: "project-lead".to_string(),
                capabilities: vec!["planning".to_string(), "code-review".to_string()],
                model_tier: "sonnet".to_string(),
            }],
            vec![AgentSuggestion {
                role: "developer".to_string(),
                capabilities: vec![analysis.scope.clone(), "implementation".to_string()],
                model_tier: "sonnet".to_string(),
            }],
            3,
        ),
        "high" | "very_high" => (
            vec![AgentSuggestion {
                role: "project-lead".to_string(),
                capabilities: vec![
                    "planning".to_string(),
                    "architecture".to_string(),
                    "code-review".to_string(),
                ],
                model_tier: "opus".to_string(),
            }],
            vec![
                AgentSuggestion {
                    role: "senior-developer".to_string(),
                    capabilities: vec![analysis.scope.clone(), "implementation".to_string()],
                    model_tier: "sonnet".to_string(),
                },
                AgentSuggestion {
                    role: "developer".to_string(),
                    capabilities: vec!["implementation".to_string(), "testing".to_string()],
                    model_tier: "sonnet".to_string(),
                },
                AgentSuggestion {
                    role: "qa-engineer".to_string(),
                    capabilities: vec!["testing".to_string(), "validation".to_string()],
                    model_tier: "haiku".to_string(),
                },
            ],
            10,
        ),
        _ => (
            // medium complexity
            vec![AgentSuggestion {
                role: "project-lead".to_string(),
                capabilities: vec!["planning".to_string(), "code-review".to_string()],
                model_tier: "opus".to_string(),
            }],
            vec![
                AgentSuggestion {
                    role: "developer".to_string(),
                    capabilities: vec![analysis.scope.clone(), "implementation".to_string()],
                    model_tier: "sonnet".to_string(),
                },
                AgentSuggestion {
                    role: "tester".to_string(),
                    capabilities: vec!["testing".to_string(), "validation".to_string()],
                    model_tier: "haiku".to_string(),
                },
            ],
            6,
        ),
    };

    TeamRecommendation {
        team_name: format!("{}-team", analysis.scope),
        coordinators,
        workers,
        rationale: format!(
            "{} complexity {} project. {}",
            analysis.complexity, analysis.scope, analysis.suggested_approach
        ),
        estimated_complexity: analysis.complexity.clone(),
        estimated_steps,
    }
}
