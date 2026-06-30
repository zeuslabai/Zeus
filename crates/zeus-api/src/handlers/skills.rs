//! Skill management handlers — install, search, update, delete, share.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::{debug, info};

use crate::SharedState;
use crate::url_validator;
use super::{marketplace_store, pantheon};

// ============================================================================
// Request/Response Types
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct InstallSkillRequest {
    pub url: Option<String>,
    pub content: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateSkillRequest {
    pub enabled: Option<bool>,
}

/// Search/filter query parameters for skills
#[derive(Debug, Deserialize)]
pub struct SkillSearchQuery {
    /// Text search across name, description, tags
    pub q: Option<String>,
    /// Filter by category
    pub category: Option<String>,
    /// Filter by enabled status
    pub enabled: Option<bool>,
}

/// Enriched skill detail for API responses
#[derive(Debug, Serialize, Clone)]
pub struct SkillDetail {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub description: String,
    pub version: String,
    pub author: Option<String>,
    pub enabled: bool,
    pub permissions: Vec<String>,
    pub category: String,
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub os: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_env: Option<String>,
    pub user_invocable: bool,
    pub disable_model_invocation: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requires: Option<SkillRequirementsResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub install_specs: Option<Vec<SkillInstallSpecResponse>>,
    pub tools_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command_dispatch: Option<SkillDispatchResponse>,
}

/// Requirements check response
#[derive(Debug, Serialize, Clone)]
pub struct SkillRequirementsResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bins: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<Vec<String>>,
    pub satisfied: bool,
    pub summary: String,
}

/// Install spec response
#[derive(Debug, Serialize, Clone)]
pub struct SkillInstallSpecResponse {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub formula: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub os: Option<Vec<String>>,
}

/// Command dispatch response
#[derive(Debug, Serialize, Clone)]
pub struct SkillDispatchResponse {
    pub kind: String,
    pub tool_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arg_mode: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ClawHubInstallRequest {
    pub name: String,
}

/// Query params for skill uninstall
#[derive(Debug, Deserialize)]
pub struct UninstallQuery {
    /// If true, preview what would be removed without actually removing
    #[serde(default)]
    pub dry_run: Option<bool>,
    /// If true, remove from registry but keep files on disk
    #[serde(default)]
    pub keep_files: Option<bool>,
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Build an enriched SkillDetail from a parsed zeus-skills Skill
fn build_skill_detail(skill: &zeus_skills::Skill, id: &str, enabled: bool) -> SkillDetail {
    let tags: Vec<String> = skill
        .frontmatter
        .get("tags")
        .map(|t| {
            t.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();

    let category = skill
        .frontmatter
        .get("category")
        .cloned()
        .unwrap_or_else(|| infer_skill_category(&skill.name, &skill.description, &tags));

    let requires = skill.metadata.as_ref().and_then(|m| {
        m.requires.as_ref().map(|r| {
            let gating = zeus_skills::check_requirements(m);
            SkillRequirementsResponse {
                bins: r.bins.clone(),
                env: r.env.clone(),
                config: r.config.clone(),
                satisfied: gating.eligible,
                summary: gating.summary,
            }
        })
    });

    let install_specs = skill.metadata.as_ref().and_then(|m| {
        m.install.as_ref().map(|specs| {
            specs
                .iter()
                .map(|s| SkillInstallSpecResponse {
                    kind: s.kind.clone(),
                    label: s.label.clone(),
                    formula: s.formula.clone(),
                    package: s.package.clone(),
                    url: s.url.clone(),
                    os: s.os.clone(),
                })
                .collect()
        })
    });

    let dispatch = skill
        .command_dispatch
        .as_ref()
        .map(|d| SkillDispatchResponse {
            kind: d.kind.clone(),
            tool_name: d.tool_name.clone(),
            arg_mode: d.arg_mode.clone(),
        });

    SkillDetail {
        id: id.to_string(),
        name: skill.name.clone(),
        slug: zeus_skills::slugify(&skill.name),
        description: skill.description.clone(),
        version: skill.version.clone(),
        author: skill.author.clone(),
        enabled,
        permissions: skill.permissions.clone(),
        category,
        tags,
        emoji: skill.metadata.as_ref().and_then(|m| m.emoji.clone()),
        homepage: skill.metadata.as_ref().and_then(|m| m.homepage.clone()),
        os: skill.metadata.as_ref().and_then(|m| m.os.clone()),
        primary_env: skill.metadata.as_ref().and_then(|m| m.primary_env.clone()),
        user_invocable: skill.invocation.user_invocable,
        disable_model_invocation: skill.invocation.disable_model_invocation,
        requires,
        install_specs,
        tools_count: skill.tools.len(),
        command_dispatch: dispatch,
    }
}

/// Infer a skill category from its name, description, and tags.
/// Frontmatter `category` field overrides this inference.
fn infer_skill_category(name: &str, description: &str, tags: &[String]) -> String {
    let haystack = format!(
        "{} {} {}",
        name.to_lowercase(),
        description.to_lowercase(),
        tags.iter()
            .map(|t| t.to_lowercase())
            .collect::<Vec<_>>()
            .join(" ")
    );

    let categories = [
        (
            "development",
            &[
                "git",
                "code",
                "build",
                "test",
                "lint",
                "debug",
                "compile",
                "deploy",
                "ci",
                "cd",
                "docker",
                "npm",
                "cargo",
                "rust",
                "python",
                "javascript",
                "typescript",
            ][..],
        ),
        (
            "messaging",
            &[
                "email",
                "mail",
                "chat",
                "message",
                "slack",
                "discord",
                "telegram",
                "sms",
                "notify",
                "notification",
            ],
        ),
        (
            "infrastructure",
            &[
                "server",
                "ssh",
                "dns",
                "network",
                "cloud",
                "aws",
                "kubernetes",
                "k8s",
                "terraform",
                "ansible",
                "monitoring",
            ],
        ),
        (
            "security",
            &[
                "security",
                "auth",
                "encrypt",
                "credential",
                "password",
                "vault",
                "secret",
                "scan",
                "audit",
            ],
        ),
        (
            "writing",
            &[
                "write",
                "blog",
                "article",
                "document",
                "markdown",
                "note",
                "journal",
                "edit",
                "grammar",
                "translate",
            ],
        ),
        (
            "research",
            &[
                "search",
                "research",
                "web",
                "browse",
                "fetch",
                "scrape",
                "crawl",
                "analyze",
                "summarize",
            ],
        ),
        (
            "data",
            &[
                "data",
                "database",
                "sql",
                "csv",
                "json",
                "excel",
                "spreadsheet",
                "import",
                "export",
                "etl",
                "transform",
            ],
        ),
    ];

    for (category, keywords) in &categories {
        if keywords.iter().any(|kw| haystack.contains(kw)) {
            return category.to_string();
        }
    }

    "general".to_string()
}

/// Helper: load and parse all skills from the workspace skills directory.
/// Returns Vec<(id, SkillDetail)>.
pub(crate) async fn load_all_skill_details(skills_dir: &std::path::Path) -> Vec<SkillDetail> {
    let mut skills = Vec::new();

    if !skills_dir.exists() {
        return skills;
    }

    let Ok(mut rd) = tokio::fs::read_dir(skills_dir).await else {
        return skills;
    };

    while let Ok(Some(entry)) = rd.next_entry().await {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let skill_file = path.join("SKILL.md");
        if !skill_file.exists() {
            continue;
        }

        let Ok(content) = tokio::fs::read_to_string(&skill_file).await else {
            continue;
        };

        let id = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        let enabled = !path.join(".disabled").exists();

        match zeus_skills::parse_skill_md(&content, path.clone()) {
            Ok(skill) => {
                skills.push(build_skill_detail(&skill, &id, enabled));
            }
            Err(e) => {
                debug!("Failed to parse skill {}: {}", id, e);
                // Fallback: include with minimal info so it still shows up
                skills.push(SkillDetail {
                    id: id.clone(),
                    name: id,
                    slug: String::new(),
                    description: format!("Parse error: {}", e),
                    version: "0.0.0".to_string(),
                    author: None,
                    enabled,
                    permissions: vec![],
                    category: "general".to_string(),
                    tags: vec![],
                    emoji: None,
                    homepage: None,
                    os: None,
                    primary_env: None,
                    user_invocable: true,
                    disable_model_invocation: false,
                    requires: None,
                    install_specs: None,
                    tools_count: 0,
                    command_dispatch: None,
                });
            }
        }
    }

    skills
}

/// Recursively collect files and total size under a directory
fn collect_dir_files(dir: &std::path::Path, files: &mut Vec<String>, total_size: &mut u64) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_dir_files(&path, files, total_size);
            } else if let Ok(meta) = path.metadata() {
                *total_size += meta.len();
                files.push(path.to_string_lossy().to_string());
            }
        }
    }
}

// ============================================================================
// Skills Endpoints
// ============================================================================

/// List installed skills (enriched with OpenClaw metadata)
pub async fn list_skills(State(state): State<SharedState>) -> Json<Value> {
    let state = state.read().await;
    let skills_dir = state.config.workspace.join("skills");
    let skills = load_all_skill_details(&skills_dir).await;
    let total = skills.len();

    Json(json!({ "skills": skills, "total": total }))
}

/// Get a single skill by ID with full detail
pub async fn get_skill(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;
    let skill_dir = state.config.workspace.join("skills").join(&id);

    if !skill_dir.exists() {
        return Err((StatusCode::NOT_FOUND, format!("Skill not found: {}", id)));
    }

    let skill_file = skill_dir.join("SKILL.md");
    let content = tokio::fs::read_to_string(&skill_file)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let skill = zeus_skills::parse_skill_md(&content, skill_dir.clone()).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Parse error: {}", e),
        )
    })?;

    let enabled = !skill_dir.join(".disabled").exists();
    let detail = build_skill_detail(&skill, &id, enabled);

    // Include system_prompt and tools in the full detail view
    let tools: Vec<Value> = skill
        .tools
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "input_schema": t.input_schema,
            })
        })
        .collect();

    let mut response = serde_json::to_value(&detail).unwrap_or_else(|_| json!({}));
    if let Some(obj) = response.as_object_mut() {
        obj.insert("system_prompt".to_string(), json!(skill.system_prompt));
        obj.insert("tools".to_string(), json!(tools));
        obj.insert("frontmatter".to_string(), json!(skill.frontmatter));
    }

    Ok(Json(response))
}

/// Get raw SKILL.md content and parsed tool schemas
pub async fn get_skill_schema(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;
    let skill_dir = state.config.workspace.join("skills").join(&id);

    if !skill_dir.exists() {
        return Err((StatusCode::NOT_FOUND, format!("Skill not found: {}", id)));
    }

    let skill_file = skill_dir.join("SKILL.md");
    let raw_content = tokio::fs::read_to_string(&skill_file)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let skill = zeus_skills::parse_skill_md(&raw_content, skill_dir.clone()).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Parse error: {}", e),
        )
    })?;

    let tool_schemas: Vec<Value> = skill
        .tools
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "input_schema": t.input_schema,
                "implementation": t.implementation,
            })
        })
        .collect();

    Ok(Json(json!({
        "id": id,
        "raw_content": raw_content,
        "tool_schemas": tool_schemas,
        "frontmatter": skill.frontmatter,
    })))
}

/// Search skills by text query, category, or enabled status
pub async fn search_skills(
    State(state): State<SharedState>,
    Query(params): Query<SkillSearchQuery>,
) -> Json<Value> {
    let state = state.read().await;
    let skills_dir = state.config.workspace.join("skills");
    let all_skills = load_all_skill_details(&skills_dir).await;

    let filtered: Vec<&SkillDetail> = all_skills
        .iter()
        .filter(|s| {
            // Text search
            if let Some(ref q) = params.q {
                let q_lower = q.to_lowercase();
                let matches = s.name.to_lowercase().contains(&q_lower)
                    || s.description.to_lowercase().contains(&q_lower)
                    || s.tags.iter().any(|t| t.to_lowercase().contains(&q_lower))
                    || s.category.to_lowercase().contains(&q_lower);
                if !matches {
                    return false;
                }
            }
            // Category filter
            if let Some(ref cat) = params.category
                && s.category.to_lowercase() != cat.to_lowercase()
            {
                return false;
            }
            // Enabled filter
            if let Some(enabled) = params.enabled
                && s.enabled != enabled
            {
                return false;
            }
            true
        })
        .collect();

    let total = filtered.len();
    Json(json!({ "skills": filtered, "total": total }))
}

/// List skill categories with counts
pub async fn list_skill_categories(State(state): State<SharedState>) -> Json<Value> {
    let state = state.read().await;
    let skills_dir = state.config.workspace.join("skills");
    let all_skills = load_all_skill_details(&skills_dir).await;

    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for skill in &all_skills {
        *counts.entry(skill.category.clone()).or_insert(0) += 1;
    }

    let mut categories: Vec<Value> = counts
        .into_iter()
        .map(|(name, count)| json!({ "name": name, "count": count }))
        .collect();
    categories.sort_by(|a, b| {
        b.get("count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
            .cmp(&a.get("count").and_then(|v| v.as_u64()).unwrap_or(0))
    });

    Json(json!({ "categories": categories, "total": all_skills.len() }))
}

/// Install a skill from ClawHub registry by name
pub async fn install_clawhub_skill(
    State(state): State<SharedState>,
    Json(req): Json<ClawHubInstallRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;
    let skills_dir = state.config.workspace.join("skills");
    let mut client = zeus_skills::ClawHubClient::new(skills_dir);

    match client.install(&req.name).await {
        Ok(result) => {
            info!(
                "Installed skill from ClawHub: {} v{}",
                result.name, result.version
            );

            // Assign source-aware permission policy (clawhub = restricted)
            let policy = zeus_skills::SkillPermissionPolicy::for_source(&result.name, "clawhub");
            let trust_level = policy.trust_level;

            // Write-through: register as marketplace listing
            let row = marketplace_store::SkillListingRow {
                id: result.name.clone(),
                name: result.name.clone(),
                description: format!("{} (installed from ClawHub)", result.name),
                publisher_id: "clawhub".to_string(),
                capabilities_json: serde_json::to_string(&result.permissions)
                    .unwrap_or_else(|_| "[]".to_string()),
                tags_json: "[]".to_string(),
                price: 0,
                version: result.version.clone(),
                rating: 0.0,
                rating_count: 0,
                downloads: 1,
                active: true,
                source: "clawhub".to_string(),
                metadata_json: serde_json::json!({
                    "trust_level": trust_level,
                    "sandbox": "marketplace_restricted",
                    "max_execution_secs": policy.max_execution_secs,
                })
                .to_string(),
                created_at: chrono::Utc::now().to_rfc3339(),
                updated_at: chrono::Utc::now().to_rfc3339(),
            };
            state.marketplace_store.publish_listing(&row).await;

            Ok(Json(json!({
                "success": true,
                "name": result.name,
                "version": result.version,
                "permissions": result.permissions,
                "warnings": result.warnings,
                "trust_level": trust_level,
                "sandbox": "marketplace_restricted",
                "message": format!("Skill '{}' v{} installed from ClawHub (restricted sandbox)", result.name, result.version),
            })))
        }
        Err(e) => Err((
            StatusCode::BAD_GATEWAY,
            format!("ClawHub install failed: {}", e),
        )),
    }
}

/// Install a skill from URL or content
pub async fn install_skill(
    State(state): State<SharedState>,
    Json(req): Json<InstallSkillRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;
    let skills_dir = state.config.workspace.join("skills");

    let content = if let Some(content) = req.content {
        content
    } else if let Some(url) = req.url {
        // SSRF protection: validate URL before fetching
        let validated_url = url_validator::validate_url(&url).map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                format!("Invalid URL (SSRF protection): {}", e),
            )
        })?;

        let response = reqwest::get(validated_url).await.map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                format!("Failed to fetch skill from URL: {}", e),
            )
        })?;
        if !response.status().is_success() {
            return Err((
                StatusCode::BAD_GATEWAY,
                format!("URL returned {}", response.status()),
            ));
        }
        response.text().await.map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                format!("Failed to read response body: {}", e),
            )
        })?
    } else {
        return Err((
            StatusCode::BAD_REQUEST,
            "Either 'url' or 'content' is required".to_string(),
        ));
    };

    // Parse to get the skill name
    let mut name = String::new();
    for line in content.lines() {
        if let Some(stripped) = line.strip_prefix("# ") {
            name = stripped.trim().to_lowercase().replace(' ', "-");
            break;
        }
    }

    if name.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "SKILL.md content missing name (# heading)".to_string(),
        ));
    }

    let skill_dir = skills_dir.join(&name);
    tokio::fs::create_dir_all(&skill_dir)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    tokio::fs::write(skill_dir.join("SKILL.md"), &content)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    info!("Installed skill: {}", name);

    // Assign source-aware permission policy (local = basic)
    let policy = zeus_skills::SkillPermissionPolicy::for_source(&name, "local");
    let trust_level = policy.trust_level;

    // Write-through: register as marketplace listing
    let row = marketplace_store::SkillListingRow {
        id: name.clone(),
        name: name.clone(),
        description: format!("{} skill", name),
        publisher_id: "local".to_string(),
        capabilities_json: "[]".to_string(),
        tags_json: "[]".to_string(),
        price: 0,
        version: "1.0.0".to_string(),
        rating: 0.0,
        rating_count: 0,
        downloads: 1,
        active: true,
        source: "local".to_string(),
        metadata_json: serde_json::json!({
            "trust_level": trust_level,
            "sandbox": "basic",
            "max_execution_secs": policy.max_execution_secs,
        })
        .to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
        updated_at: chrono::Utc::now().to_rfc3339(),
    };
    state.marketplace_store.publish_listing(&row).await;

    Ok(Json(json!({
        "success": true,
        "id": name,
        "trust_level": trust_level,
        "sandbox": "basic",
        "message": format!("Skill '{}' installed (basic sandbox)", name)
    })))
}

/// Update a skill (enable/disable)
pub async fn update_skill(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateSkillRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;
    let skill_dir = state.config.workspace.join("skills").join(&id);

    if !skill_dir.exists() {
        return Err((StatusCode::NOT_FOUND, format!("Skill not found: {}", id)));
    }

    if let Some(enabled) = req.enabled {
        let disabled_marker = skill_dir.join(".disabled");
        if enabled {
            // Remove disabled marker if it exists
            let _ = tokio::fs::remove_file(&disabled_marker).await;
        } else {
            // Create disabled marker
            tokio::fs::write(&disabled_marker, "")
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        }
    }

    Ok(Json(json!({
        "success": true,
        "id": id,
        "message": format!("Skill '{}' updated", id)
    })))
}

/// Delete/uninstall a skill
///
/// Supports query params:
///   ?dry_run=true  — preview what would be removed
///   ?keep_files=true — remove from registry but keep files on disk
pub async fn delete_skill(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Query(params): Query<UninstallQuery>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;
    let skill_dir = state.config.workspace.join("skills").join(&id);

    if !skill_dir.exists() {
        return Err((StatusCode::NOT_FOUND, format!("Skill not found: {}", id)));
    }

    let dry_run = params.dry_run.unwrap_or(false);
    let keep_files = params.keep_files.unwrap_or(false);

    if dry_run {
        // Collect files that would be removed
        let mut files = Vec::new();
        let mut total_size = 0u64;
        collect_dir_files(&skill_dir, &mut files, &mut total_size);

        return Ok(Json(json!({
            "dry_run": true,
            "id": id,
            "skill_dir": skill_dir.to_string_lossy(),
            "files": files,
            "total_size": total_size,
            "message": format!("Would uninstall skill '{}'", id)
        })));
    }

    if !keep_files {
        tokio::fs::remove_dir_all(&skill_dir)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }

    info!("Uninstalled skill: {} (keep_files={})", id, keep_files);

    Ok(Json(json!({
        "success": true,
        "id": id,
        "keep_files": keep_files,
        "message": format!("Skill '{}' uninstalled", id)
    })))
}

// ============================================================================
// Skill Cards (Agora -> Pantheon social wiring)
// ============================================================================

/// POST /v1/pantheon/rooms/:id/skill-card — share a skill card in a room
pub async fn share_skill_card(
    State(state): State<SharedState>,
    Path(room_id): Path<String>,
    Json(body): Json<Value>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, String)> {
    let skill_id = body
        .get("skill_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing 'skill_id'".to_string()))?;
    let sender_id = body
        .get("sender_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "Missing 'sender_id'".to_string()))?;
    let sender_name = body
        .get("sender_name")
        .and_then(|v| v.as_str())
        .unwrap_or(sender_id);

    let state_guard = state.read().await;

    // Look up the skill
    let listing = state_guard
        .marketplace_store
        .get_listing(skill_id)
        .await
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Skill not found".to_string()))?;

    // Get publisher badge
    let pub_rep = state_guard
        .marketplace_store
        .get_reputation_with_badge(&listing.publisher_id)
        .await;

    let skill_card = marketplace_store::SkillCard {
        skill_id: listing.id.clone(),
        skill_name: listing.name.clone(),
        publisher_id: listing.publisher_id.clone(),
        publisher_badge: pub_rep.badge,
        price_tokens: listing.price,
        rating: listing.rating,
        rating_count: listing.rating_count,
        tags: serde_json::from_str(&listing.tags_json).unwrap_or(json!([])),
        description: listing.description.clone(),
        can_invoke: true,
    };

    // Post as a room message with type "skill_card"
    let msg_id = uuid::Uuid::new_v4().to_string();
    let card_json = serde_json::to_value(&skill_card).unwrap_or_default();
    let room_msg = pantheon::RoomMessage {
        id: msg_id.clone(),
        room_id: room_id.clone(),
        sender_id: sender_id.to_string(),
        sender_name: sender_name.to_string(),
        content: format!("Shared skill: {}", listing.name),
        message_type: "skill_card".to_string(),
        metadata: Some(card_json.clone()),
        reply_to: None,
        edited: false,
        attachments: vec![],
        timestamp: chrono::Utc::now(),
    };
    state_guard.pantheon.insert_room_message(&room_msg).await;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "message_id": msg_id,
            "skill_card": card_json,
        })),
    ))
}
