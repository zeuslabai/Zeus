//! Project management handlers.

use axum::{
    Json,
    extract::Path,
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::info;

use std::path::PathBuf;

#[derive(Debug, Deserialize)]
pub struct CreateProjectRequest {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub budget: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateProjectRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub status: Option<String>,
    pub budget: Option<f64>,
    /// Project lead agent name. (#249)
    #[serde(default)]
    pub lead: Option<String>,
    /// Project progress 0–100. (#249)
    #[serde(default)]
    pub progress: Option<u8>,
}

#[derive(Debug, Deserialize)]
pub struct AssignAgentsRequest {
    pub agents: Vec<String>,
}

fn projects_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".zeus")
        .join("projects")
}

/// GET /v1/projects — List all projects
pub async fn list_projects() -> Json<Value> {
    let dir = projects_dir();
    let mut projects = Vec::new();

    if dir.exists()
        && let Ok(mut rd) = tokio::fs::read_dir(&dir).await
    {
        while let Ok(Some(entry)) = rd.next_entry().await {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json")
                && let Ok(content) = tokio::fs::read_to_string(&path).await
                && let Ok(mut project) = serde_json::from_str::<Value>(&content)
            {
                // Ensure TUI Advanced panel fields exist with defaults (#249).
                if let Some(obj) = project.as_object_mut() {
                    obj.entry("lead").or_insert(json!(""));
                    obj.entry("progress").or_insert(json!(0u8));
                }
                projects.push(project);
            }
        }
    }

    Json(json!({ "projects": projects }))
}

/// POST /v1/projects — Create project
pub async fn create_project(
    Json(req): Json<CreateProjectRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let dir = projects_dir();
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    let project = json!({
        "id": id,
        "name": req.name,
        "description": req.description.unwrap_or_default(),
        "status": "active",
        "missions": 0,
        "budget": req.budget.unwrap_or(0.0),
        "spent": 0.0,
        "agents": [],
        "created": now,
        // TUI Advanced panel fields (#249).
        "lead": "",
        "progress": 0u8
    });

    let path = dir.join(format!("{}.json", id));
    tokio::fs::write(
        &path,
        serde_json::to_string_pretty(&project).unwrap_or_default(),
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    info!("Created project: {} ({})", req.name, id);

    Ok(Json(project))
}

/// GET /v1/projects/:id — Project detail
pub async fn get_project(Path(id): Path<String>) -> Result<Json<Value>, (StatusCode, String)> {
    let path = projects_dir().join(format!("{}.json", id));

    if !path.exists() {
        return Err((StatusCode::NOT_FOUND, format!("Project not found: {}", id)));
    }

    let content = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut project: Value = serde_json::from_str(&content)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Ensure TUI Advanced panel fields exist with defaults (#249).
    if let Some(obj) = project.as_object_mut() {
        obj.entry("lead").or_insert(json!(""));
        obj.entry("progress").or_insert(json!(0u8));
    }

    Ok(Json(project))
}

/// PUT /v1/projects/:id — Update project
pub async fn update_project(
    Path(id): Path<String>,
    Json(req): Json<UpdateProjectRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let path = projects_dir().join(format!("{}.json", id));

    if !path.exists() {
        return Err((StatusCode::NOT_FOUND, format!("Project not found: {}", id)));
    }

    let content = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut project: Value = serde_json::from_str(&content)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if let Some(name) = req.name {
        project["name"] = Value::String(name);
    }
    if let Some(description) = req.description {
        project["description"] = Value::String(description);
    }
    if let Some(status) = req.status {
        project["status"] = Value::String(status);
    }
    if let Some(budget) = req.budget {
        project["budget"] = json!(budget);
    }
    if let Some(lead) = req.lead {
        project["lead"] = Value::String(lead);
    }
    if let Some(progress) = req.progress {
        project["progress"] = json!(progress);
    }

    tokio::fs::write(
        &path,
        serde_json::to_string_pretty(&project).unwrap_or_default(),
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(json!({
        "success": true,
        "id": id,
        "message": "Project updated"
    })))
}

/// PUT /v1/projects/:id/agents — Assign agents to project
pub async fn assign_project_agents(
    Path(id): Path<String>,
    Json(req): Json<AssignAgentsRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let path = projects_dir().join(format!("{}.json", id));

    if !path.exists() {
        return Err((StatusCode::NOT_FOUND, format!("Project not found: {}", id)));
    }

    let content = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut project: Value = serde_json::from_str(&content)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    project["agents"] = json!(req.agents);

    tokio::fs::write(
        &path,
        serde_json::to_string_pretty(&project).unwrap_or_default(),
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(json!({
        "success": true,
        "id": id,
        "agents": req.agents,
        "message": "Agents assigned to project"
    })))
}

/// DELETE /v1/projects/:id — Delete project
pub async fn delete_project(Path(id): Path<String>) -> Result<Json<Value>, (StatusCode, String)> {
    let path = projects_dir().join(format!("{}.json", id));

    if !path.exists() {
        return Err((StatusCode::NOT_FOUND, format!("Project not found: {}", id)));
    }

    tokio::fs::remove_file(&path)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(json!({
        "success": true,
        "id": id,
        "message": "Project deleted"
    })))
}
