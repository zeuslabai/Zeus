//! Integration tests for file upload endpoints

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower::ServiceExt;
use zeus_api::{AppState, create_test_router};
use zeus_core::Config;

/// Helper to create test app state
fn create_test_state() -> Arc<RwLock<AppState>> {
    let config = Config::default();
    let state = AppState::new(config).unwrap();
    Arc::new(RwLock::new(state))
}

/// Helper to create multipart body
fn create_multipart_body(filename: &str, content: &[u8], boundary: &str) -> Vec<u8> {
    let mut body = Vec::new();

    // Start boundary
    body.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());

    // Content-Disposition header
    body.extend_from_slice(
        format!(
            "Content-Disposition: form-data; name=\"file\"; filename=\"{}\"\r\n",
            filename
        )
        .as_bytes(),
    );

    // Content-Type header
    body.extend_from_slice(b"Content-Type: application/octet-stream\r\n");

    // Empty line before content
    body.extend_from_slice(b"\r\n");

    // File content
    body.extend_from_slice(content);

    // End boundary
    body.extend_from_slice(format!("\r\n--{}--\r\n", boundary).as_bytes());

    body
}

#[tokio::test]
async fn test_upload_file() {
    let state = create_test_state();
    let app = create_test_router(state.clone());

    let boundary = "----WebKitFormBoundary7MA4YWxkTrZu0gW";
    let file_content = b"Hello, Zeus!";
    let multipart_body = create_multipart_body("test.txt", file_content, boundary);

    let request = Request::builder()
        .method("POST")
        .uri("/v1/uploads")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={}", boundary),
        )
        .body(Body::from(multipart_body))
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["name"], "test.txt");
    assert_eq!(json["size"], file_content.len());
    assert!(json["id"].is_string());
    assert!(json["url"].is_string());
}

#[tokio::test]
async fn test_list_uploads() {
    let state = create_test_state();
    let app = create_test_router(state.clone());

    let request = Request::builder()
        .method("GET")
        .uri("/v1/uploads")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();

    assert!(json.is_array());
}

#[tokio::test]
async fn test_upload_and_retrieve() {
    let state = create_test_state();
    let app = create_test_router(state.clone());

    // Upload a file
    let boundary = "----WebKitFormBoundary7MA4YWxkTrZu0gW";
    let file_content = b"Test content for retrieval";
    let multipart_body = create_multipart_body("retrieve.txt", file_content, boundary);

    let upload_request = Request::builder()
        .method("POST")
        .uri("/v1/uploads")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={}", boundary),
        )
        .body(Body::from(multipart_body))
        .unwrap();

    let upload_response = app.clone().oneshot(upload_request).await.unwrap();
    let upload_body = axum::body::to_bytes(upload_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let upload_json: Value = serde_json::from_slice(&upload_body).unwrap();
    let file_id = upload_json["id"].as_str().unwrap();

    // Retrieve metadata
    let get_request = Request::builder()
        .method("GET")
        .uri(format!("/v1/uploads/{}", file_id))
        .body(Body::empty())
        .unwrap();

    let get_response = app.oneshot(get_request).await.unwrap();
    assert_eq!(get_response.status(), StatusCode::OK);

    let get_body = axum::body::to_bytes(get_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let get_json: Value = serde_json::from_slice(&get_body).unwrap();

    assert_eq!(get_json["id"], file_id);
    assert_eq!(get_json["name"], "retrieve.txt");
}

#[tokio::test]
async fn test_delete_upload() {
    let state = create_test_state();
    let app = create_test_router(state.clone());

    // Upload a file
    let boundary = "----WebKitFormBoundary7MA4YWxkTrZu0gW";
    let file_content = b"Delete this";
    let multipart_body = create_multipart_body("delete.txt", file_content, boundary);

    let upload_request = Request::builder()
        .method("POST")
        .uri("/v1/uploads")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={}", boundary),
        )
        .body(Body::from(multipart_body))
        .unwrap();

    let upload_response = app.clone().oneshot(upload_request).await.unwrap();
    let upload_body = axum::body::to_bytes(upload_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let upload_json: Value = serde_json::from_slice(&upload_body).unwrap();
    let file_id = upload_json["id"].as_str().unwrap();

    // Delete the file
    let delete_request = Request::builder()
        .method("DELETE")
        .uri(format!("/v1/uploads/{}", file_id))
        .body(Body::empty())
        .unwrap();

    let delete_response = app.clone().oneshot(delete_request).await.unwrap();
    assert_eq!(delete_response.status(), StatusCode::NO_CONTENT);

    // Try to get the deleted file (should fail)
    let get_request = Request::builder()
        .method("GET")
        .uri(format!("/v1/uploads/{}", file_id))
        .body(Body::empty())
        .unwrap();

    let get_response = app.oneshot(get_request).await.unwrap();
    assert_eq!(get_response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_upload_not_found() {
    let state = create_test_state();
    let app = create_test_router(state.clone());

    let request = Request::builder()
        .method("GET")
        .uri("/v1/uploads/nonexistent-id")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
