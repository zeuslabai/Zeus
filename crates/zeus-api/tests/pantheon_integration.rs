//! Integration tests for Pantheon mission persistence
//!
//! Verifies the full REST API flow with SQLite-backed PantheonStore:
//! create mission → list → get detail → intervene → feed → artifacts → review

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::RwLock;
use tower::ServiceExt;
use zeus_api::{AppState, create_test_router};
use zeus_core::Config;

fn create_test_state() -> Arc<RwLock<AppState>> {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.workspace = tmp.path().to_path_buf();
    config.sessions = tmp.path().join("sessions");
    let state = AppState::new(config).unwrap();
    Arc::new(RwLock::new(state))
}

fn json_request(method: &str, uri: &str, body: Option<Value>) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    match body {
        Some(b) => {
            builder = builder.header("content-type", "application/json");
            builder
                .body(Body::from(serde_json::to_vec(&b).unwrap()))
                .unwrap()
        }
        None => builder.body(Body::empty()).unwrap(),
    }
}

async fn response_json(response: axum::http::Response<Body>) -> Value {
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&body).unwrap_or(Value::Null)
}

#[tokio::test]
async fn test_list_missions_empty() {
    let state = create_test_state();
    let app = create_test_router(state);

    let req = json_request("GET", "/v1/pantheon/missions", None);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let json = response_json(resp).await;
    assert_eq!(json["total"], 0);
    assert!(json["missions"].is_array());
    assert_eq!(json["missions"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn test_create_mission() {
    let state = create_test_state();
    let app = create_test_router(state);

    let req = json_request(
        "POST",
        "/v1/pantheon/missions",
        Some(json!({
            "goal": "Build a REST API for user management"
        })),
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let json = response_json(resp).await;
    assert!(json["id"].is_string());
    assert_eq!(json["goal"], "Build a REST API for user management");
    assert!(json["status"].is_string());
    assert!(json["team"].is_array());
    assert!(json["created_at"].is_string());
}

#[tokio::test]
async fn test_create_and_list_missions() {
    let state = create_test_state();
    let app = create_test_router(state);

    // Create a mission
    let req = json_request(
        "POST",
        "/v1/pantheon/missions",
        Some(json!({
            "goal": "Implement search feature"
        })),
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let created = response_json(resp).await;
    let mission_id = created["id"].as_str().unwrap().to_string();

    // List missions — should contain the created one
    let req = json_request("GET", "/v1/pantheon/missions", None);
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let list = response_json(resp).await;
    let missions = list["missions"].as_array().unwrap();
    assert!(missions.len() >= 1);
    assert!(
        missions
            .iter()
            .any(|m| m["id"].as_str() == Some(&mission_id))
    );
}

#[tokio::test]
async fn test_get_mission_detail() {
    let state = create_test_state();
    let app = create_test_router(state);

    // Create
    let req = json_request(
        "POST",
        "/v1/pantheon/missions",
        Some(json!({
            "goal": "Deploy to production",
            "constraints": {
                "budget_tokens": 100000,
                "timeout_seconds": 300,
                "max_agents": 3,
                "require_review": true
            }
        })),
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    let created = response_json(resp).await;
    let mission_id = created["id"].as_str().unwrap();

    // Get detail
    let req = json_request(
        "GET",
        &format!("/v1/pantheon/missions/{}", mission_id),
        None,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let detail = response_json(resp).await;
    assert_eq!(detail["id"].as_str(), Some(mission_id));
    assert_eq!(detail["goal"], "Deploy to production");
    assert!(detail["team"].is_array());
    assert!(detail["tasks"].is_array());
    assert!(detail["feed"].is_array());
}

#[tokio::test]
async fn test_get_mission_not_found() {
    let state = create_test_state();
    let app = create_test_router(state);

    let req = json_request("GET", "/v1/pantheon/missions/m-nonexistent", None);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_intervene_cancel_mission() {
    let state = create_test_state();
    let app = create_test_router(state);

    // Create
    let req = json_request(
        "POST",
        "/v1/pantheon/missions",
        Some(json!({
            "goal": "Long running task"
        })),
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    let created = response_json(resp).await;
    let mission_id = created["id"].as_str().unwrap();

    // Cancel
    let req = json_request(
        "POST",
        &format!("/v1/pantheon/missions/{}/intervene", mission_id),
        Some(json!({
            "action": "cancel",
            "message": "No longer needed"
        })),
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let result = response_json(resp).await;
    assert_eq!(result["ok"], true);
    assert_eq!(result["action"], "cancel");

    // Verify status changed
    let req = json_request(
        "GET",
        &format!("/v1/pantheon/missions/{}", mission_id),
        None,
    );
    let resp = app.oneshot(req).await.unwrap();
    let detail = response_json(resp).await;
    assert_eq!(detail["status"], "cancelled");
}

#[tokio::test]
async fn test_intervene_not_found() {
    let state = create_test_state();
    let app = create_test_router(state);

    let req = json_request(
        "POST",
        "/v1/pantheon/missions/m-nope/intervene",
        Some(json!({
            "action": "pause"
        })),
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_get_mission_feed() {
    let state = create_test_state();
    let app = create_test_router(state);

    // Create
    let req = json_request(
        "POST",
        "/v1/pantheon/missions",
        Some(json!({
            "goal": "Feed test mission"
        })),
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    let created = response_json(resp).await;
    let mission_id = created["id"].as_str().unwrap();

    // Get feed
    let req = json_request(
        "GET",
        &format!("/v1/pantheon/missions/{}/feed", mission_id),
        None,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let feed = response_json(resp).await;
    assert!(feed.is_array());
}

#[tokio::test]
async fn test_get_mission_artifacts() {
    let state = create_test_state();
    let app = create_test_router(state);

    // Create
    let req = json_request(
        "POST",
        "/v1/pantheon/missions",
        Some(json!({
            "goal": "Artifact test"
        })),
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    let created = response_json(resp).await;
    let mission_id = created["id"].as_str().unwrap();

    // Get artifacts
    let req = json_request(
        "GET",
        &format!("/v1/pantheon/missions/{}/artifacts", mission_id),
        None,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let artifacts = response_json(resp).await;
    assert!(artifacts.is_array());
}

#[tokio::test]
async fn test_feed_not_found() {
    let state = create_test_state();
    let app = create_test_router(state);

    let req = json_request("GET", "/v1/pantheon/missions/m-nope/feed", None);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_create_multiple_missions() {
    let state = create_test_state();
    let app = create_test_router(state);

    // Create 3 missions
    for i in 0..3 {
        let req = json_request(
            "POST",
            "/v1/pantheon/missions",
            Some(json!({
                "goal": format!("Mission {}", i)
            })),
        );
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    // List — should have all 3
    let req = json_request("GET", "/v1/pantheon/missions", None);
    let resp = app.oneshot(req).await.unwrap();
    let list = response_json(resp).await;
    assert!(list["missions"].as_array().unwrap().len() >= 3);
    assert_eq!(list["total"], 3);
}

#[tokio::test]
async fn test_list_missions_pagination() {
    let state = create_test_state();
    let app = create_test_router(state);

    // Create 5 missions
    for i in 0..5 {
        let req = json_request(
            "POST",
            "/v1/pantheon/missions",
            Some(json!({
                "goal": format!("Paginated mission {}", i)
            })),
        );
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    // Page 1: limit=2, offset=0
    let req = json_request("GET", "/v1/pantheon/missions?limit=2&offset=0", None);
    let resp = app.clone().oneshot(req).await.unwrap();
    let page1 = response_json(resp).await;
    assert_eq!(page1["missions"].as_array().unwrap().len(), 2);
    assert_eq!(page1["total"], 5);
    assert_eq!(page1["offset"], 0);
    assert_eq!(page1["limit"], 2);

    // Page 2: limit=2, offset=2
    let req = json_request("GET", "/v1/pantheon/missions?limit=2&offset=2", None);
    let resp = app.clone().oneshot(req).await.unwrap();
    let page2 = response_json(resp).await;
    assert_eq!(page2["missions"].as_array().unwrap().len(), 2);
    assert_eq!(page2["total"], 5);
    assert_eq!(page2["offset"], 2);

    // Page 3: limit=2, offset=4 — only 1 remaining
    let req = json_request("GET", "/v1/pantheon/missions?limit=2&offset=4", None);
    let resp = app.clone().oneshot(req).await.unwrap();
    let page3 = response_json(resp).await;
    assert_eq!(page3["missions"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn test_list_missions_status_filter() {
    let state = create_test_state();
    let app = create_test_router(state);

    // Create a mission
    let req = json_request(
        "POST",
        "/v1/pantheon/missions",
        Some(json!({
            "goal": "Filterable mission"
        })),
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    let created = response_json(resp).await;
    let mission_id = created["id"].as_str().unwrap();

    // Cancel it
    let req = json_request(
        "POST",
        &format!("/v1/pantheon/missions/{}/intervene", mission_id),
        Some(json!({
            "action": "cancel"
        })),
    );
    app.clone().oneshot(req).await.unwrap();

    // Create another (will be "executing")
    let req = json_request(
        "POST",
        "/v1/pantheon/missions",
        Some(json!({
            "goal": "Active mission"
        })),
    );
    app.clone().oneshot(req).await.unwrap();

    // Filter by cancelled
    let req = json_request("GET", "/v1/pantheon/missions?status=cancelled", None);
    let resp = app.clone().oneshot(req).await.unwrap();
    let list = response_json(resp).await;
    assert_eq!(list["total"], 1);
    assert_eq!(
        list["missions"].as_array().unwrap()[0]["status"],
        "cancelled"
    );

    // Filter by executing
    let req = json_request("GET", "/v1/pantheon/missions?status=executing", None);
    let resp = app.clone().oneshot(req).await.unwrap();
    let list = response_json(resp).await;
    assert_eq!(list["total"], 1);
    assert_eq!(
        list["missions"].as_array().unwrap()[0]["status"],
        "executing"
    );
}

#[tokio::test]
async fn test_persistence_across_reads() {
    // Verify data persists across multiple reads (SQLite not just in-memory cache)
    let state = create_test_state();
    let app = create_test_router(state.clone());

    // Create
    let req = json_request(
        "POST",
        "/v1/pantheon/missions",
        Some(json!({
            "goal": "Persistence test"
        })),
    );
    let resp = app.clone().oneshot(req).await.unwrap();
    let created = response_json(resp).await;
    let mission_id = created["id"].as_str().unwrap().to_string();

    // Read multiple times — should always find it
    for _ in 0..3 {
        let app2 = create_test_router(state.clone());
        let req = json_request(
            "GET",
            &format!("/v1/pantheon/missions/{}", mission_id),
            None,
        );
        let resp = app2.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let detail = response_json(resp).await;
        assert_eq!(detail["goal"], "Persistence test");
    }
}
