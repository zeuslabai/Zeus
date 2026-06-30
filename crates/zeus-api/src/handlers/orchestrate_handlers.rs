//! Orchestrate + team handlers

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde_json::{Value, json};
use zeus_llm::LlmClient;
use crate::SharedState;
use super::{parse_goal_analysis, generate_team_recommendation};
use super::prometheus_handlers::{ExecutionMode, execute_plan_steps};

pub async fn orchestrate_start(
    State(state): State<SharedState>,
    Json(body): Json<Value>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, String)> {
    let goal = body
        .get("goal")
        .and_then(|v| v.as_str())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing 'goal' field".to_string()))?;

    let state_guard = state.read().await;
    let session = state_guard.orchestration().create(goal).await;
    let session_id = session.id.clone();

    // Transition to Analyzing
    state_guard
        .orchestration()
        .update(&session_id, |s| s.start_analysis())
        .await;

    // Run LLM analysis to determine scope and generate questions
    let llm = LlmClient::from_config(&state_guard.config).map_err(|e| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            format!("LLM required for orchestration: {e}"),
        )
    })?;

    let analysis_prompt = format!(
        "You are a project orchestration engine. Analyze this goal and respond with JSON only.\n\n\
         Goal: \"{goal}\"\n\n\
         Respond with this exact JSON structure:\n\
         {{\n  \
           \"summary\": \"one-sentence summary of the project\",\n  \
           \"scope\": \"frontend|backend|fullstack|data|devops|other\",\n  \
           \"complexity\": \"low|medium|high|very_high\",\n  \
           \"suggested_approach\": \"brief technical approach\",\n  \
           \"needs_clarification\": true/false,\n  \
           \"clarification_questions\": [\n    \
             {{\"question\": \"...\", \"purpose\": \"why this matters\"}}\n  \
           ]\n\
         }}\n\n\
         Generate 0-5 clarification questions. Only ask if genuinely needed.\n\
         If the goal is clear enough, set needs_clarification to false and return empty questions array."
    );

    let messages = vec![zeus_core::Message::user(&analysis_prompt)];
    let llm_response = llm.complete(&messages, &[], None).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("LLM analysis failed: {e}"),
        )
    })?;

    // Parse LLM response into GoalAnalysis
    let analysis = parse_goal_analysis(&llm_response.content);

    let has_questions =
        analysis.needs_clarification && !analysis.clarification_questions.is_empty();

    // Complete analysis (transitions to Onboarding or stays for team recommendation)
    state_guard
        .orchestration()
        .update(&session_id, |s| s.complete_analysis(analysis.clone()))
        .await;

    // If no clarification needed, generate team recommendation immediately
    if !has_questions {
        let recommendation = generate_team_recommendation(&analysis, goal);
        state_guard
            .orchestration()
            .update(&session_id, |s| s.recommend_team(recommendation))
            .await;
    }

    let updated = state_guard.orchestration().get(&session_id).await;
    drop(state_guard);

    match updated {
        Some(session) => {
            let mut response = serde_json::to_value(&session).unwrap_or_default();
            if let Some(q) = session.current_question()
                && let Some(obj) = response.as_object_mut()
            {
                obj.insert(
                    "next_question".to_string(),
                    json!({
                        "question": q.question,
                        "purpose": q.purpose,
                    }),
                );
            }
            Ok((StatusCode::CREATED, Json(response)))
        }
        None => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Session lost after creation".to_string(),
        )),
    }
}

pub async fn orchestrate_respond(
    State(state): State<SharedState>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let session_id = body
        .get("session_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing 'session_id'".to_string()))?;
    let answer = body
        .get("answer")
        .and_then(|v| v.as_str())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing 'answer'".to_string()))?;

    let state_guard = state.read().await;

    // Record the answer (discard bool return to satisfy FnOnce -> () signature)
    state_guard
        .orchestration()
        .update(session_id, |s| {
            let _ = s.answer_question(answer.to_string());
        })
        .await
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Session not found".to_string()))?;

    // Check if there are more questions by inspecting current state
    let session = state_guard
        .orchestration()
        .get(session_id)
        .await
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Session not found".to_string()))?;
    let has_more = session.current_question().is_some();

    // If onboarding is complete, generate team recommendation
    if !has_more {
        let analysis = session.analysis.as_ref().ok_or_else(|| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "No analysis found".to_string(),
            )
        })?;

        let recommendation = generate_team_recommendation(analysis, &session.goal);
        state_guard
            .orchestration()
            .update(session_id, |s| s.recommend_team(recommendation))
            .await;
    }

    let session = state_guard
        .orchestration()
        .get(session_id)
        .await
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Session not found".to_string()))?;

    drop(state_guard);

    let mut response = serde_json::to_value(&session).unwrap_or_default();
    if let Some(q) = session.current_question()
        && let Some(obj) = response.as_object_mut()
    {
        obj.insert(
            "next_question".to_string(),
            json!({
                "question": q.question,
                "purpose": q.purpose,
            }),
        );
    }

    Ok(Json(response))
}

pub async fn orchestrate_status(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state_guard = state.read().await;
    let session = state_guard
        .orchestration()
        .get(&id)
        .await
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Session not found".to_string()))?;
    drop(state_guard);

    Ok(Json(serde_json::to_value(&session).unwrap_or_default()))
}

pub async fn orchestrate_confirm(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, String)> {
    use zeus_prometheus::orchestrate::OrchestrationAutonomy;

    let autonomy_str = body
        .get("autonomy")
        .and_then(|v| v.as_str())
        .unwrap_or("supervised");

    let autonomy = match autonomy_str {
        "full" => OrchestrationAutonomy::Full,
        _ => OrchestrationAutonomy::Supervised,
    };

    let state_guard = state.read().await;

    // Get session to extract goal for planning
    let session = state_guard
        .orchestration()
        .get(&id)
        .await
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Session not found".to_string()))?;

    let goal = session.goal.clone();

    // Build a plan using the Planner
    let llm = LlmClient::from_config(&state_guard.config).map_err(|e| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            format!("LLM required: {e}"),
        )
    })?;

    let tool_schemas = state_guard.tools.schemas();
    let planner = zeus_prometheus::planner::Planner::new();
    let plan = planner
        .create_plan(&goal, &llm, &tool_schemas)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Planning failed: {e}"),
            )
        })?;

    let dag = state_guard
        .strategic_planner()
        .analyze(&plan)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("DAG analysis failed: {e}"),
            )
        })?;

    let plan_id = format!("orch-plan-{}", uuid::Uuid::new_v4());
    let steps_total = dag.nodes.len();

    // Confirm and start execution
    state_guard
        .orchestration()
        .update(&id, |s| {
            s.confirm_and_execute(autonomy, plan_id.clone(), steps_total);
        })
        .await
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Session not found".to_string()))?;

    let broadcast = state_guard.plan_broadcast.clone();
    let orch_broadcast = state_guard.orchestration_broadcast.clone();
    let orch_mgr = state_guard.orchestration().clone();
    let orch_id = id.clone();
    let exec_state = state.clone();
    let plan_id_for_spawn = plan_id.clone();
    let workspace_dir = state_guard.config.workspace.clone();
    drop(state_guard);

    // Spawn background execution
    tokio::spawn(async move {
        use crate::websocket::OrchestrationEvent;
        use zeus_prometheus::packaging::{PackageEntry, PackagingConfig, package_deliverable};

        let steps_total_exec = dag.nodes.len();

        // Emit phase change: executing
        orch_broadcast.send(OrchestrationEvent::PhaseChanged {
            session_id: orch_id.clone(),
            phase: "executing".to_string(),
            data: serde_json::json!({ "plan_id": plan_id_for_spawn, "steps_total": steps_total_exec }),
        });

        let mode = ExecutionMode::Agent(exec_state);
        let exec_result =
            execute_plan_steps(dag, &plan_id_for_spawn, &goal, mode, &broadcast).await;

        // Transition to packaging
        orch_mgr.update(&orch_id, |s| s.start_packaging()).await;
        orch_broadcast.send(OrchestrationEvent::PhaseChanged {
            session_id: orch_id.clone(),
            phase: "packaging".to_string(),
            data: serde_json::json!({}),
        });

        // Collect artifacts from the session
        let artifacts = orch_mgr
            .get(&orch_id)
            .await
            .map(|s| s.artifacts.clone())
            .unwrap_or_default();

        // Build package entries from artifacts
        let mut entries = Vec::new();
        for artifact in &artifacts {
            let path = std::path::Path::new(&artifact.path);
            if path.exists()
                && let Ok(content) = std::fs::read(path)
            {
                entries.push(PackageEntry {
                    archive_path: artifact.name.clone(),
                    content,
                });
            }
        }

        // Also collect any files created in the workspace during this session
        let session_workspace = workspace_dir.join("orchestrations").join(&orch_id);
        if session_workspace.exists()
            && let Ok(dir_entries) =
                zeus_prometheus::packaging::collect_directory(&session_workspace)
        {
            entries.extend(dir_entries);
        }

        // Package into ZIP (with execution transcript from plan steps)
        let config = PackagingConfig::default();
        let zip_result = package_deliverable(
            &orch_id,
            &goal,
            &entries,
            Some(&exec_result.transcript),
            &config,
        );

        let (artifact_path, summary) = match zip_result {
            Ok(result) => {
                let path_str = result.zip_path.display().to_string();
                orch_broadcast.send(OrchestrationEvent::ArtifactCreated {
                    session_id: orch_id.clone(),
                    artifact_name: format!("{}.zip", orch_id),
                    artifact_path: path_str.clone(),
                });
                let summary = format!(
                    "Orchestration complete for: {}. Packaged {} files ({} bytes) into {}",
                    goal, result.file_count, result.size_bytes, path_str
                );
                (path_str, summary)
            }
            Err(e) => {
                tracing::warn!("Packaging failed: {e}");
                let summary = format!(
                    "Orchestration complete for: {} (packaging failed: {e})",
                    goal
                );
                (String::new(), summary)
            }
        };

        // Mark delivered
        let final_summary = summary.clone();
        let final_path = artifact_path.clone();
        orch_mgr
            .update(&orch_id, |s| {
                s.deliver(final_path, final_summary);
            })
            .await;

        // Emit completion event
        orch_broadcast.send(OrchestrationEvent::Complete {
            session_id: orch_id,
            status: "delivered".to_string(),
            summary,
            artifact_path,
            duration_ms: exec_result.duration_ms,
        });
    });

    Ok((
        StatusCode::ACCEPTED,
        Json(json!({
            "session_id": id,
            "plan_id": plan_id,
            "steps_total": steps_total,
            "autonomy": autonomy_str,
            "status": "executing",
            "watch_via": format!("WebSocket PrometheusWatch with plan_id: {}", plan_id),
        })),
    ))
}

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
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("LLM failed: {e}"),
        )
    })?;

    let analysis = parse_goal_analysis(&resp.content);
    let recommendation = generate_team_recommendation(&analysis, goal);

    Ok(Json(
        serde_json::to_value(&recommendation).unwrap_or_default(),
    ))
}

