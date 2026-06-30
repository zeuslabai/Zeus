//! Integration tests for the documentation site endpoints

use axum::http::StatusCode;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower::ServiceExt;
use zeus_api::{AppState, create_test_router};
use zeus_core::Config;

fn create_test_state() -> Arc<RwLock<AppState>> {
    let config = Config::default();
    Arc::new(RwLock::new(AppState::new(config).unwrap()))
}

#[tokio::test]
async fn test_docs_index() {
    let state = create_test_state();
    let router = create_test_router(state);
    let req = axum::http::Request::builder()
        .uri("/docs")
        .body(axum::body::Body::empty())
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("Zeus Documentation"));
    assert!(html.contains("/docs/tools"));
    assert!(html.contains("/docs/config"));
}

#[tokio::test]
async fn test_docs_openapi_json() {
    let state = create_test_state();
    let router = create_test_router(state);
    let req = axum::http::Request::builder()
        .uri("/docs/openapi.json")
        .body(axum::body::Body::empty())
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let spec: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(spec["openapi"], "3.0.3");
    assert_eq!(spec["info"]["title"], "Zeus API");
    assert!(spec["paths"].as_object().unwrap().len() > 50);
    assert!(spec["components"]["securitySchemes"]["bearerAuth"].is_object());
}

#[tokio::test]
async fn test_docs_tools() {
    let state = create_test_state();
    let router = create_test_router(state);
    let req = axum::http::Request::builder()
        .uri("/docs/tools")
        .body(axum::body::Body::empty())
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("Tool Reference"));
    assert!(html.contains("read_file"));
    assert!(html.contains("write_file"));
    assert!(html.contains("shell"));
}

#[tokio::test]
async fn test_docs_config() {
    let state = create_test_state();
    let router = create_test_router(state);
    let req = axum::http::Request::builder()
        .uri("/docs/config")
        .body(axum::body::Body::empty())
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("Configuration Guide"));
    assert!(html.contains("[mnemosyne]"));
    assert!(html.contains("[aegis]"));
    assert!(html.contains("ANTHROPIC_API_KEY"));
}

#[tokio::test]
async fn test_docs_getting_started() {
    let state = create_test_state();
    let router = create_test_router(state);
    let req = axum::http::Request::builder()
        .uri("/docs/getting-started")
        .body(axum::body::Body::empty())
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("Getting Started"));
    assert!(html.contains("zeus onboard"));
    assert!(html.contains("zeus gateway"));
}

#[tokio::test]
async fn test_openapi_has_tool_components() {
    let state = create_test_state();
    let router = create_test_router(state);
    let req = axum::http::Request::builder()
        .uri("/docs/openapi.json")
        .body(axum::body::Body::empty())
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let spec: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let schemas = spec["components"]["schemas"].as_object().unwrap();
    assert!(schemas.contains_key("Tool_read_file"));
    assert!(schemas.contains_key("Tool_shell"));
    assert!(schemas.contains_key("Tool_message"));
}
