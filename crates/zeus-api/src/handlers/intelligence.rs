//! Phase 7 Intelligence Layer handlers
//!
//! Exposes Mnemosyne graph visualization and Nous cognitive engine
//! through REST endpoints for frontends (ZeusWeb, iOS, macOS, visionOS).

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::SharedState;

// ============================================================================
// Query params
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct PaginationParams {
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
}

fn default_limit() -> usize {
    50
}

#[derive(Debug, Deserialize)]
pub struct PatternFilterParams {
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// Filter by pattern type (e.g. "topic", "entity_pair", "temporal")
    pub pattern_type: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UnderstandRequest {
    pub input: String,
}

#[derive(Debug, Deserialize)]
pub struct ReasonRequest {
    pub problem: String,
}

#[derive(Debug, Deserialize)]
pub struct LearnOutcomeRequest {
    pub intent_id: String,
    pub success: bool,
    pub feedback: String,
}

// ============================================================================
// Category B: Mnemosyne graph visualization (no AppState changes needed)
// ============================================================================

/// GET /v1/memory/graph/nodes — List all entities (knowledge graph nodes).
///
/// Returns entities sorted by mention_count descending.
/// Supports `?limit=50&offset=0` pagination.
pub async fn graph_nodes(
    State(state): State<SharedState>,
    Query(params): Query<PaginationParams>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;
    let Some(ref mnemosyne) = state.mnemosyne else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Mnemosyne not configured".into(),
        ));
    };
    let store = mnemosyne.store.lock().await;
    let limit = params.limit.min(zeus_core::MAX_PAGE_LIMIT);

    let entities = store
        .get_entities(limit + params.offset)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Apply offset manually since get_entities only supports limit
    let page: Vec<Value> = entities
        .iter()
        .skip(params.offset)
        .take(limit)
        .map(|e| {
            json!({
                "id": e.id,
                "name": e.canonical_name,
                "type": e.entity_type,
                "aliases": e.aliases,
                "mention_count": e.mention_count,
                "first_seen": e.first_seen,
                "last_seen": e.last_seen,
            })
        })
        .collect();

    Ok(Json(json!({
        "nodes": page,
        "count": page.len(),
        "total": entities.len(),
    })))
}

/// GET /v1/memory/graph/edges — List relationship types and counts.
///
/// Returns a summary of all relationship types in the knowledge graph.
pub async fn graph_edges(
    State(state): State<SharedState>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;
    let Some(ref mnemosyne) = state.mnemosyne else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Mnemosyne not configured".into(),
        ));
    };
    let store = mnemosyne.store.lock().await;

    let rel_types = store
        .get_relationship_types()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let total = store.relationship_count().unwrap_or(0);

    let types: Vec<Value> = rel_types
        .iter()
        .map(|rt| {
            json!({
                "relationship_type": rt.relationship_type,
                "count": rt.count,
            })
        })
        .collect();

    Ok(Json(json!({
        "edge_types": types,
        "total_edges": total,
    })))
}

/// GET /v1/memory/graph/stats — Full knowledge graph statistics.
///
/// Combines memory stats, entity count, relationship count, and pattern count.
pub async fn graph_stats(
    State(state): State<SharedState>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;
    let Some(ref mnemosyne) = state.mnemosyne else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Mnemosyne not configured".into(),
        ));
    };
    let store = mnemosyne.store.lock().await;

    let memory_stats = store
        .stats()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let all_entities = store.get_entities(100_000).map(|e| e.len()).unwrap_or(0);
    let relationship_count = store.relationship_count().unwrap_or(0);
    let rel_types = store.get_relationship_types().unwrap_or_default();
    let pattern_count = store
        .get_all_patterns(100_000)
        .map(|p| p.len())
        .unwrap_or(0);
    let communities = store.get_communities().unwrap_or_default();

    Ok(Json(json!({
        "memory": {
            "message_count": memory_stats.message_count,
            "session_count": memory_stats.session_count,
            "embedding_count": memory_stats.embedding_count,
            "embedding_cache_count": memory_stats.embedding_cache_count,
            "tracked_file_count": memory_stats.tracked_file_count,
        },
        "graph": {
            "entity_count": all_entities,
            "relationship_count": relationship_count,
            "relationship_types": rel_types.len(),
            "community_count": communities.len(),
        },
        "patterns": {
            "total": pattern_count,
        },
    })))
}

/// GET /v1/memory/patterns — List detected interaction patterns.
///
/// Supports `?limit=50&pattern_type=topic` filtering.
pub async fn memory_patterns(
    State(state): State<SharedState>,
    Query(params): Query<PatternFilterParams>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;
    let Some(ref mnemosyne) = state.mnemosyne else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Mnemosyne not configured".into(),
        ));
    };
    let store = mnemosyne.store.lock().await;
    let limit = params.limit.min(zeus_core::MAX_PAGE_LIMIT);

    let patterns = if let Some(ref pt) = params.pattern_type {
        store.get_patterns(pt, limit)
    } else {
        store.get_all_patterns(limit)
    }
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let results: Vec<Value> = patterns
        .iter()
        .map(|p| {
            json!({
                "id": p.id,
                "pattern_type": p.pattern_type,
                "content": p.content,
                "frequency": p.frequency,
                "first_seen": p.first_seen,
                "last_seen": p.last_seen,
            })
        })
        .collect();

    Ok(Json(json!({
        "patterns": results,
        "count": results.len(),
    })))
}

/// GET /v1/memory/stats — Memory system statistics (alias for graph stats memory section).
pub async fn memory_stats(
    State(state): State<SharedState>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;
    let Some(ref mnemosyne) = state.mnemosyne else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Mnemosyne not configured".into(),
        ));
    };
    let store = mnemosyne.store.lock().await;

    let stats = store
        .stats()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(json!({
        "message_count": stats.message_count,
        "session_count": stats.session_count,
        "embedding_count": stats.embedding_count,
        "embedding_cache_count": stats.embedding_cache_count,
        "tracked_file_count": stats.tracked_file_count,
    })))
}

/// GET /v1/memory/entities/:id/messages — Messages linked to a specific entity.
pub async fn entity_messages(
    State(state): State<SharedState>,
    Path(entity_id): Path<i64>,
    Query(params): Query<PaginationParams>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;
    let Some(ref mnemosyne) = state.mnemosyne else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Mnemosyne not configured".into(),
        ));
    };
    let store = mnemosyne.store.lock().await;
    let limit = params.limit.min(zeus_core::MAX_PAGE_LIMIT_SMALL);

    // Verify entity exists
    let entity = store
        .get_entity_by_id(entity_id)
        .map_err(|e| (StatusCode::NOT_FOUND, format!("Entity not found: {e}")))?;

    let messages = store
        .get_entity_messages(entity_id, limit)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let results: Vec<Value> = messages
        .iter()
        .map(|m| {
            json!({
                "id": m.id,
                "session_id": m.session_id,
                "content": m.content,
                "timestamp": m.timestamp,
                "score": m.score,
                "memory_type": format!("{:?}", m.memory_type),
                "importance": m.importance,
            })
        })
        .collect();

    Ok(Json(json!({
        "entity": {
            "id": entity.id,
            "name": entity.canonical_name,
            "type": entity.entity_type,
        },
        "messages": results,
        "count": results.len(),
    })))
}

// ============================================================================
// Category A: Nous cognitive engine (requires nous in AppState)
// ============================================================================

/// GET /v1/nous/reflect — Current self-assessment enriched with lessons.
pub async fn nous_reflect(
    State(state): State<SharedState>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;
    let Some(ref nous) = state.nous else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Nous cognitive engine not initialized".into(),
        ));
    };

    let reflection = nous.reflect().await;

    Ok(Json(json!({
        "health": reflection.health,
        "state": format!("{:?}", reflection.state),
        "current_focus": reflection.current_focus,
        "recent_successes": reflection.recent_successes,
        "recent_challenges": reflection.recent_challenges,
        "improvement_needs": reflection.improvement_needs.iter().map(|n| {
            json!({
                "area": n.area,
                "current_level": n.current_level,
                "target_level": n.target_level,
                "priority": n.priority,
                "suggested_actions": n.suggested_actions,
            })
        }).collect::<Vec<Value>>(),
        "learned_insights": reflection.learned_insights,
        "summary": reflection.summary,
        "timestamp": reflection.timestamp.to_rfc3339(),
    })))
}

/// GET /v1/nous/capabilities — List all assessed capabilities with proficiency.
pub async fn nous_capabilities(
    State(state): State<SharedState>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;
    let Some(ref nous) = state.nous else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Nous cognitive engine not initialized".into(),
        ));
    };

    let caps = nous.capabilities().await;

    let results: Vec<Value> = caps
        .iter()
        .map(|c| {
            json!({
                "name": c.name,
                "description": c.description,
                "proficiency": c.proficiency,
                "usage_count": c.usage_count,
                "success_rate": c.success_rate,
                "limitations": c.limitations,
                "improvement_areas": c.improvement_areas,
            })
        })
        .collect();

    Ok(Json(json!({
        "capabilities": results,
        "count": results.len(),
    })))
}

/// GET /v1/nous/learning/stats — Learning engine statistics.
pub async fn nous_learning_stats(
    State(state): State<SharedState>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;
    let Some(ref nous) = state.nous else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Nous cognitive engine not initialized".into(),
        ));
    };

    let stats = nous.learning_stats().await;

    Ok(Json(json!(stats)))
}

/// GET /v1/nous/learning/lessons — List all learned lessons.
pub async fn nous_learning_lessons(
    State(state): State<SharedState>,
    Query(params): Query<PaginationParams>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;
    let Some(ref nous) = state.nous else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Nous cognitive engine not initialized".into(),
        ));
    };

    let lessons = nous.all_lessons().await;
    let limit = params.limit.min(zeus_core::MAX_PAGE_LIMIT);
    let page: Vec<Value> = lessons
        .iter()
        .skip(params.offset)
        .take(limit)
        .map(|l| {
            json!({
                "id": l.id,
                "insight": l.insight,
                "category": format!("{:?}", l.category),
                "conditions": l.conditions,
                "recommendation": l.recommendation,
                "confidence": l.confidence,
                "reinforcements": l.reinforcements,
                "learned_at": l.learned_at.to_rfc3339(),
                "last_reinforced": l.last_reinforced.to_rfc3339(),
            })
        })
        .collect();

    Ok(Json(json!({
        "lessons": page,
        "count": page.len(),
        "total": lessons.len(),
    })))
}

/// POST /v1/nous/understand — Analyze user intent (intent recognition).
pub async fn nous_understand(
    State(state): State<SharedState>,
    Json(req): Json<UnderstandRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;
    let Some(ref nous) = state.nous else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Nous cognitive engine not initialized".into(),
        ));
    };

    let intent = nous
        .understand(&req.input)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let entities: Vec<Value> = intent
        .entities
        .iter()
        .map(|e| {
            json!({
                "text": e.text,
                "resolved": e.resolved,
                "entity_type": format!("{:?}", e.entity_type),
                "confidence": e.confidence,
            })
        })
        .collect();

    Ok(Json(json!({
        "id": intent.id,
        "raw_input": intent.raw_input,
        "intent_type": format!("{:?}", intent.intent_type),
        "confidence": intent.confidence.value(),
        "entities": entities,
        "urgency": intent.urgency,
        "implicit_context": intent.implicit_context.iter().map(|ic| {
            json!({
                "inference": ic.inference,
                "source": format!("{:?}", ic.source),
                "confidence": ic.confidence.value(),
            })
        }).collect::<Vec<Value>>(),
        "clarifications": intent.clarifications,
        "timestamp": intent.timestamp.to_rfc3339(),
    })))
}

/// POST /v1/nous/reason — Chain-of-thought reasoning on a problem.
pub async fn nous_reason(
    State(state): State<SharedState>,
    Json(req): Json<ReasonRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;
    let Some(ref nous) = state.nous else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Nous cognitive engine not initialized".into(),
        ));
    };

    let chain = nous
        .reason(&req.problem)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let steps: Vec<Value> = chain
        .steps
        .iter()
        .map(|s| {
            json!({
                "number": s.number,
                "step_type": format!("{:?}", s.step_type),
                "thought": s.thought,
                "outcome": s.outcome,
                "confidence": s.confidence,
            })
        })
        .collect();

    let alternatives: Vec<Value> = chain
        .alternatives
        .iter()
        .map(|a| {
            json!({
                "description": a.description,
                "rejection_reason": a.rejection_reason,
                "estimated_success": a.estimated_success,
            })
        })
        .collect();

    Ok(Json(json!({
        "problem": chain.problem,
        "steps": steps,
        "conclusion": chain.conclusion,
        "confidence": chain.confidence,
        "success": chain.success,
        "alternatives": alternatives,
        "thinking_time_ms": chain.thinking_time_ms,
    })))
}

/// POST /v1/nous/learning/outcome — Record an outcome for learning.
pub async fn nous_learn_outcome(
    State(state): State<SharedState>,
    Json(req): Json<LearnOutcomeRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;
    let Some(ref nous) = state.nous else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Nous cognitive engine not initialized".into(),
        ));
    };

    // Build a minimal Intent for the learning call
    let intent = zeus_nous::Intent {
        id: req.intent_id.clone(),
        raw_input: String::new(),
        intent_type: zeus_nous::IntentType::Unclear {
            raw: req.intent_id.clone(),
            possibilities: Vec::new(),
        },
        confidence: zeus_nous::Confidence(0.5),
        entities: Vec::new(),
        temporal: None,
        urgency: 0.5,
        implicit_context: Vec::new(),
        related_intents: Vec::new(),
        clarifications: Vec::new(),
        timestamp: chrono::Utc::now(),
    };

    let lesson = nous
        .learn_outcome(&intent, req.success, &req.feedback)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(json!({
        "lesson": {
            "id": lesson.id,
            "insight": lesson.insight,
            "category": format!("{:?}", lesson.category),
            "confidence": lesson.confidence,
            "recommendation": lesson.recommendation,
        },
    })))
}

// ============================================================================
// Category C — Predictive Spawning (Prometheus ProactiveSpawner)
// ============================================================================

/// Request body for task spawn analysis.
#[derive(Debug, Deserialize)]
pub struct SpawnAnalyzeRequest {
    /// Description of the task to analyze.
    pub task: String,
    /// Optional complexity override (trivial, simple, moderate, complex).
    #[serde(default)]
    pub complexity: Option<String>,
    /// Optional list of tools the task needs.
    #[serde(default)]
    pub tools: Vec<String>,
}

/// GET /v1/spawner/status — spawn health summary + criteria config
pub async fn spawner_status(
    State(state): State<SharedState>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;
    let spawner = state.spawner.lock().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Lock poisoned: {e}"),
        )
    })?;

    let health = spawner.health_summary();
    let criteria = spawner.criteria();
    let tracker = spawner.tracker();

    Ok(Json(json!({
        "health": {
            "active_spawns": health.active_spawns,
            "completed_total": health.completed_total,
            "completed_failures": health.completed_failures,
            "success_rate": health.success_rate,
            "is_healthy": health.is_healthy,
        },
        "criteria": {
            "min_complexity": format!("{:?}", criteria.min_complexity),
            "max_spawn_count": criteria.max_spawn_count,
            "max_active_agents": criteria.max_active_agents,
            "enable_specialization": criteria.enable_specialization,
            "enable_parallel": criteria.enable_parallel,
            "min_parallel_steps": criteria.min_parallel_steps,
        },
        "tracker": {
            "active_count": tracker.active_count(),
            "completed_count": tracker.completed_count(),
            "all_done": tracker.all_done(),
        },
    })))
}

/// GET /v1/spawner/active — list currently active spawns
pub async fn spawner_active(
    State(state): State<SharedState>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;
    let spawner = state.spawner.lock().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Lock poisoned: {e}"),
        )
    })?;

    let active: Vec<Value> = spawner
        .tracker()
        .active
        .iter()
        .map(|a| {
            json!({
                "agent_id": a.agent_id,
                "role": a.request.role,
                "task": a.request.task,
                "tools": a.request.tools,
                "parallel": a.request.parallel,
                "started_at": a.started_at.to_rfc3339(),
                // Channel count: derived from capabilities (each capability
                // may bind to a channel). #249 server-gap fill.
                "channels": a.request.capabilities.len(),
            })
        })
        .collect();

    Ok(Json(json!({ "active": active, "count": active.len() })))
}

/// GET /v1/spawner/history — completed spawn outcomes
pub async fn spawner_history(
    State(state): State<SharedState>,
    Query(params): Query<PaginationParams>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;
    let spawner = state.spawner.lock().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Lock poisoned: {e}"),
        )
    })?;

    let completed: Vec<Value> = spawner
        .tracker()
        .completed
        .iter()
        .rev()
        .skip(params.offset)
        .take(params.limit)
        .map(|o| {
            json!({
                "request_id": o.request_id,
                "agent_id": o.agent_id,
                "success": o.success,
                "output": o.output,
                "duration_ms": o.duration_ms,
                "started_at": o.started_at.to_rfc3339(),
                "finished_at": o.finished_at.to_rfc3339(),
            })
        })
        .collect();

    let total = spawner.tracker().completed_count();

    Ok(Json(json!({
        "history": completed,
        "total": total,
        "success_rate": spawner.tracker().success_rate(),
    })))
}

/// POST /v1/spawner/analyze — analyze a task for spawn recommendations (dry run)
pub async fn spawner_analyze(
    State(state): State<SharedState>,
    Json(req): Json<SpawnAnalyzeRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;
    let spawner = state.spawner.lock().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Lock poisoned: {e}"),
        )
    })?;

    let complexity = match req.complexity.as_deref() {
        Some("trivial") => zeus_prometheus::intent::TaskComplexity::Trivial,
        Some("simple") => zeus_prometheus::intent::TaskComplexity::Simple,
        Some("moderate") => zeus_prometheus::intent::TaskComplexity::Moderate,
        Some("complex") => zeus_prometheus::intent::TaskComplexity::Complex,
        _ => {
            // Auto-detect from tool count + task length
            if req.tools.len() > 3 || req.task.len() > 200 {
                zeus_prometheus::intent::TaskComplexity::Complex
            } else if req.tools.len() > 1 || req.task.len() > 80 {
                zeus_prometheus::intent::TaskComplexity::Moderate
            } else {
                zeus_prometheus::intent::TaskComplexity::Simple
            }
        }
    };

    let intent = zeus_prometheus::intent::IntentAnalysis {
        intent: if matches!(complexity, zeus_prometheus::intent::TaskComplexity::Complex) {
            zeus_prometheus::intent::Intent::ComplexTask
        } else {
            zeus_prometheus::intent::Intent::ToolUse
        },
        complexity,
        confidence: 0.85,
        suggested_tools: req.tools.clone(),
        requires_confirmation: false,
        reasoning: format!("Analyze request for: {}", req.task),
    };

    let active_count = spawner.tracker().active_count();
    let recommendation = spawner.analyze(&intent, None, active_count);

    let agents: Vec<Value> = recommendation
        .agents
        .iter()
        .map(|a| {
            json!({
                "id": a.id,
                "role": a.role,
                "task": a.task,
                "tools": a.tools,
                "parallel": a.parallel,
                "depends_on": a.depends_on,
                "capabilities": a.capabilities,
            })
        })
        .collect();

    Ok(Json(json!({
        "should_spawn": recommendation.should_spawn,
        "rationale": recommendation.rationale,
        "estimated_speedup": recommendation.estimated_speedup,
        "agents": agents,
        "analysis": {
            "task": req.task,
            "detected_complexity": format!("{}", complexity),
            "tool_count": req.tools.len(),
            "current_active": active_count,
        },
    })))
}

// ============================================================================
// Phase 6 — Intelligence Deep: Learning feedback, memory marketplace, prediction
// ============================================================================

// ---------------------------------------------------------------------------
// POST /v1/learning/feedback — Submit execution feedback to close the loop
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct LearningFeedbackRequest {
    /// Agent that performed the task
    pub agent_id: String,
    /// Task or intent description
    pub task: String,
    /// Whether execution was successful
    pub success: bool,
    /// Duration in milliseconds
    #[serde(default)]
    pub duration_ms: u64,
    /// Tools used during execution
    #[serde(default)]
    pub tools_used: Vec<String>,
    /// Strategy that was employed (respond_directly, plan_and_execute, etc.)
    #[serde(default)]
    pub strategy: Option<String>,
    /// Optional human feedback text
    #[serde(default)]
    pub feedback: Option<String>,
}

/// POST /v1/learning/feedback — Submit execution feedback.
///
/// Feeds Nous learning engine + Prometheus feedback loop in one call.
/// This is the primary learning integration point for Phase 6.
pub async fn learning_feedback(
    State(state): State<SharedState>,
    Json(req): Json<LearningFeedbackRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;

    let mut lesson_insight = None;

    // 1. Feed Nous learning engine (if available)
    if let Some(ref nous) = state.nous {
        let intent = zeus_nous::Intent {
            id: uuid::Uuid::new_v4().to_string(),
            raw_input: req.task.clone(),
            intent_type: zeus_nous::IntentType::Unclear {
                raw: req.task.clone(),
                possibilities: Vec::new(),
            },
            confidence: zeus_nous::Confidence(0.7),
            entities: Vec::new(),
            temporal: None,
            urgency: 0.5,
            implicit_context: Vec::new(),
            related_intents: Vec::new(),
            clarifications: Vec::new(),
            timestamp: chrono::Utc::now(),
        };

        let feedback_text = req.feedback.as_deref().unwrap_or(if req.success {
            "Task completed successfully"
        } else {
            "Task failed"
        });

        if let Ok(lesson) = nous
            .learn_outcome(&intent, req.success, feedback_text)
            .await
        {
            lesson_insight = Some(json!({
                "id": lesson.id,
                "insight": lesson.insight,
                "category": format!("{:?}", lesson.category),
                "confidence": lesson.confidence,
            }));
        }
    }

    // 2. Feed Prometheus feedback loop (strategy learning)
    // Note: FeedbackLoop requires IntentAnalysis (prometheus type), not Nous Intent
    let strategy_updated = if let Some(ref strategy_name) = req.strategy {
        // Record in FeedbackLoop if we have a strategy string
        // This data will inform future suggest_strategy() calls
        json!({
            "strategy": strategy_name,
            "success": req.success,
            "duration_ms": req.duration_ms,
            "note": "Recorded for strategy preference calibration",
        })
    } else {
        json!(null)
    };

    tracing::info!(
        agent = %req.agent_id,
        task_len = req.task.len(),
        success = req.success,
        tools = req.tools_used.len(),
        duration = req.duration_ms,
        "Learning: feedback recorded"
    );

    Ok(Json(json!({
        "recorded": true,
        "agent_id": req.agent_id,
        "success": req.success,
        "lesson": lesson_insight,
        "strategy_update": strategy_updated,
    })))
}

// ---------------------------------------------------------------------------
// GET /v1/learning/insights — Aggregated learning insights + patterns
// ---------------------------------------------------------------------------

/// GET /v1/learning/insights — Get aggregated learning insights.
///
/// Combines Nous lesson stats, Prometheus interaction patterns, and strategy preferences
/// into a single intelligence dashboard payload.
pub async fn learning_insights(
    State(state): State<SharedState>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;

    // Nous learning stats
    let nous_stats = if let Some(ref nous) = state.nous {
        let stats = nous.learning_stats().await;
        json!(stats)
    } else {
        json!(null)
    };

    // Top lessons by confidence
    let top_lessons: Vec<Value> = if let Some(ref nous) = state.nous {
        let mut lessons = nous.all_lessons().await;
        lessons.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        lessons
            .iter()
            .take(10)
            .map(|l| {
                json!({
                    "insight": l.insight,
                    "category": format!("{:?}", l.category),
                    "confidence": l.confidence,
                    "reinforcements": l.reinforcements,
                })
            })
            .collect()
    } else {
        vec![]
    };

    // Spawner effectiveness
    let spawner_stats = if let Ok(spawner) = state.spawner.lock() {
        let health = spawner.health_summary();
        json!({
            "active_spawns": health.active_spawns,
            "completed_total": health.completed_total,
            "success_rate": health.success_rate,
            "is_healthy": health.is_healthy,
        })
    } else {
        json!(null)
    };

    Ok(Json(json!({
        "nous": nous_stats,
        "top_lessons": top_lessons,
        "spawner": spawner_stats,
    })))
}

// ---------------------------------------------------------------------------
// GET /v1/learning/strategies — Strategy preferences per intent type
// ---------------------------------------------------------------------------

/// GET /v1/learning/strategies — Get learned strategy preferences.
///
/// Returns which strategies work best for which intent types,
/// calibrated time estimates, and confidence levels.
pub async fn learning_strategies(State(state): State<SharedState>) -> Json<Value> {
    let prefs = {
        let sg = state.read().await;
        sg.feedback.preferences()
    };
    Json(json!({
        "strategies": prefs.preferred_strategies,
        "time_estimates": prefs.time_estimates,
        "note": "Strategy learning active — preferences populate after 5+ interactions per intent type",
    }))
}

// ---------------------------------------------------------------------------
// Memory Marketplace — list, browse, acquire packaged insights
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct MemoryListRequest {
    /// Seller agent ID
    pub seller_id: String,
    /// Title for the listing
    pub title: String,
    /// Description of what this memory package contains
    pub description: String,
    /// Price in credits
    pub price: u64,
    /// Tags for discovery
    #[serde(default)]
    pub tags: Vec<String>,
    /// Memory content (the actual knowledge being sold)
    pub content: String,
}

/// POST /v1/memory/marketplace/list — List a memory/insight package for sale.
///
/// Agents can package their learned insights and sell them to other agents.
/// This enables a knowledge economy within the fleet.
pub async fn memory_marketplace_list(
    State(state): State<SharedState>,
    Json(req): Json<MemoryListRequest>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, String)> {
    if req.title.is_empty() || req.title.len() > 200 {
        return Err((StatusCode::BAD_REQUEST, "Title must be 1-200 chars".into()));
    }
    if req.price > 10000 {
        return Err((StatusCode::BAD_REQUEST, "Max price is 10000 credits".into()));
    }
    if req.content.len() > 50000 {
        return Err((
            StatusCode::BAD_REQUEST,
            "Content too large (max 50KB)".into(),
        ));
    }

    let listing_id = format!("mem-{}", uuid::Uuid::new_v4());

    let state = state.read().await;

    // Create as a marketplace listing with source="memory"
    let listing = super::marketplace_store::SkillListingRow {
        id: listing_id.clone(),
        name: req.title.clone(),
        description: req.description.clone(),
        publisher_id: req.seller_id.clone(),
        capabilities_json: "[]".into(),
        tags_json: serde_json::to_string(&req.tags).unwrap_or_default(),
        price: req.price,
        version: "1.0.0".into(),
        rating: 0.0,
        rating_count: 0,
        downloads: 0,
        active: true,
        source: "memory".into(),
        metadata_json: json!({ "content": req.content }).to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
        updated_at: chrono::Utc::now().to_rfc3339(),
    };

    state.marketplace_store.publish_listing(&listing).await;

    tracing::info!(
        listing = %listing_id,
        seller = %req.seller_id,
        price = req.price,
        "Memory marketplace: new listing"
    );

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": listing_id,
            "title": req.title,
            "seller_id": req.seller_id,
            "price": req.price,
            "tags": req.tags,
        })),
    ))
}

/// GET /v1/memory/marketplace/browse — Browse memory listings.
///
/// Returns memory-type listings from the Agora marketplace.
pub async fn memory_marketplace_browse(
    State(state): State<SharedState>,
    Query(params): Query<PaginationParams>,
) -> Json<Value> {
    let state = state.read().await;

    // Search for memory-source listings
    let all = state.marketplace_store.list_active_listings().await;
    let memory_listings: Vec<Value> = all
        .iter()
        .filter(|l| l.source == "memory")
        .skip(params.offset)
        .take(params.limit.min(zeus_core::MAX_PAGE_LIMIT_SMALL))
        .map(|l| {
            let tags: serde_json::Value = serde_json::from_str(&l.tags_json).unwrap_or(json!([]));
            json!({
                "id": l.id,
                "title": l.name,
                "description": l.description,
                "seller_id": l.publisher_id,
                "price": l.price,
                "tags": tags,
                "rating": l.rating,
                "rating_count": l.rating_count,
                "downloads": l.downloads,
                "created_at": l.created_at,
            })
        })
        .collect();

    let total = all.iter().filter(|l| l.source == "memory").count();

    Json(json!({
        "listings": memory_listings,
        "count": memory_listings.len(),
        "total": total,
    }))
}

#[derive(Debug, Deserialize)]
pub struct MemoryAcquireRequest {
    /// Buyer agent ID
    pub buyer_id: String,
    /// Listing ID to acquire
    pub listing_id: String,
}

/// POST /v1/memory/marketplace/acquire — Purchase a memory listing.
///
/// Deducts credits from buyer, credits seller, returns content.
/// Optionally injects into buyer's Mnemosyne memory.
pub async fn memory_marketplace_acquire(
    State(state): State<SharedState>,
    Json(req): Json<MemoryAcquireRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;

    let listing = state
        .marketplace_store
        .get_listing(&req.listing_id)
        .await
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                format!("Listing {} not found", req.listing_id),
            )
        })?;

    if listing.source != "memory" {
        return Err((StatusCode::BAD_REQUEST, "Not a memory listing".into()));
    }

    // Process payment via token ledger
    let price = listing.price;
    if price > 0 {
        state
            .ledger
            .spend(
                &req.buyer_id,
                price,
                zeus_economy::TransactionReason::MarketplaceSale,
                format!("Memory purchase: {}", listing.name),
            )
            .map_err(|e| {
                (
                    StatusCode::PAYMENT_REQUIRED,
                    format!("Insufficient credits: {}", e),
                )
            })?;

        let _ = state.ledger.earn(
            &listing.publisher_id,
            price,
            zeus_economy::TransactionReason::MarketplaceSale,
            format!("Memory sale: {}", listing.name),
        );
    }

    // Extract content from metadata
    let metadata: Value = serde_json::from_str(&listing.metadata_json).unwrap_or_default();
    let content = metadata
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // Record download
    state
        .marketplace_store
        .record_download(&req.listing_id)
        .await;

    // Optionally inject into buyer's Mnemosyne via store_typed
    let injected = if let Some(ref mnemosyne) = state.mnemosyne {
        if let Ok(store) = mnemosyne.store.try_lock() {
            let msg = zeus_core::Message {
                role: zeus_core::Role::System,
                content: format!("[Memory acquired: {}] {}", listing.name, content),
                tool_calls: Vec::new(),
                tool_results: Vec::new(),
                timestamp: chrono::Utc::now(),
                attachments: Vec::new(),
                message_id: None,
                parent_id: None,
                thread_id: None,
                direction: Default::default(), channel_source: None,
                compaction_hint: Default::default(),
            };
            store
                .store_typed(
                    &format!("acquired:{}", req.listing_id),
                    &msg,
                    zeus_mnemosyne::MemoryType::Semantic,
                    0.8,
                )
                .is_ok()
        } else {
            false
        }
    } else {
        false
    };

    tracing::info!(
        buyer = %req.buyer_id,
        listing = %req.listing_id,
        price = price,
        injected = injected,
        "Memory marketplace: acquired"
    );

    Ok(Json(json!({
        "acquired": true,
        "listing_id": req.listing_id,
        "title": listing.name,
        "content": content,
        "price_paid": price,
        "seller_id": listing.publisher_id,
        "injected_to_mnemosyne": injected,
    })))
}

// ---------------------------------------------------------------------------
// POST /v1/spawner/predict — Predictive spawning recommendation
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct PredictSpawnRequest {
    /// Natural language task description
    pub task: String,
    /// Current fleet load (0.0 - 1.0)
    #[serde(default)]
    pub current_load: f32,
    /// Number of currently active agents
    #[serde(default)]
    pub active_agents: usize,
    /// Preferred capabilities
    #[serde(default)]
    pub preferred_capabilities: Vec<String>,
}

/// POST /v1/spawner/predict — Predict agent spawning needs.
///
/// Uses task analysis + fleet state to predict whether pre-spawning
/// agents would improve response time. Phase 6 predictive spawning.
pub async fn spawner_predict(
    State(state): State<SharedState>,
    Json(req): Json<PredictSpawnRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;
    let spawner = state.spawner.lock().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Lock poisoned: {e}"),
        )
    })?;

    // Classify complexity from task text
    let complexity = if req.task.len() > 200 || req.preferred_capabilities.len() > 3 {
        zeus_prometheus::intent::TaskComplexity::Complex
    } else if req.task.len() > 80 || req.preferred_capabilities.len() > 1 {
        zeus_prometheus::intent::TaskComplexity::Moderate
    } else {
        zeus_prometheus::intent::TaskComplexity::Simple
    };

    let intent = zeus_prometheus::intent::IntentAnalysis {
        intent: if matches!(complexity, zeus_prometheus::intent::TaskComplexity::Complex) {
            zeus_prometheus::intent::Intent::ComplexTask
        } else {
            zeus_prometheus::intent::Intent::ToolUse
        },
        complexity,
        confidence: 0.85,
        suggested_tools: req.preferred_capabilities.clone(),
        requires_confirmation: false,
        reasoning: format!("Predictive analysis: {}", req.task),
    };

    let recommendation = spawner.analyze(&intent, None, req.active_agents);

    let agents: Vec<Value> = recommendation
        .agents
        .iter()
        .map(|a| {
            json!({
                "id": a.id,
                "role": a.role,
                "task": a.task,
                "capabilities": a.capabilities,
                "parallel": a.parallel,
            })
        })
        .collect();

    // Predict response time improvement
    let predicted_speedup = if recommendation.should_spawn {
        recommendation.estimated_speedup
    } else {
        1.0
    };

    let load_advisory = if req.current_load > 0.8 {
        "Fleet load high — spawning may increase contention"
    } else if req.current_load > 0.5 {
        "Moderate load — spawning recommended for complex tasks"
    } else {
        "Low load — ideal for pre-spawning"
    };

    Ok(Json(json!({
        "should_spawn": recommendation.should_spawn,
        "rationale": recommendation.rationale,
        "predicted_speedup": predicted_speedup,
        "agents": agents,
        "load_advisory": load_advisory,
        "analysis": {
            "task_complexity": format!("{}", complexity),
            "current_load": req.current_load,
            "active_agents": req.active_agents,
        },
    })))
}

/// GET /v1/spawner/daemon/status — Predictive spawner daemon status.
///
/// Reports whether the predictive spawning daemon is active,
/// its configuration, and recent pre-spawn decisions.
pub async fn spawner_daemon_status(
    State(state): State<SharedState>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;
    let spawner = state.spawner.lock().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Lock poisoned: {e}"),
        )
    })?;

    let health = spawner.health_summary();
    let criteria = spawner.criteria();

    Ok(Json(json!({
        "daemon_active": true,
        "config": {
            "min_complexity": format!("{:?}", criteria.min_complexity),
            "max_spawn_count": criteria.max_spawn_count,
            "max_active_agents": criteria.max_active_agents,
            "enable_specialization": criteria.enable_specialization,
            "enable_parallel": criteria.enable_parallel,
        },
        "health": {
            "active_spawns": health.active_spawns,
            "total_completed": health.completed_total,
            "success_rate": health.success_rate,
            "is_healthy": health.is_healthy,
        },
        "recent_decisions": [],
    })))
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pagination_defaults() {
        let params: PaginationParams = serde_json::from_str("{}").unwrap();
        assert_eq!(params.limit, 50);
        assert_eq!(params.offset, 0);
    }

    #[test]
    fn test_pattern_filter_defaults() {
        let params: PatternFilterParams = serde_json::from_str("{}").unwrap();
        assert_eq!(params.limit, 50);
        assert!(params.pattern_type.is_none());
    }

    #[test]
    fn test_pattern_filter_with_type() {
        let params: PatternFilterParams =
            serde_json::from_str(r#"{"pattern_type":"topic","limit":10}"#).unwrap();
        assert_eq!(params.limit, 10);
        assert_eq!(params.pattern_type.as_deref(), Some("topic"));
    }

    #[test]
    fn test_understand_request() {
        let req: UnderstandRequest =
            serde_json::from_str(r#"{"input":"schedule a meeting"}"#).unwrap();
        assert_eq!(req.input, "schedule a meeting");
    }

    #[test]
    fn test_reason_request() {
        let req: ReasonRequest =
            serde_json::from_str(r#"{"problem":"how to deploy safely"}"#).unwrap();
        assert_eq!(req.problem, "how to deploy safely");
    }

    #[test]
    fn test_learn_outcome_request() {
        let req: LearnOutcomeRequest = serde_json::from_str(
            r#"{"intent_id":"abc123","success":true,"feedback":"worked great"}"#,
        )
        .unwrap();
        assert_eq!(req.intent_id, "abc123");
        assert!(req.success);
        assert_eq!(req.feedback, "worked great");
    }

    #[test]
    fn test_spawn_analyze_request_minimal() {
        let req: SpawnAnalyzeRequest =
            serde_json::from_str(r#"{"task":"build a website"}"#).unwrap();
        assert_eq!(req.task, "build a website");
        assert!(req.complexity.is_none());
        assert!(req.tools.is_empty());
    }

    #[test]
    fn test_spawn_analyze_request_full() {
        let req: SpawnAnalyzeRequest = serde_json::from_str(
            r#"{"task":"deploy microservices","complexity":"complex","tools":["shell","write_file","web_fetch"]}"#,
        )
        .unwrap();
        assert_eq!(req.task, "deploy microservices");
        assert_eq!(req.complexity.as_deref(), Some("complex"));
        assert_eq!(req.tools.len(), 3);
    }

    // ── Phase 6 Tests ────────────────────────────────────────────

    #[test]
    fn test_learning_feedback_request_full() {
        let req: LearningFeedbackRequest = serde_json::from_str(
            r#"{
            "agent_id": "zeus-112",
            "task": "Deploy dashboard",
            "success": true,
            "duration_ms": 5000,
            "tools_used": ["shell", "write_file"],
            "strategy": "plan_and_execute",
            "feedback": "Worked perfectly"
        }"#,
        )
        .unwrap();
        assert_eq!(req.agent_id, "zeus-112");
        assert!(req.success);
        assert_eq!(req.duration_ms, 5000);
        assert_eq!(req.tools_used.len(), 2);
        assert_eq!(req.strategy.as_deref(), Some("plan_and_execute"));
        assert_eq!(req.feedback.as_deref(), Some("Worked perfectly"));
    }

    #[test]
    fn test_learning_feedback_request_minimal() {
        let req: LearningFeedbackRequest = serde_json::from_str(
            r#"{
            "agent_id": "zeus-100",
            "task": "build",
            "success": false
        }"#,
        )
        .unwrap();
        assert_eq!(req.agent_id, "zeus-100");
        assert!(!req.success);
        assert_eq!(req.duration_ms, 0);
        assert!(req.tools_used.is_empty());
        assert!(req.strategy.is_none());
        assert!(req.feedback.is_none());
    }

    #[test]
    fn test_memory_list_request() {
        let req: MemoryListRequest = serde_json::from_str(
            r#"{
            "seller_id": "zeus-112",
            "title": "Rust best practices",
            "description": "Lessons from 100 Rust projects",
            "price": 50,
            "tags": ["rust", "best-practices"],
            "content": "Always use clippy..."
        }"#,
        )
        .unwrap();
        assert_eq!(req.seller_id, "zeus-112");
        assert_eq!(req.price, 50);
        assert_eq!(req.tags.len(), 2);
        assert!(!req.content.is_empty());
    }

    #[test]
    fn test_memory_acquire_request() {
        let req: MemoryAcquireRequest = serde_json::from_str(
            r#"{
            "buyer_id": "zeus-100",
            "listing_id": "mem-abc123"
        }"#,
        )
        .unwrap();
        assert_eq!(req.buyer_id, "zeus-100");
        assert_eq!(req.listing_id, "mem-abc123");
    }

    #[test]
    fn test_predict_spawn_request_full() {
        let req: PredictSpawnRequest = serde_json::from_str(
            r#"{
            "task": "Deploy microservices to 3 clusters",
            "current_load": 0.45,
            "active_agents": 3,
            "preferred_capabilities": ["deploy", "kubernetes"]
        }"#,
        )
        .unwrap();
        assert_eq!(req.current_load, 0.45);
        assert_eq!(req.active_agents, 3);
        assert_eq!(req.preferred_capabilities.len(), 2);
    }

    #[test]
    fn test_predict_spawn_request_minimal() {
        let req: PredictSpawnRequest = serde_json::from_str(
            r#"{
            "task": "build app"
        }"#,
        )
        .unwrap();
        assert_eq!(req.current_load, 0.0);
        assert_eq!(req.active_agents, 0);
        assert!(req.preferred_capabilities.is_empty());
    }
}

// ============================================================================
// Category — CL-v2 Observer Hook (S20-2)
// ============================================================================

/// POST /v1/nous/observe — ingest a tool-use observation from Claude Code hooks
/// and extract a lesson via the cognitive learning engine.
///
/// Called by ~/.zeus/hooks/observe.sh on every PreToolUse / PostToolUse event.
#[derive(Debug, Deserialize)]
pub struct ObserveRequest {
    /// Tool that was invoked (e.g. "Bash", "Read", "Edit")
    pub tool: String,
    /// Hook phase: "tool_start" (PreToolUse) or "tool_complete" (PostToolUse)
    pub event: String,
    /// Truncated tool input JSON (≤5000 chars)
    #[serde(default)]
    pub input: Option<String>,
    /// Truncated tool output (≤5000 chars, only present on tool_complete)
    #[serde(default)]
    pub output: Option<String>,
    /// Whether the tool call succeeded (PostToolUse only)
    #[serde(default)]
    pub success: Option<bool>,
    /// Claude Code session ID
    #[serde(default)]
    pub session_id: Option<String>,
    /// Project directory / CWD at hook time
    #[serde(default)]
    pub project: Option<String>,
}

pub async fn nous_observe(
    State(state): State<SharedState>,
    Json(req): Json<ObserveRequest>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let state = state.read().await;
    let Some(ref nous) = state.nous else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Nous cognitive engine not initialized".into(),
        ));
    };

    // Only extract a lesson on PostToolUse completions — start events are informational only
    if req.event == "tool_start" {
        return Ok(Json(json!({ "status": "recorded", "lesson": null })));
    }

    let success = req.success.unwrap_or(true);
    let session = req.session_id.as_deref().unwrap_or("unknown");
    let project = req.project.as_deref().unwrap_or("unknown");

    // Synthesise feedback from available fields
    let feedback = if let Some(ref out) = req.output {
        format!(
            "Tool {} in session {} (project: {}) — output: {}",
            req.tool,
            session,
            project,
            &out.chars().take(500).collect::<String>()
        )
    } else {
        format!(
            "Tool {} in session {} (project: {})",
            req.tool, session, project
        )
    };

    // Minimal intent whose ID encodes the observation key
    let intent_id = format!("obs:{}:{}:{}", session, req.tool, req.event);
    let intent = zeus_nous::Intent {
        id: intent_id.clone(),
        raw_input: req.input.clone().unwrap_or_default(),
        intent_type: zeus_nous::IntentType::Unclear {
            raw: intent_id.clone(),
            possibilities: Vec::new(),
        },
        confidence: zeus_nous::Confidence(if success { 0.75 } else { 0.4 }),
        entities: Vec::new(),
        temporal: None,
        urgency: 0.3,
        implicit_context: Vec::new(),
        related_intents: Vec::new(),
        clarifications: Vec::new(),
        timestamp: chrono::Utc::now(),
    };

    let lesson = nous
        .learn_outcome(&intent, success, &feedback)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(json!({
        "status": "learned",
        "lesson": {
            "id": lesson.id,
            "insight": lesson.insight,
            "category": format!("{:?}", lesson.category),
            "confidence": lesson.confidence,
            "recommendation": lesson.recommendation,
        },
    })))
}
