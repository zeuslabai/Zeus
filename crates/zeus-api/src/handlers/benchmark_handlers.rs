//! Benchmark API handlers — wires BenchmarkStore into REST endpoints.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::{Value, json};
use zeus_prometheus::BenchmarkStore;

use crate::SharedState;

/// Query params for the compare endpoint.
#[derive(Debug, Deserialize)]
pub struct CompareParams {
    pub baseline: String,
    pub candidate: String,
}

/// GET /v1/benchmarks — list all benchmark runs (most recent first).
pub async fn list_benchmark_runs(
    State(_state): State<SharedState>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let store = BenchmarkStore::open_default().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to open benchmark store: {e}"),
        )
    })?;

    let run_ids = store.list_runs().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to list runs: {e}"),
        )
    })?;

    // Summarize each run
    let mut runs = Vec::new();
    for run_id in &run_ids {
        if let Ok(summary) = store.summarize_run(run_id) {
            runs.push(serde_json::to_value(&summary).unwrap_or_default());
        }
    }

    Ok(Json(json!({
        "runs": runs,
        "total": runs.len(),
    })))
}

/// GET /v1/benchmarks/:run_id — get results for a specific run.
pub async fn get_benchmark_run(
    State(_state): State<SharedState>,
    Path(run_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let store = BenchmarkStore::open_default().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to open benchmark store: {e}"),
        )
    })?;

    let results = store.get_results(&run_id).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to get results: {e}"),
        )
    })?;

    if results.is_empty() {
        return Err((
            StatusCode::NOT_FOUND,
            format!("No results for run '{run_id}'"),
        ));
    }

    let summary = store.summarize_run(&run_id).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to summarize run: {e}"),
        )
    })?;

    Ok(Json(json!({
        "run_id": run_id,
        "summary": serde_json::to_value(&summary).unwrap_or_default(),
        "results": results.iter().map(|r| serde_json::to_value(r).unwrap_or_default()).collect::<Vec<_>>(),
    })))
}

/// GET /v1/benchmarks/compare?baseline=X&candidate=Y — compare two runs.
pub async fn compare_benchmark_runs(
    State(_state): State<SharedState>,
    Query(params): Query<CompareParams>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let store = BenchmarkStore::open_default().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to open benchmark store: {e}"),
        )
    })?;

    let comparison = store
        .compare_runs(&params.baseline, &params.candidate)
        .map_err(|e| {
            (
                StatusCode::NOT_FOUND,
                format!("Comparison failed: {e}"),
            )
        })?;

    Ok(Json(
        serde_json::to_value(&comparison).unwrap_or_default(),
    ))
}
